use thiserror::Error;

#[cfg(feature = "verification")]
#[derive(Debug, Error)]
pub enum ChecksumError {
    #[error("Could not recognize the length of inputted checksum")]
    UnrecognizedSize,
    #[error("Unrecognized checksum type")]
    UnrecognizedType,
    #[error("Input file does not match the given checksum")]
    VerificationFailure,
}

#[cfg(feature = "unarchive")]
#[derive(Debug, Error)]
pub enum ArchiveError {
    #[error("Failed to unarchive file")]
    UnarchiveError,
    #[error("Failed to handle archive files: {0}")]
    FileError(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum DownloadError {
    #[error("Unable to parse URL")]
    URLParse,
    #[error("Unable to determine length of content")]
    ContentLength,
    #[error("{0}")]
    RequestError(#[from] reqwest_middleware::Error),
    #[error("{0}")]
    ReqwestError(#[from] reqwest::Error),
    #[error("{0}")]
    FileError(#[from] tokio::io::Error),
    #[error("Invalid amount of threads requested")]
    InvalidThreads,
    #[error("Invalid checksum")]
    InvalidChecksum,
    #[error("Unable to save to file")]
    SaveError,
    #[cfg(feature = "unarchive")]
    #[error("File names are unsupported for tarballs or zip archives")]
    UnsupportedFileName,
    #[cfg(feature = "unarchive")]
    #[error("{0}")]
    ArchiveError(#[from] ArchiveError),
}
