use crate::os_mem::{map_pages, page_size};

struct RegionHeader {
    frontier: *mut u8,
    size: usize,
    next_region: *mut RegionHeader
}

impl RegionHeader {
    fn new(start: *mut u8, size: usize) -> Self {
        RegionHeader {
            frontier: unsafe { start.add(std::mem::size_of::<RegionHeader>()) },
            size,
            next_region: std::ptr::null_mut()
        }
    }
}

pub(crate) unsafe fn create_region(num_pages: usize) -> Option<*mut u8> {
    let addr = unsafe { map_pages(num_pages)? };
    let region_header = RegionHeader::new(addr, num_pages * page_size());
    let region_header_ptr = addr as *mut RegionHeader;
    unsafe { region_header_ptr.write(region_header) };
    Some(addr)
}

#[cfg(test)]
mod tests {
    use crate::os_mem::unmap_pages;
    use super::*;

    #[test]
    fn test_create_region() {
        let addr = unsafe { create_region(2).expect("Failed to create region") };
        let region_header_addr = addr as *mut RegionHeader;
        let region_header = unsafe { &*region_header_addr };
        assert_eq!(region_header.frontier, unsafe { addr.add(std::mem::size_of::<RegionHeader>()) }) ;
        assert_eq!(region_header.size, 2 * page_size());
        assert!(region_header.next_region.is_null());
        unsafe { unmap_pages(addr, 2).expect("Failed to free memory") ; }
    }
}