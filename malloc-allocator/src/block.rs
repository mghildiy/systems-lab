const ALLOC_FLAG_MASK: usize = 1;

struct BlockHeader {
    packed: usize,
}

impl BlockHeader {
    fn new(size: usize, allocated: bool) -> Self {
        BlockHeader {
            packed: size | if allocated { ALLOC_FLAG_MASK } else { 0 },
        }
    }

    fn set_allocated(&mut self, allocated: bool) {
        if allocated {
            self.packed = self.packed | ALLOC_FLAG_MASK
        } else {
            self.packed = self.packed & !ALLOC_FLAG_MASK
        }
    }
    pub(crate) fn size(&self) -> usize {
        self.packed & !ALLOC_FLAG_MASK
    }

    pub(crate) fn is_allocated(&self) -> bool {
        (self.packed & ALLOC_FLAG_MASK) != 0
    }
}

pub(crate) unsafe fn write_header(addr: *mut u8, size: usize, allocated: bool) {
    let header_addr = addr as *mut BlockHeader;
    let block_header = BlockHeader::new(size, allocated);
    unsafe { header_addr.write(block_header) }
}

pub(crate) unsafe fn read_header(addr: *mut u8) -> (usize, bool) {
    let header_addr = addr as *mut BlockHeader;
    let header = unsafe { &*header_addr };
    (header.size(), header.is_allocated())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os_mem::{map_pages, unmap_pages};

    #[test]
    fn test_block_header() {
        let mut header = BlockHeader::new(512, true);

        assert_eq!(header.size(), 512);
        assert_eq!(header.is_allocated(), true);

        header.set_allocated(false);

        assert_eq!(header.is_allocated(), false);
    }

    #[test]
    fn test_write_and_read_heeader() {
        unsafe {
            let addr = map_pages(1).expect("failed to map pages");
            // use aligned size ie multiple of 8
            write_header(addr, 80, true);
            let (size, allocated) = read_header(addr);
            assert_eq!(size, 80);
            assert_eq!(allocated, true);
            unmap_pages(addr, 1).expect("cleanup failed");
        }
    }
}
