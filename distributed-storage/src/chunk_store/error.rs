use std::fmt::Display;

#[derive(Debug)]
#[derive(PartialEq)]
pub enum ChunkError {
    MagicByteMismatch
}

impl Display for ChunkError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ChunkError::MagicByteMismatch => {write!(f,"Magic bytes mismatch in chunk footer")}
        }
    }
}

