//! A bump allocator: enough for the handful of `Box`/`Vec` allocations the
//! loader and one instance make, and no allocator dependency at all.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

const ARENA_LEN: usize = 64 * 1024;

#[repr(align(16))]
struct Arena([u8; ARENA_LEN]);

static mut ARENA: Arena = Arena([0; ARENA_LEN]);
static USED: AtomicUsize = AtomicUsize::new(0);

struct Bump;

unsafe impl GlobalAlloc for Bump {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let start = (USED.load(Ordering::Relaxed) + align - 1) & !(align - 1);
        let end = start + layout.size();
        if end > ARENA_LEN {
            return ptr::null_mut();
        }
        USED.store(end, Ordering::Relaxed);
        unsafe { core::ptr::addr_of_mut!(ARENA.0).cast::<u8>().add(start) }
    }

    unsafe fn dealloc(&self, _ptr: *mut u8, _layout: Layout) {}
}

#[global_allocator]
static ALLOCATOR: Bump = Bump;
