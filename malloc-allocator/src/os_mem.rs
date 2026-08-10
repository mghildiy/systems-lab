use std::ffi::c_void;
use std::ptr;
use std::sync::OnceLock;

static PAGE_SIZE: OnceLock<usize> = OnceLock::new();
pub(crate) fn page_size() -> usize {
    *PAGE_SIZE.get_or_init(|| unsafe { libc::sysconf(libc::_SC_PAGESIZE) as usize })
}
pub(crate) unsafe fn map_pages(num_pages: usize) -> Option<*mut u8> {
    let memory_size = num_pages * page_size();
    unsafe {
        let addr = libc::mmap(
            ptr::null_mut(),
            memory_size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
            -1,
            0,
        );

        if addr == libc::MAP_FAILED {
            return None;
        }
        Some(addr as *mut u8)
    }
}
pub(crate) unsafe fn unmap_pages(addr: *mut u8, num_pages: usize) -> Result<(), std::io::Error> {
    unsafe {
        let result = libc::munmap(addr as *mut c_void, num_pages * page_size());
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_pages() {
        unsafe {
            let addr = map_pages(1).expect("failed to map pages");

            // write to first 2 bytes
            *addr.add(0) = 42;
            *addr.add(1) = 50;

            // read and verify
            assert_eq!(*addr.add(0), 42, "First memory location must have value 42");
            assert_eq!(*addr.add(1), 50, "Second memory location must have value 50");
            let page_size = page_size();
            for index in 2..page_size {
                assert_eq!(*addr.add(index), 0, "Memory location {} must be untouched with value 0", index);
            }
        }
    }

    #[test]
    fn test_unmap_pages() {
        unsafe {
            let addr = map_pages(2).expect("failed to map pages");
            unmap_pages(addr, 2).expect("failed to unmap pages");
        }
    }

    #[test]
    fn test_unmap_pages_zero_length_fails() {
        unsafe {
            let addr = map_pages(2).expect("failed to map pages");
            let result = unmap_pages(addr, 0);
            assert!(result.is_err());
            // first call failed, so mapping is still live — clean up for real
            unmap_pages(addr, 2).expect("cleanup failed");
        }
    }

    #[test]
    fn test_unmap_pages_nonaligned_address_fails() {
        unsafe {
            let addr = map_pages(2).expect("failed to map pages");
            let result = unmap_pages(addr.add(200), 2);
            assert!(result.is_err());
            // first call failed, so mapping is still live — clean up for real
            unmap_pages(addr, 2).expect("cleanup failed");
        }
    }

    #[test]
    fn test_unmap_pages_middle_of_mapping_succeeds_but_creates_gap() {
        unsafe {
            let addr = map_pages(4).expect("failed to map pages");
            let result = unmap_pages(addr.add(page_size()), 1);
            assert!(result.is_ok());

            *addr.add(0) = 42;
            assert_eq!(*addr.add(0), 42, "First memory location must have value 42");
            *addr.add(2 * page_size()) = 41;
            assert_eq!(*addr.add(2 * page_size()), 41, "Memory location {} must have value 41", 2 * page_size());
            *addr.add(3 * page_size()) = 40;
            assert_eq!(*addr.add(3 * page_size()), 40, "Memory location {} must have value 40", 3 * page_size());

            unmap_pages(addr, 1).expect("cleanup page 0 failed"); // clean page 0 alone
            unmap_pages(addr.add(2 * page_size()), 2).expect("cleanup pages 2-3 failed"); // clean pages 2,3 together
        }
    }
}

