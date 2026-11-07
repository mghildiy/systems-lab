use crate::chunk_store::error::ChunkError;
use crate::chunk_store::error::ChunkError::{ChecksumMismatch, FileLengthMismatch, FileTooShort, StoreRootError};
use crate::chunk_store::footer::{Footer, FOOTER_SIZE};
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

pub struct ChunkStore {
    path: PathBuf
}

impl ChunkStore {
    pub fn open(root: &PathBuf) -> Result<ChunkStore, ChunkError> {
        match fs::create_dir_all(&root) {
            Ok(_) => {
                cleanup_partial_files(&root)?;
                Ok(ChunkStore{ path: root.clone()})
            },
            Err(e) => {
                eprintln!("Error while creating store root directory: {:?}", e);
                Err(StoreRootError(root.clone(), e.kind()))
            }
        }
    }

    pub fn put(&self, id: &str, data: &[u8]) -> Result<(), ChunkError> {
        let footer = Footer::for_data(data).encode();
        let tmp_path = self.path.join(format!("{id}.tmp"));
        let final_path = self.path.join(id);
        let mut file = File::create(&tmp_path)?;
        file.write_all(data)?;
        file.write_all(&footer)?;
        file.sync_all()?;
        fs::rename(&tmp_path, &final_path)?;
        File::open(&self.path)?.sync_all()?;

        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<Vec<u8>, ChunkError> {
        let chunk_path = self.path.join(id);
        let mut file = File::open(&chunk_path)?;
        let chunk_length = file.metadata()?.len();
        if chunk_length < FOOTER_SIZE as u64 {
            return Err(FileTooShort { actual: chunk_length });
        }

        file.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let mut raw_footer = [0u8; FOOTER_SIZE];
        file.read_exact(&mut raw_footer)?;
        let footer = Footer::decode(&raw_footer)?;
        let actual_data_len = chunk_length - FOOTER_SIZE as u64;
        if footer.data_len != actual_data_len {
            return Err(FileLengthMismatch { expected: footer.data_len, actual: actual_data_len });
        }

        file.seek(SeekFrom::Start(0))?;
        let mut content = vec![0u8; footer.data_len as usize];
        file.read_exact(&mut content)?;
        let checksum = crc32c::crc32c(&content);
        if checksum != footer.checksum {
            return Err(ChecksumMismatch { expected: footer.checksum, actual: checksum });
        }

        Ok(content)
    }
}

fn cleanup_partial_files(path: &PathBuf) -> Result<(), ChunkError> {
    match fs::read_dir(&path) {
        Ok(readdir) => {
            for entry in readdir {
                if let Ok(entry) = entry {
                    let entry_path = entry.path();
                    if entry_path.is_file() && entry_path.extension() == Some(std::ffi::OsStr::new("tmp")) {
                        if let Err(e ) =  fs::remove_file(&entry_path) {
                            eprintln!("Error while cleaning partial files: {:?}", e);
                            return Err(StoreRootError(entry_path, e.kind()))
                        }
                    }
                }
            }
            Ok(())
        },
        Err(e) => {
            eprintln!("Error while reading store root directory: {:?}", e);
            Err(StoreRootError(path.clone(), e.kind()))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::chunk_store::chunk_store::ChunkStore;
    use std::fs;
    use tempfile::tempdir;
    use crate::chunk_store::error::ChunkError::{ChecksumMismatch, FileLengthMismatch, FileTooShort, MagicByteMismatch};

    #[test]
    fn test_open() {
        let temp_dir = tempdir().expect("failed to create temp directory");
        let root = temp_dir.keep();

        let chunk_store = ChunkStore::open(&root).expect("Failed to open store root.");
        assert_eq!(chunk_store.path, root, "Chunk store root path different from expected.");
        assert!(chunk_store.path.is_dir(), "Directory expected, found file.");
        let root_dir = fs::read_dir(chunk_store.path).expect("Failed to read root directory.");
        assert_eq!(root_dir.into_iter().count(), 0, "Root directory must be empty.");
    }

    #[test]
    fn test_open_clean_up() {
        let temp_dir = tempdir().expect("failed to create temp directory");
        let root = temp_dir.keep();
        let file_path = root.join("test-file.tmp");
        fs::write(&file_path, "Hello, distributed storage!").expect("Failed to write file");
        assert!(file_path.exists(), "test setup failed: .tmp file was not created");

        let chunk_store = ChunkStore::open(&root).expect("Failed to open store root.");
        let root_dir = fs::read_dir(chunk_store.path).expect("Failed to read root directory.");
        assert_eq!(root_dir.into_iter().count(), 0, "Tmp file must be cleaned up.");
    }

    #[test]
    fn test_put_get_good_flow() {
        let temp_dir = tempdir().unwrap();
        let store = ChunkStore::open(&temp_dir.path().to_path_buf()).unwrap();

        store.put("test_file", "Hello, distributed storage!".as_bytes()).expect("Failed to put chunk");
        let chunk_content = store.get("test_file").expect("Failed to get chunk data");
        assert_eq!(chunk_content, "Hello, distributed storage!".as_bytes());
    }

    #[test]
    fn test_get_detects_corrupted_data() {
        let temp_dir = tempdir().unwrap();
        let store = ChunkStore::open(&temp_dir.path().to_path_buf()).unwrap();

        store.put("c1", b"Hello, distributed storage!").unwrap();

        let path = temp_dir.path().join("c1");
        let mut bytes = fs::read(&path).unwrap();
        bytes[0] ^= 0xFF;
        fs::write(&path, bytes).unwrap();

        let result = store.get("c1");
        assert!(matches!(result, Err(ChecksumMismatch { .. })));
    }

    #[test]
    fn test_get_detects_corrupted_footer() {
        let temp_dir = tempdir().unwrap();
        let store = ChunkStore::open(&temp_dir.path().to_path_buf()).unwrap();

        store.put("c1", b"Hello, distributed storage!").unwrap();

        let path = temp_dir.path().join("c1");
        let mut bytes = fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        fs::write(&path, bytes).unwrap();

        let result = store.get("c1");
        assert!(matches!(result, Err(MagicByteMismatch { .. })));
    }

    #[test]
    fn test_get_rejects_file_shorter_than_footer() {
        let temp_dir = tempdir().unwrap();
        let store = ChunkStore::open(&temp_dir.path().to_path_buf()).unwrap();

        store.put("c1", b"Hello, distributed storage!").unwrap();

        let path = temp_dir.path().join("c1");
        let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
        file.set_len(10).unwrap();   // any value below 16

        let result = store.get("c1");
        assert!(matches!(result, Err(FileTooShort { actual: 10 })));
    }

    #[test]
    fn test_get_rejects_file_length_mismatch() {
        let temp_dir = tempdir().unwrap();
        let store = ChunkStore::open(&temp_dir.path().to_path_buf()).unwrap();

        store.put("c1", b"Hello, distributed storage!").unwrap();

        let path = temp_dir.path().join("c1");
        let mut bytes = fs::read(&path).unwrap();
        let len_start = bytes.len() - 16;      // start of footer = start of length field
        let len_end = bytes.len()- 8;         // end of length field
        let old = u64::from_be_bytes(bytes[len_start..len_end].try_into().unwrap());
        let fake_len: u64 = old + 5;   // set a diff value
        bytes[len_start..len_end].copy_from_slice(&fake_len.to_be_bytes());
        fs::write(&path, bytes).unwrap();

        let result = store.get("c1");
        assert!(matches!(result, Err(FileLengthMismatch { .. })));
    }
}