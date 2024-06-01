#[cfg(feature = "verification")]
use crate::downloader::verify::Checksum;

use crate::error::DownloadError;
use reqwest::{
    header::{HeaderMap, RANGE},
    Url,
};
use reqwest_middleware::ClientWithMiddleware;
use std::{cmp::min, io::SeekFrom, sync::Arc};
use tokio::{
    fs::File,
    io::{AsyncSeekExt, AsyncWriteExt},
    spawn,
};

pub struct Chunks {
    chunks: Vec<Chunk>,
}

impl Chunks {
    pub(crate) fn new(threads: u8, length: u64) -> Self {
        let size = length / threads as u64;
        let chunks = (0..threads)
            .map(|t| {
                let begin = size * t as u64;
                let end = min(begin + size, length);
                log::info!("Chunk: {}-{}, t: {t}, length: {length}", begin, end);
                Chunk { buf: Vec::new(), begin, end }
            })
            .collect::<Vec<Chunk>>();
        Self { chunks }
    }
    pub(crate) async fn download(
        &mut self,
        client: &ClientWithMiddleware,
        url: Arc<Url>,
        headers: Option<Arc<HeaderMap>>,
        #[cfg(feature = "render_progress")] progress: Option<indicatif::ProgressBar>,
    ) -> Result<(), DownloadError> {
        let futures = self.chunks.iter_mut().map(|chunk| {
            let headers = headers.clone();
            chunk.download(
                client,
                (*url).clone(),
                headers,
                #[cfg(feature = "render_progress")]
                progress.clone(),
            )
        });
        futures::future::join_all(futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, DownloadError>>()?;
        #[cfg(feature = "render_progress")]
        if let Some(progress) = progress {
            progress.finish();
        }
        Ok(())
    }
    pub(crate) async fn save(self, output: File) -> Result<(), DownloadError> {
        let mut futures = Vec::new();
        for chunk in self.chunks {
            let output = output.try_clone().await.map_err(DownloadError::FileError)?;
            futures.push(spawn(chunk.save(output)));
        }
        futures::future::join_all(futures)
            .await
            .into_iter()
            .map(|result| result.map_err(|_| DownloadError::SaveError))
            .collect::<Result<Vec<_>, DownloadError>>()?;
        Ok(())
    }
    #[cfg(feature = "verification")]
    pub(crate) fn verify(&self, mut checksum: Checksum) -> Result<(), DownloadError> {
        self.chunks.iter().for_each(|chunk| checksum.update(chunk.buf.as_slice()));
        if checksum.verify() {
            Ok(())
        } else {
            Err(DownloadError::InvalidChecksum)
        }
    }
}

pub struct Chunk {
    buf: Vec<u8>,
    begin: u64,
    end: u64,
}

impl Chunk {
    async fn download(
        &mut self,
        client: &ClientWithMiddleware,
        url: Url,
        headers: Option<Arc<HeaderMap>>,
        #[cfg(feature = "render_progress")] progress: Option<indicatif::ProgressBar>,
    ) -> Result<(), DownloadError> {
        let range = format!("bytes={}-{}", self.begin, self.end);
        let mut response = client.get(url).header(RANGE, range);
        if let Some(headers) = headers {
            response = response.headers((*headers).clone());
        }
        let response = response.send().await.map_err(DownloadError::RequestError)?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = futures::StreamExt::next(&mut stream)
            .await
            .transpose()
            .map_err(DownloadError::ReqwestError)?
        {
            self.buf.extend_from_slice(&chunk);
            #[cfg(feature = "render_progress")]
            if let Some(ref progress) = progress {
                progress.inc(chunk.len() as u64);
            }
        }
        Ok(())
    }
    async fn save(self, mut output: File) -> Result<(), DownloadError> {
        output
            .seek(SeekFrom::Start(self.begin))
            .await
            .map_err(DownloadError::FileError)?;
        output.write_all(self.buf.as_slice()).await.map_err(DownloadError::FileError)?;
        Ok(())
    }
}
