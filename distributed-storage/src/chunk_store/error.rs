use std::fmt::Display;
use std::io::ErrorKind;
use std::path::PathBuf;

#[derive(Debug)]
pub enum ChunkError {
    MagicByteMismatch,
    StoreRootError(PathBuf, ErrorKind),
    Io(std::io::Error),
    FileTooShort { actual: u64 },
    FileLengthMismatch { actual: u64, expected: u64 },
    ChecksumMismatch { expected: u32, actual: u32 },
}

impl From<std::io::Error> for ChunkError {
    fn from(e: std::io::Error) -> Self {
        ChunkError::Io(e)
    }
}

impl Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ChunkError::MagicByteMismatch => {write!(f,"Magic bytes mismatch in chunk footer")},
            ChunkError::StoreRootError(p, e) => {write!(f,"Store root creation error: {} at path: {}", e, p.display())},
            ChunkError::Io(e) => {write!(f,"I/O error while saving chunk: {}", e)},
            ChunkError::FileTooShort { actual } => write!(f,"Chunk size is {} bytes, minimum 16 bytes expected", actual),
            ChunkError::FileLengthMismatch { actual, expected} => write!(f, "Chunk length is {} bytes, but is expected to be {} bytes", actual, expected),
            ChunkError::ChecksumMismatch { actual, expected} => write!(f, "Chunk checksum is {}, but is expected to be {}", actual, expected),
        }
    }
}

