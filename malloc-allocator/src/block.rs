
/// Mask isolating the lowest bit, used to store the allocated flag inside
/// a block's size word.
const ALLOC_FLAG_MASK: usize = 1;

/// All block sizes are rounded up to a multiple of this value. Chosen to
/// match usize's width so the lowest bit is always free for ALLOC_FLAG_MASK.
const ALIGNMENT: usize = size_of::<usize>();

struct BlockHeader {
    packed: usize,
}

impl BlockHeader {

    /// Packs `size` and `allocated` into a single word: the allocated flag
    /// occupies the lowest bit, which is always free because every valid
    /// size is a multiple of ALIGNMENT (so its low bit is guaranteed zero).
    fn new(size: usize, allocated: bool) -> Self {
        BlockHeader {
            packed: size | if allocated { ALLOC_FLAG_MASK } else { 0 },
        }
    }


    /// Sets or clears the allocated flag (the lowest bit) without disturbing
    /// the size bits stored in the rest of the word.
    fn set_allocated(&mut self, allocated: bool) {
        if allocated {
            self.packed = self.packed | ALLOC_FLAG_MASK
        } else {
            self.packed = self.packed & !ALLOC_FLAG_MASK
        }
    }

    /// Returns the block's total size by masking off the low allocated-flag bit.
    pub(crate) fn size(&self) -> usize {
        self.packed & !ALLOC_FLAG_MASK
    }

    /// Returns whether the block is currently allocated, by checking the low bit.
    pub(crate) fn is_allocated(&self) -> bool {
        (self.packed & ALLOC_FLAG_MASK) != 0
    }
}

/// Writes a block header at `addr`, encoding `size` (total block size,
/// header included) and `allocated` into a single packed word.
pub(crate) unsafe fn write_header(addr: *mut u8, size: usize, allocated: bool) {
    let header_addr = addr as *mut BlockHeader;
    let block_header = BlockHeader::new(size, allocated);
    unsafe { header_addr.write(block_header) }
}


/// Reads the block header at `addr`, returning (total block size, allocated flag).
pub(crate) unsafe fn read_header(addr: *mut u8) -> (usize, bool) {
    let header_addr = addr as *mut BlockHeader;
    let header = unsafe { &*header_addr };
    (header.size(), header.is_allocated())
}


/// Returns the number of bytes a `BlockHeader` occupies in memory.
pub(crate) fn block_header_size() -> usize {
    size_of::<BlockHeader>()
}


/// Converts a raw client-requested size into the total block size that must
/// be reserved: the header's own size plus the payload rounded up to `ALIGNMENT`.
pub(crate) fn total_block_size(raw_size: usize) -> usize {
    let with_header = raw_size + size_of::<BlockHeader>();
    round_up_to_alignment(with_header)
}


/// Rounds `n` up to the next multiple of `ALIGNMENT`.
/// Formula: add (ALIGNMENT - 1) to push past the next boundary, then mask
/// off the low bits to snap down to that boundary. Requires ALIGNMENT to be
/// a power of 2 for the bitmask trick to work correctly.
fn round_up_to_alignment(n: usize) -> usize {
    (n + ALIGNMENT - 1) & !(ALIGNMENT - 1)
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
