use crate::error::ChecksumError;
use md5::Md5;
use sha1::Sha1;
use sha2::{Digest, Sha224, Sha256, Sha384, Sha512};

pub struct Checksum {
    hasher: Hasher,
    contents: String,
}

impl Checksum {
    pub fn new_inner(hash: impl Into<String>, checksum_type: CsType) -> Self {
        Self {
            hasher: checksum_type.into(),
            contents: hash.into(),
        }
    }
    pub fn new(hash: impl Into<String>) -> Result<Self, ChecksumError> {
        let hash = hash.into();
        let checksum_type = match hash.len() {
            32 => CsType::MD5,
            40 => CsType::Sha1,
            56 => CsType::Sha224,
            64 => CsType::Sha256,
            96 => CsType::Sha384,
            128 => CsType::Sha512,
            _ => return Err(ChecksumError::UnrecognizedSize),
        };
        Ok(Self::new_inner(hash, checksum_type))
    }
    pub fn update(&mut self, data: &[u8]) {
        match &mut self.hasher {
            Hasher::Md5(hasher) => hasher.update(data),
            Hasher::Sha1(hasher) => hasher.update(data),
            Hasher::Sha224(hasher) => hasher.update(data),
            Hasher::Sha256(hasher) => hasher.update(data),
            Hasher::Sha384(hasher) => hasher.update(data),
            Hasher::Sha512(hasher) => hasher.update(data),
        }
    }
    pub fn verify(self) -> bool {
        let hash = match self.hasher {
            Hasher::Md5(hasher) => format!("{:x}", hasher.finalize()),
            Hasher::Sha1(hasher) => format!("{:x}", hasher.finalize()),
            Hasher::Sha224(hasher) => format!("{:x}", hasher.finalize()),
            Hasher::Sha256(hasher) => format!("{:x}", hasher.finalize()),
            Hasher::Sha384(hasher) => format!("{:x}", hasher.finalize()),
            Hasher::Sha512(hasher) => format!("{:x}", hasher.finalize()),
        };
        hash == self.contents
    }
}

pub enum Hasher {
    Md5(Md5),
    Sha1(Sha1),
    Sha224(Sha224),
    Sha256(Sha256),
    Sha384(Sha384),
    Sha512(Sha512),
}
impl From<CsType> for Hasher {
    fn from(value: CsType) -> Self {
        match value {
            CsType::MD5 => Self::Md5(Md5::new()),
            CsType::Sha1 => Self::Sha1(Sha1::new()),
            CsType::Sha224 => Self::Sha224(Sha224::new()),
            CsType::Sha256 => Self::Sha256(Sha256::new()),
            CsType::Sha384 => Self::Sha384(Sha384::new()),
            CsType::Sha512 => Self::Sha512(Sha512::new()),
        }
    }
}

pub enum CsType {
    MD5,
    Sha1,
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}
