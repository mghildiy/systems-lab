use std::fmt;
use std::fmt::Display;


/// Errors that can occur while allocating memory.
#[derive(Debug)]
pub enum AllocError {
    ZeroSizeRequest,
    OutOfMemory,
    OsError(std::io::Error),
    RegionAllocationFailed
}

impl Display for AllocError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AllocError::ZeroSizeRequest => write!(f, "Can't allocate zero size memory block"),
            AllocError::OutOfMemory => write!(f, "System does not have enough memory"),
            AllocError::OsError(io_error) => write!(f, "OS error: {}", io_error),
            AllocError::RegionAllocationFailed => write!(f, "Memory allocation failed due to unknown reason"),
        }
    }
}