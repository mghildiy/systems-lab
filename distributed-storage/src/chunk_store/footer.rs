use std::array::TryFromSliceError;
use std::ops::Range;
use crate::chunk_store::error::ChunkError;

const MAGIC_BYTES: [u8; 4] = *b"CHNK";
const FOOTER_SIZE: usize = 16;
const LEN_RANGE: Range<usize> = 0..8;
const CHECKSUM_RANGE: Range<usize> = 8..12;
const MAGIC_RANGE: Range<usize> = 12..16;

#[derive(Debug, PartialEq)]
pub struct Footer {
    pub data_len: u64,  // 8 bytes for data length
    pub checksum: u32   // 4 bytes for checksum
}

impl Footer {
    pub fn for_data(data: &[u8]) -> Self {
        Footer {
            data_len: data.len() as u64,
            checksum: crc32c::crc32c(data)
        }
    }

    pub fn encode(&self) -> [u8; FOOTER_SIZE] {
        let mut encoded = [0u8; FOOTER_SIZE];
        encoded[LEN_RANGE].copy_from_slice(&self.data_len.to_be_bytes());
        encoded[CHECKSUM_RANGE].copy_from_slice(&self.checksum.to_be_bytes());
        encoded[MAGIC_RANGE].copy_from_slice(&MAGIC_BYTES);

        encoded
    }

    pub fn decode(raw: &[u8; FOOTER_SIZE]) -> Result<Footer, ChunkError>  {
        if raw[MAGIC_RANGE] != MAGIC_BYTES {
            return Err(ChunkError::MagicByteMismatch);
        }

        let data_len: [u8; 8] = raw[LEN_RANGE].try_into().unwrap();
        let checksum: [u8; 4] = raw[CHECKSUM_RANGE].try_into().unwrap();

        Ok(Footer {
            data_len: u64::from_be_bytes(data_len),
            checksum: u32::from_be_bytes(checksum)
        })
    }

}

#[cfg(test)]
mod tests {
    use crate::chunk_store::error::ChunkError;
    use crate::chunk_store::footer::{Footer, CHECKSUM_RANGE, LEN_RANGE, MAGIC_BYTES, MAGIC_RANGE};

    #[test]
    fn test_encode_decode() {
        let footer = Footer::for_data(b"This is chunk footer test.");
        let encoded = footer.encode();

        let data_len: [u8; 8] = encoded[LEN_RANGE].try_into().unwrap();
        assert_eq!(u64::from_be_bytes(data_len), 26);
        let checksum: [u8; 4] = encoded[CHECKSUM_RANGE].try_into().unwrap();
        assert_eq!(u32::from_be_bytes(checksum), crc32c::crc32c("This is chunk footer test.".as_ref()));
        assert_eq!(&encoded[MAGIC_RANGE], &MAGIC_BYTES[..]);

        let decoded = Footer::decode(&encoded).expect("Successful footer decode must happen.");
        assert_eq!(footer, decoded);
    }

    #[test]
    fn test_bad_magic_bytes() {
        let footer = Footer::for_data(b"Test chunk.");
        let mut encoded = footer.encode();
        encoded[MAGIC_RANGE.start + 2] = b'U';

        let decoded = Footer::decode(&encoded);
        assert!(decoded.is_err());
        assert_eq!(decoded.unwrap_err(), ChunkError::MagicByteMismatch);
    }
}