use crate::block::{read_header, write_header};
use crate::os_mem::{map_pages, page_size};

struct RegionHeader {
    frontier: *mut u8,
    size: usize,
    next_region: *mut RegionHeader,
}

impl RegionHeader {
    fn new(start: *mut u8, size: usize) -> Self {
        RegionHeader {
            frontier: unsafe { start.add(std::mem::size_of::<RegionHeader>()) },
            size,
            next_region: std::ptr::null_mut(),
        }
    }

    unsafe fn get_block(&mut self, requested_size: usize) -> *mut u8 {
        match self.walk_blocks(requested_size) {
            None => {
                return if !self.next_region.is_null() {
                    self.next_region.get_block(requested_size)
                } else {
                    *map_pages(2).get_or_insert(std::ptr::null_mut())
                };
            }
            Some(addr) => addr,
        }
    }

    unsafe fn walk_blocks(&mut self, size: usize) -> Option<*mut u8> {
        let region_header_addr = std::ptr::from_mut(self) as *mut u8;
        let mut block_header_addr = unsafe { region_header_addr.add(size_of::<RegionHeader>()) };
        let mut bytes_so_far = 0;
        while block_header_addr != self.frontier {
            let (block_size, is_allocated) = unsafe { read_header(block_header_addr) };
            bytes_so_far += block_size;
            if !is_allocated && block_size >= size {
                unsafe { write_header(block_header_addr, block_size, true) };
                return Some(block_header_addr);
            }
            block_header_addr = unsafe { block_header_addr.add(block_size) };
        }
        let region_header_end_addr = region_header_addr.add(self.size);
        let remaining_space = region_header_end_addr as usize - self.frontier as usize;
        if remaining_space >= size {
            unsafe { write_header(self.frontier, size, true) };
            self.frontier = unsafe { self.frontier.add(size) };
            return Some(self.frontier);
        }
        None
    }
}

pub(crate) unsafe fn create_region(num_pages: usize) -> Option<*mut u8> {
    let addr = unsafe { map_pages(num_pages)? };
    let region_header = RegionHeader::new(addr, num_pages * page_size());
    let region_header_ptr = addr as *mut RegionHeader;
    unsafe { region_header_ptr.write(region_header) };
    Some(addr)
}

// TODO take care of negative usize
pub(crate) unsafe fn request_block(region_addr: *mut u8, block_size: usize) -> *mut u8 {
    let region = &mut *(region_addr as *mut RegionHeader);
    region.get_block(block_size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::os_mem::unmap_pages;

    #[test]
    fn test_create_region() {
        let addr = unsafe { create_region(2).expect("Failed to create region") };
        let region_header_addr = addr as *mut RegionHeader;
        let region_header = unsafe { &*region_header_addr };
        assert_eq!(region_header.frontier, unsafe {
            addr.add(std::mem::size_of::<RegionHeader>())
        });
        assert_eq!(region_header.size, 2 * page_size());
        assert!(region_header.next_region.is_null());
        unsafe {
            unmap_pages(addr, 2).expect("Failed to free memory");
        }
    }
}
