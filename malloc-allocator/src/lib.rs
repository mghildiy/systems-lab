// memory: static, Sync , UnsafeCell, GlobalAlloc trait
// counter to track how much memory has been consumed: static, Sync, without Mutex for fast ops(use atomic)

pub mod os_mem;
pub mod block;
mod region;

