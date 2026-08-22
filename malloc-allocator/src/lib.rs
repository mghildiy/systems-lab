// memory: static, Sync , UnsafeCell, GlobalAlloc trait
// counter to track how much memory has been consumed: static, Sync, without Mutex for fast ops(use atomic)

mod allocator;
pub mod block;
pub mod os_mem;
mod region;
mod errors;
