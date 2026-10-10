//! glibc allocator policy for the long-lived daemon.
//!
//! A 40-turn tool-heavy session grew `forge serve` from 282 MB to 401 MB while the live heap
//! stayed near 100 MB: the rest was free memory glibc kept. Every tokio worker and blocking-pool
//! thread gets its own malloc arena, each large short-lived buffer (a request body, a serialized
//! frame, a tool result) is carved from whichever arena the thread uses, and an arena only returns
//! memory from the end of its heap. Capping the arenas and handing free pages back at turn
//! boundaries keeps the resident size tracking the live heap. `FORGE_ALLOC_POLICY=off` restores the
//! stock behaviour for measuring. Other platforms are untouched.

#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};

    const ARENAS: libc::c_int = 2;

    fn disabled() -> bool {
        std::env::var("FORGE_ALLOC_POLICY").is_ok_and(|v| v == "off")
    }

    pub fn init() {
        if disabled() {
            return;
        }
        // An operator who set the knob themselves keeps their value.
        if std::env::var_os("MALLOC_ARENA_MAX").is_none() {
            // SAFETY: mallopt only adjusts allocator parameters and is callable from any thread.
            unsafe {
                libc::mallopt(libc::M_ARENA_MAX, ARENAS);
            }
        }
    }

    pub fn release_free_memory() {
        static TRIMMING: AtomicBool = AtomicBool::new(false);
        if disabled() || TRIMMING.swap(true, Ordering::AcqRel) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("malloc-trim".into())
            .spawn(|| {
                // SAFETY: malloc_trim only returns free pages to the kernel.
                unsafe {
                    libc::malloc_trim(0);
                }
                TRIMMING.store(false, Ordering::Release);
            });
        if spawned.is_err() {
            TRIMMING.store(false, Ordering::Release);
        }
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
mod imp {
    pub fn init() {}
    pub fn release_free_memory() {}
}

/// Call before any worker thread exists.
pub fn init() {
    imp::init();
}

/// Return free heap pages to the OS without waiting for the allocator to decide to. Cheap to call
/// repeatedly: a trim already in flight absorbs the request.
pub fn release_free_memory() {
    imp::release_free_memory();
}
