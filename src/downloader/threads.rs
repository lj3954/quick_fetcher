#[cfg(feature = "verification")]
use crate::downloader::verify::Checksum;

#[cfg(feature = "unarchive")]
use super::decompress::ArchiveFormat;
use crate::error::DownloadError;
use reqwest::{
    header::{HeaderMap, RANGE},
    Url,
};
use reqwest_middleware::ClientWithMiddleware;
use std::{
    cmp::min,
    fs::File,
    io::{Seek, SeekFrom, Write},
    sync::Arc,
};

pub struct Chunks {
    chunks: Vec<Chunk>,
}

impl Chunks {
    pub(crate) fn new(threads: u8, length: u64) -> Self {
        let t = threads as u64;
        let size = (length + t) / t;
        let chunks = (0..threads)
            .map(|t| {
                let begin = size * t as u64;
                let end = min(begin + size, length);
                log::info!("Chunk: {}-{}, t: {t}, length: {length}", begin, end);
                Chunk { buf: Vec::new(), begin, end, length }
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
        self.chunks.sort_by_key(|chunk| chunk.begin);
        Ok(())
    }
    pub(crate) fn save(self, output: File) -> Result<(), DownloadError> {
        for chunk in self.chunks {
            let output = output.try_clone().map_err(DownloadError::FileError)?;
            chunk.save(output)?;
        }
        output.sync_all().map_err(DownloadError::FileError)?;
        Ok(())
    }
    #[cfg(feature = "unarchive")]
    pub(crate) fn save_archive(self, path: Option<std::path::PathBuf>, output: File, archive_format: ArchiveFormat) -> Result<(), crate::error::ArchiveError> {
        let mut data = self
            .chunks
            .iter()
            .map(|chunk| {
                let end = (chunk.end - chunk.begin) as usize;
                &chunk.buf[0..end]
            })
            .collect::<Vec<&[u8]>>();
        archive_format.decompress(output, path, &mut data)
    }
    #[cfg(feature = "verification")]
    pub(crate) fn verify(&self, mut checksum: Checksum) -> Result<(), DownloadError> {
        self.chunks.iter().for_each(|chunk| {
            let range = 0..chunk.end as usize - chunk.begin as usize;
            checksum.update(&chunk.buf[range]);
        });
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
    length: u64,
}

impl Chunk {
    async fn download(
        &mut self,
        client: &ClientWithMiddleware,
        url: Url,
        headers: Option<Arc<HeaderMap>>,
        #[cfg(feature = "render_progress")] progress: Option<indicatif::ProgressBar>,
    ) -> Result<(), DownloadError> {
        let mut response = client.get(url);

        let range = match (self.begin, self.end, self.length) {
            (0, end, length) if end == length => None,
            (_, end, length) if end == length => Some(format!("bytes={}-", self.begin)),
            _ => Some(format!("bytes={}-{}", self.begin, self.end)),
        };
        if let Some(range) = range {
            response = response.header(RANGE, range);
        }
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
    fn save(self, mut output: File) -> Result<(), DownloadError> {
        log::debug!("Buf: {}, intended: {}", self.buf.len(), self.end - self.begin);
        let pos = output.seek(SeekFrom::Start(self.begin)).map_err(DownloadError::FileError)?;
        log::debug!("Seeked to {}, {}", self.begin, pos);
        output.write_all(self.buf.as_slice()).map_err(DownloadError::FileError)?;
        log::debug!("Wrote to {}", self.end);
        Ok(())
    }
}
