use crate::block::{block_header_size, read_header, total_block_size, write_header};
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

    unsafe fn get_block(&mut self, requested_size: usize) -> Option<*mut u8> {
        let total_size = total_block_size(requested_size);
        match unsafe { self.walk_blocks(total_size) } {
            None => {
                // create next region, and get block from it
                if self.next_region.is_null() {
                    let total_needed = size_of::<RegionHeader>() + total_size;
                    let num_pages = (total_needed + page_size() - 1) / page_size();
                    let region_header = unsafe { create_region(num_pages)? } as *mut RegionHeader;
                    self.next_region = region_header;
                    unsafe { (*region_header).get_block(requested_size) }
                } else {
                    unsafe { (&mut *self.next_region).get_block(requested_size) }
                }
            }
            Some(addr) => Some (addr),
        }
    }

    unsafe fn walk_blocks(&mut self, size: usize) -> Option<*mut u8> {
        let region_header_addr = std::ptr::from_mut(self) as *mut u8;
        let mut block_header_addr = unsafe { region_header_addr.add(size_of::<RegionHeader>()) };
        while block_header_addr != self.frontier {
            let (block_size, is_allocated) = unsafe { read_header(block_header_addr) };
            if !is_allocated && block_size >= size {
                unsafe { write_header(block_header_addr, block_size, true) };
                return Some(block_header_addr);
            }
            block_header_addr = unsafe { block_header_addr.add(block_size) };
        }
        let region_header_end_addr = unsafe { region_header_addr.add(self.size) };
        let remaining_space = region_header_end_addr as usize - self.frontier as usize;
        if remaining_space >= size {
            let new_block_addr = self.frontier;
            // write block header
            unsafe { write_header(new_block_addr, size, true) };
            // advance frontier
            self.frontier = unsafe { self.frontier.add(size) };
            return Some(new_block_addr);
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

pub(crate) fn region_header_size() -> usize {
    size_of::<RegionHeader>()
}

pub(crate) fn has_next_region(region_addr: *const u8) -> bool {
    let region_header = region_addr as *const RegionHeader;
    unsafe { !(*region_header).next_region.is_null() }
}

// TODO take care of negative usize
pub(crate) unsafe fn request_block(region_addr: *mut u8, block_size: usize) ->
        Option<(*mut u8, *mut u8)> {
    if region_addr.is_null() {
        // ask for pages just enough for this request
        let total_size = total_block_size(block_size);
        let total_needed = size_of::<RegionHeader>() + total_size;
        let num_pages = (total_needed + page_size() - 1) / page_size();
        let region_addr = unsafe { create_region(num_pages)? };
        let region = unsafe { &mut *(region_addr as *mut RegionHeader) };
        let block_header_addr = unsafe { region.get_block(block_size)? };
        let payload_addr = unsafe { block_header_addr.add(block_header_size()) };
        Some((region_addr, payload_addr))
    } else {
        let region = unsafe { &mut *(region_addr as *mut RegionHeader) };
        let block_header_addr = unsafe { region.get_block(block_size)? };
        let payload_addr = unsafe { block_header_addr.add(block_header_size()) };
        Some((region_addr, payload_addr))
    }
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
