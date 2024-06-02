mod threads;
#[cfg(feature = "verification")]
pub(crate) mod verify;

use crate::error::DownloadError;
use futures::{
    future,
    stream::{self, StreamExt},
};
#[cfg(feature = "render_progress")]
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use reqwest::{header::HeaderMap, Url};
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::fs::File;

const DEFAULT_RETRIES: u32 = 3;
const DEFAULT_SIMULTANEOUS_DOWNLOADS: usize = 3;
const DEFAULT_TOTAL_PROGRESS: &str = "{elapsed_precise} {bar:30.cyan} {human_pos:>} / {human_len} ({percent}%)";
const DEFAULT_INDIVIDUAL_PROGRESS: &str = "{bar:30.blue/red} ({percent}%) {bytes:>12.green} / {total_bytes:<12.green} {bytes_per_sec:>13.blue} - ETA: {eta_precise}";
const PROGRESS_LINE: &str = "━╾╴─";

static CURRENT_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::current_dir().unwrap());

pub struct Downloader {
    downloads: Vec<Download>,
    client: Option<ClientWithMiddleware>,
    #[cfg(feature = "render_progress")]
    progress: Option<Progress>,
    simultaneous: usize,
    retries: u32,
}

impl Downloader {
    pub fn new(downloads: Vec<Download>) -> Self {
        Self {
            downloads,
            client: None,
            #[cfg(feature = "render_progress")]
            progress: None,
            simultaneous: DEFAULT_SIMULTANEOUS_DOWNLOADS,
            retries: DEFAULT_RETRIES,
        }
    }
    pub fn new_empty() -> Self {
        Self::new(Vec::new())
    }
    pub fn with_progress(mut self, progress: Progress) -> Self {
        self.progress = Some(progress);
        self
    }
    pub fn with_download(mut self, download: Download) -> Self {
        self.downloads.push(download);
        self
    }
    pub fn with_simultaneous_downloads(mut self, simultaneous: usize) -> Self {
        self.simultaneous = simultaneous;
        self
    }
    pub fn with_retries(mut self, retries: u32) -> Self {
        self.retries = retries;
        self
    }
    pub async fn start_downloads(mut self) -> Result<(), DownloadError> {
        let retries = ExponentialBackoff::builder().build_with_max_retries(self.retries);
        let client = reqwest::ClientBuilder::new().connect_timeout(Duration::from_secs(6)).build()?;
        let client = ClientBuilder::new(client)
            .with(RetryTransientMiddleware::new_with_policy(retries))
            .build();
        self.client = Some(client);
        self.fill_download_files().await?;
        self.fill_lengths().await?;
        self.finalize_threads();
        #[cfg(feature = "render_progress")]
        let progress = self.initialize_progress();
        let main = progress.and_then(|progress| progress.1);

        let downloads = self.downloads.into_iter().map(|download| {
            download.spawn(
                self.client.as_ref().unwrap(),
                #[cfg(feature = "render_progress")]
                main.clone(),
            )
        });
        stream::iter(downloads)
            .buffer_unordered(self.simultaneous)
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, DownloadError>>()?;

        #[cfg(feature = "render_progress")]
        if let Some(main_bar) = main {
            main_bar.finish();
        }
        Ok(())
    }
    async fn fill_download_files(&mut self) -> Result<(), DownloadError> {
        let futures = self.downloads.iter_mut().map(|download| download.fill_output());
        future::join_all(futures)
            .await
            .into_iter()
            .collect::<Result<Vec<_>, DownloadError>>()?;
        Ok(())
    }
    #[cfg(feature = "render_progress")]
    fn initialize_progress(&mut self) -> Option<(MultiProgress, Option<ProgressBar>)> {
        let progress = self.progress.as_ref()?;
        if !progress.is_enabled() {
            return None;
        }
        let multi = MultiProgress::new();
        let main_bar = match (&progress.total, self.downloads.len()) {
            (Some(style), 2..) => {
                let progress = ProgressBar::new(self.downloads.len() as u64).with_style(style.clone());
                progress.enable_steady_tick(std::time::Duration::from_millis(100));
                Some(multi.add(progress))
            }
            _ => None,
        };
        if let Some(individual_style) = &progress.individual {
            self.downloads.iter_mut().for_each(|download| {
                let progress = ProgressBar::new(download.content_length.unwrap()).with_style(individual_style.clone());
                progress.enable_steady_tick(std::time::Duration::from_millis(100));
                download.progress = Some(multi.add(progress));
            });
        }
        Some((multi, main_bar))
    }
    fn finalize_threads(&mut self) {
        self.downloads.iter_mut().for_each(|download| {
            if download.preferred_threads.is_none() {
                download.preferred_threads = choose_threads(download.content_length);
            }
        });
    }
    async fn fill_lengths(&mut self) -> Result<(), DownloadError> {
        let client = self.client.as_ref().unwrap();
        let futures = self
            .downloads
            .iter()
            .map(|download| async {
                let mut request = client.get((*download.url).clone());
                if let Some(headers) = &download.headers {
                    request = request.headers((**headers).clone());
                }
                let response = request.send().await.map_err(DownloadError::RequestError)?;
                response.content_length().ok_or(DownloadError::ContentLength)
            })
            .collect::<Vec<_>>();
        let futures = future::join_all(futures).await;
        self.downloads
            .iter_mut()
            .zip(futures)
            .map(|(download, length)| {
                download.content_length = Some(length?);
                Ok(())
            })
            .collect::<Result<Vec<_>, DownloadError>>()?;
        Ok(())
    }
}

fn choose_threads(length: Option<u64>) -> Option<u8> {
    length.map(|length| match length {
        2_000_000_000.. => 5,
        1_000_000_000.. => 4,
        250_000_000.. => 3,
        100_000_000.. => 2,
        _ => 1,
    })
}
pub struct Download {
    url: Arc<Url>,
    output: Option<File>,
    directory: Option<PathBuf>,
    filename: Option<String>,
    headers: Option<Arc<HeaderMap>>,
    #[cfg(feature = "verification")]
    checksum: Option<verify::Checksum>,
    preferred_threads: Option<u8>,
    content_length: Option<u64>,
    progress: Option<ProgressBar>,
}

impl Download {
    pub fn new(url: impl AsRef<str>) -> Result<Self, DownloadError> {
        let url = Url::parse(url.as_ref()).map_err(|_| DownloadError::URLParse)?;
        Ok(Self::new_from_url(url))
    }
    pub fn new_from_url(url: impl Into<Arc<Url>>) -> Self {
        Self {
            url: url.into(),
            output: None,
            directory: None,
            filename: None,
            headers: None,
            checksum: None,
            preferred_threads: None,
            content_length: None,
            progress: None,
        }
    }
    pub fn with_filename(mut self, filename: String) -> Self {
        self.filename = Some(filename);
        self
    }
    pub fn with_output_dir(mut self, path: PathBuf) -> Self {
        self.directory = Some(path);
        self
    }
    pub fn with_output_file(mut self, file: impl Into<File>) -> Self {
        self.output = Some(file.into());
        self
    }
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = Some(headers.into());
        self
    }
    #[cfg(feature = "verification")]
    pub fn with_checksum(mut self, checksum: verify::Checksum) -> Self {
        self.checksum = Some(checksum);
        self
    }
    pub fn with_threads(mut self, threads: u8) -> Self {
        self.preferred_threads = Some(threads);
        self
    }
    async fn fill_output(&mut self) -> Result<(), DownloadError> {
        if self.output.is_none() {
            let filename = self.filename.as_deref().unwrap_or_else(|| {
                self.url
                    .path_segments()
                    .and_then(|segments| segments.last())
                    .and_then(|name| if name.is_empty() { None } else { Some(name) })
                    .unwrap_or("download")
            });
            let dir = self.directory.as_ref().unwrap_or(&*CURRENT_DIR);
            let file = File::create_new(dir.join(filename)).await.map_err(DownloadError::FileError)?;
            self.output = Some(file);
        }
        Ok(())
    }
    async fn spawn(self, client: &ClientWithMiddleware, #[cfg(feature = "render_progress")] main_bar: Option<ProgressBar>) -> Result<(), DownloadError> {
        let mut chunks = threads::Chunks::new(self.preferred_threads.unwrap(), self.content_length.unwrap());
        chunks
            .download(
                client,
                self.url,
                self.headers,
                #[cfg(feature = "render_progress")]
                self.progress,
            )
            .await?;
        #[cfg(feature = "verification")]
        if let Some(checksum) = self.checksum {
            chunks.verify(checksum)?;
        }
        chunks.save(self.output.unwrap()).await?;
        #[cfg(feature = "render_progress")]
        if let Some(main_bar) = main_bar {
            main_bar.inc(1);
        }
        Ok(())
    }
}

#[cfg(feature = "render_progress")]
pub struct Progress {
    total: Option<ProgressStyle>,
    individual: Option<ProgressStyle>,
}
impl Default for Progress {
    fn default() -> Self {
        Self::new().with_default_total().with_default_individual()
    }
}
#[cfg(feature = "render_progress")]
impl Progress {
    pub fn new() -> Self {
        Self { total: None, individual: None }
    }
    pub fn with_default_total(mut self) -> Self {
        self.total = Some(ProgressStyle::with_template(DEFAULT_TOTAL_PROGRESS).unwrap());
        self
    }
    pub fn with_default_individual(mut self) -> Self {
        self.individual = Some(
            ProgressStyle::with_template(DEFAULT_INDIVIDUAL_PROGRESS)
                .unwrap()
                .progress_chars(PROGRESS_LINE),
        );
        self
    }
    pub fn with_total(mut self, style: ProgressStyle) -> Self {
        self.total = Some(style);
        self
    }
    pub fn with_individual(mut self, style: ProgressStyle) -> Self {
        self.individual = Some(style);
        self
    }
    fn is_enabled(&self) -> bool {
        self.total.is_some() || self.individual.is_some()
    }
}
