mod downloader;
mod error;

pub use downloader::{Download, Downloader};

#[cfg(feature = "verification")]
pub use downloader::verify::{Checksum, CsType};

#[cfg(feature = "render_progress")]
pub use downloader::Progress;
