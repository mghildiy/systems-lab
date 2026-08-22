use crate::errors::AllocError;
use crate::errors::AllocError::{OutOfMemory, ZeroSizeRequest};
use crate::region::request_block;

pub struct Allocator {
    first_region: *mut u8,
}

impl Allocator {

    pub fn new() -> Allocator {
        Allocator {
            first_region: std::ptr::null_mut()
        }
    }
    pub unsafe fn malloc(&mut self, size: usize) -> Result<*mut u8, AllocError> {
        if size == 0 {
            return Err(ZeroSizeRequest);
        }

        match unsafe { request_block(self.first_region, size) } {
            Ok((region_addr, block_addr)) => {
                self.first_region = region_addr;
                Ok(block_addr)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write;
    use crate::block::{block_header_size, total_block_size};
    use crate::os_mem::page_size;
    use crate::region::{has_next_region, region_header_size};
    use super::*;

    #[test]
    fn zero_malloc() {
        unsafe {
            let mut allocator = Allocator::new();
            let result = allocator.malloc(0);

            match result {
                Err(ZeroSizeRequest) => (),
                _ => assert!(false)
            }
        }
    }

    #[test]
    fn single_malloc() {
        unsafe {
            let mut allocator = Allocator::new();
            let addr = allocator.malloc(10).expect("alloc failed");

            for i in 0..10u8 {
                *addr.add(i as usize) = i;
            }

            for i in 0..10u8 {
                assert_eq!(*addr.add(i as usize),  i, "Wrong value found");
            }
        }
    }

    #[test]
    fn multi_alloc_same_region() {
        unsafe {
            let mut allocator = Allocator::new();
            let first_block_payload_addr = allocator.malloc(10).expect("alloc failed");
            let second_block_payload_addr = allocator.malloc(7).expect("alloc failed");

            assert_eq!(
                second_block_payload_addr as usize - first_block_payload_addr as usize,
                total_block_size(10),
                "Gap between allocations should equal first block's total size"
            );
        }
    }

    #[test]
    fn large__alloc_then_snall_alloc_then_gap_check() {
        unsafe {
            let mut allocator = Allocator::new();
            let size = page_size() * 2 + 500;

            let first_addr = allocator.malloc(size).expect("alloc failed");

            // spot-check the large allocation is genuinely usable across its full span
            let positions = [0, 1, size / 2, size - 2, size - 1];
            for &i in &positions {
                *first_addr.add(i) = 42;
                assert_eq!(*first_addr.add(i), 42, "Mismatch at offset {}", i);
            }

            // second, smaller allocation — confirm gap math holds at this scale too
            let second_addr = allocator.malloc(7).expect("alloc failed");
            assert_eq!(
                second_addr as usize - first_addr as usize,
                total_block_size(size),
                "Gap between large first block and second block should equal first block's total size"
            );
        }
    }

    #[test]
    fn test_malloc_creates_new_region_when_first_is_full() {
        unsafe {
            let mut allocator = Allocator::new();
            let page_size = page_size();

            let first_addr = allocator.malloc(page_size).expect("alloc failed");
            assert!(!has_next_region(allocator.first_region), "should have no next region yet after first alloc");

            let second_addr = allocator.malloc(page_size).expect("alloc failed");
            assert!(has_next_region(allocator.first_region), "second alloc should have forced a new, linked region");
        }
    }
}
