//! A counting global allocator for measuring live heap bytes.
//!
//! Install [`Counting`] as the `#[global_allocator]` of a binary. Counting is off by default so
//! timed repetitions do not pay for atomic updates; [`measure::run_reps`](crate::measure::run_reps)
//! turns it on for one warm-up repetition.

#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};

static ENABLED: AtomicBool = AtomicBool::new(false);
static LIVE: AtomicUsize = AtomicUsize::new(0);

/// Forwards to the system allocator and, while enabled, tracks live requested bytes.
pub struct Counting;

// SAFETY: every method forwards to `System`, which upholds the `GlobalAlloc` contract. The
// counters are bookkeeping only and never affect the returned pointers.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which `System` shares.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() && ENABLED.load(Relaxed) {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract, which `System` shares.
        unsafe { System.dealloc(ptr, layout) };
        if ENABLED.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
        }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() && ENABLED.load(Relaxed) {
            LIVE.fetch_add(layout.size(), Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() && ENABLED.load(Relaxed) {
            LIVE.fetch_sub(layout.size(), Relaxed);
            LIVE.fetch_add(new_size, Relaxed);
        }
        new_ptr
    }
}

/// Turns byte counting on or off. Only blocks allocated and freed inside one enabled window
/// are accounted correctly, so enable it around a build-use-drop cycle.
pub fn set_counting(on: bool) {
    ENABLED.store(on, Relaxed);
}

/// Bytes currently live while counting is enabled; always 0 if [`Counting`] is not installed.
pub fn live_bytes() -> usize {
    LIVE.load(Relaxed)
}
