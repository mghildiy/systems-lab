use crate::region::request_block;

pub struct Allocator {
    first_region: *mut u8,
}

impl Allocator {
    pub unsafe fn malloc(self, size: usize) -> *mut u8 {
        request_block(self.first_region, size)
    }
}
