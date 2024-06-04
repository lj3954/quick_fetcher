use crate::error::ArchiveError;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
};

pub enum ArchiveFormat {
    Tar,
    TarBz2,
    TarGz,
    TarXz,
    TarZst,
    Zip,
    Xz,
    Gz,
    Bz2,
    Zst,
}

impl<'a> ArchiveFormat {
    pub fn decompress(&self, file: File, path: Option<PathBuf>, data: &'a mut [&'a [u8]]) -> Result<(), ArchiveError> {
        log::debug!("Decompressing archive");
        let path = || path.unwrap_or(std::env::current_dir().unwrap());
        let mut archive_output = if matches!(self, Self::Tar | Self::TarBz2 | Self::TarGz | Self::TarXz | Self::TarZst) {
            Output::Tarball(Vec::new())
        } else {
            Output::File(file)
        };
        let reader = SliceReader::new(data);
        match self {
            Self::Bz2 | Self::TarBz2 => {
                let mut decompressor = bzip2::read::BzDecoder::new(reader);
                std::io::copy(&mut decompressor, &mut archive_output)?;
            }
            Self::Gz | Self::TarGz => {
                let mut decompressor = flate2::read::GzDecoder::new(reader);
                std::io::copy(&mut decompressor, &mut archive_output)?;
            }
            Self::Xz | Self::TarXz => {
                let mut decompressor = liblzma::read::XzDecoder::new(reader);
                std::io::copy(&mut decompressor, &mut archive_output)?;
            }
            Self::Zst | Self::TarZst => {
                let mut decompressor = zstd::stream::Decoder::new(reader)?;
                std::io::copy(&mut decompressor, &mut archive_output)?;
            }
            Self::Zip => {
                let path = path();
                let mut archive = zip::ZipArchive::new(reader).map_err(|_| ArchiveError::UnarchiveError)?;
                for i in 0..archive.len() {
                    let mut file = archive.by_index(i).map_err(|_| ArchiveError::UnarchiveError)?;
                    let mut output = File::create(&path.join(file.name()))?;
                    std::io::copy(&mut file, &mut output)?;
                }
                return Ok(());
            }
            _ => (),
        }
        if let Output::Tarball(data) = archive_output {
            let mut archive = tar::Archive::new(data.as_slice());
            archive.unpack(path())?;
        }
        Ok(())
    }
}

struct SliceReader<'a> {
    slices: &'a mut [&'a [u8]],
    index: usize,
    inner_index: usize,
}

impl<'a> SliceReader<'a> {
    fn new(slices: &'a mut [&'a [u8]]) -> Self {
        Self { slices, index: 0, inner_index: 0 }
    }
}

impl<'a> Read for SliceReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        while self.index < self.slices.len() {
            match self.slices[self.index].read(buf) {
                Ok(0) => self.index += 1,
                r => return r,
            }
        }
        Ok(0)
    }
}

impl<'a> Seek for SliceReader<'a> {
    fn seek(&mut self, seek: SeekFrom) -> std::io::Result<u64> {
        let pos = match seek {
            SeekFrom::Start(offset) => offset as usize,
            SeekFrom::End(offset) => self.slices.iter().map(|s| s.len()).sum::<usize>() - offset as usize,
            SeekFrom::Current(offset) => (self.index + self.inner_index) + offset as usize,
        };

        self.index = 0;
        self.inner_index = 0;
        let mut total = 0;

        for (i, slice) in self.slices.iter().enumerate() {
            if total + slice.len() > pos {
                self.index = i;
                self.inner_index = pos - total;
                return Ok(pos as u64);
            }
            total += slice.len();
        }

        Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid seek"))
    }
}

enum Output {
    Tarball(Vec<u8>),
    File(File),
}

impl Write for Output {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Tarball(data) => data.write(buf),
            Self::File(file) => file.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Tarball(data) => data.flush(),
            Self::File(file) => file.flush(),
        }
    }
}
