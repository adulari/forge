//! Paces WebKitGTK's animation frames at the monitor's real refresh rate on drivers without
//! `drmWaitVBlank`.
//!
//! WebKit's `DisplayVBlankMonitorDRM` probes `drmWaitVBlank` and, when it fails, falls back to a
//! fixed 60 fps timer. NVIDIA's proprietary DRM driver answers `EOPNOTSUPP` on every CRTC, so a
//! 144 Hz monitor got `requestAnimationFrame` every 16.0 ms. The executable defines and
//! dynamically exports `drmWaitVBlank` (see build.rs), which puts it ahead of libdrm's in the
//! global lookup scope libwebkit's calls resolve through. A call the driver supports passes
//! through untouched; on `EOPNOTSUPP` the wait is emulated with `clock_nanosleep` against the
//! CRTC's mode timings. With this in place the same setup ran at 143 fps, one frame per vblank.

use std::ffi::{c_int, c_long, c_uint, c_ulong, c_void, CStr};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

const DRM_VBLANK_RELATIVE: u32 = 0x1;
const DRM_VBLANK_HIGH_CRTC_MASK: u32 = 0x3e;
const DRM_VBLANK_HIGH_CRTC_SHIFT: u32 = 1;
const DRM_VBLANK_EVENT: u32 = 0x400_0000;
const DRM_VBLANK_SECONDARY: u32 = 0x2000_0000;
const DRM_VBLANK_SIGNAL: u32 = 0x4000_0000;

/// Offsets the emulated counter so that small absolute targets are always far behind it, even
/// right after boot when `CLOCK_MONOTONIC / period` is still small.
const COUNTER_BASE: u32 = 1 << 24;
/// An absolute target further behind the counter than this (about 7 minutes at 144 Hz) was not
/// derived from a sequence we replied with. WebKit's per-frame wait arrives as absolute `1`; taken
/// literally it is always in the past, returns at once and spins the frame loop at ~1000 fps.
const STALE_WINDOW: i32 = 1 << 16;

const NANOS_PER_SEC: i64 = 1_000_000_000;
const FALLBACK_PERIOD_NS: i64 = NANOS_PER_SEC / 60;

#[repr(C)]
#[derive(Clone, Copy)]
struct VBlankRequest {
    kind: c_uint,
    sequence: c_uint,
    signal: c_ulong,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct VBlankReply {
    kind: c_uint,
    sequence: c_uint,
    tval_sec: c_long,
    tval_usec: c_long,
}

#[repr(C)]
pub union DrmVBlank {
    request: VBlankRequest,
    reply: VBlankReply,
}

/// Leading fields of libdrm's `drmModeRes`; only ever read through libdrm's pointer.
#[repr(C)]
#[allow(dead_code)]
struct ModeResPrefix {
    count_fbs: c_int,
    fbs: *mut u32,
    count_crtcs: c_int,
    crtcs: *mut u32,
}

/// Leading fields of libdrm's `drmModeCrtc` up to `mode.vtotal`.
#[repr(C)]
#[allow(dead_code)]
struct ModeCrtcPrefix {
    crtc_id: u32,
    buffer_id: u32,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    mode_valid: c_int,
    clock: u32,
    hdisplay: u16,
    hsync_start: u16,
    hsync_end: u16,
    htotal: u16,
    hskew: u16,
    vdisplay: u16,
    vsync_start: u16,
    vsync_end: u16,
    vtotal: u16,
}

type WaitFn = unsafe extern "C" fn(c_int, *mut DrmVBlank) -> c_int;
type GetResourcesFn = unsafe extern "C" fn(c_int) -> *mut ModeResPrefix;
type FreeResourcesFn = unsafe extern "C" fn(*mut ModeResPrefix);
type GetCrtcFn = unsafe extern "C" fn(c_int, u32) -> *mut ModeCrtcPrefix;
type FreeCrtcFn = unsafe extern "C" fn(*mut ModeCrtcPrefix);

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Emulated {
    pub sequence: u32,
    pub at_ns: i64,
}

/// The vblank a request waits for, against a counter that ticks once per `period_ns`.
pub(crate) fn emulate(now_ns: i64, period_ns: i64, request_type: u32, requested: u32) -> Emulated {
    let index = now_ns / period_ns;
    let current = (index as u32).wrapping_add(COUNTER_BASE);
    let stale = (current.wrapping_sub(requested) as i32) > STALE_WINDOW;
    let relative = request_type & DRM_VBLANK_RELATIVE != 0 || stale;
    let mut target = if relative {
        current.wrapping_add(requested)
    } else {
        requested
    };
    if (target.wrapping_sub(current) as i32) <= 0 {
        target = current;
    }
    let ahead = i64::from(target.wrapping_sub(current) as i32);
    Emulated {
        sequence: target,
        at_ns: (index + ahead) * period_ns,
    }
}

pub(crate) fn refresh_period_ns(clock_khz: u32, htotal: u16, vtotal: u16) -> Option<i64> {
    if htotal == 0 || vtotal == 0 {
        return None;
    }
    let hz = f64::from(clock_khz) * 1000.0 / (f64::from(htotal) * f64::from(vtotal));
    (hz > 20.0 && hz < 1000.0).then(|| (1e9 / hz).round() as i64)
}

pub(crate) fn crtc_index(request_type: u32) -> u32 {
    if request_type & DRM_VBLANK_SECONDARY != 0 {
        return 1;
    }
    (request_type & DRM_VBLANK_HIGH_CRTC_MASK) >> DRM_VBLANK_HIGH_CRTC_SHIFT
}

fn next_symbol<T: Copy>(name: &CStr) -> Option<T> {
    let pointer = unsafe { libc::dlsym(libc::RTLD_NEXT, name.as_ptr()) };
    if pointer.is_null() {
        return None;
    }
    Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&pointer) })
}

fn real_wait() -> Option<WaitFn> {
    static REAL: OnceLock<Option<WaitFn>> = OnceLock::new();
    *REAL.get_or_init(|| next_symbol(c"drmWaitVBlank"))
}

struct ModeApi {
    get_resources: GetResourcesFn,
    free_resources: FreeResourcesFn,
    get_crtc: GetCrtcFn,
    free_crtc: FreeCrtcFn,
}

fn mode_api() -> Option<&'static ModeApi> {
    static API: OnceLock<Option<ModeApi>> = OnceLock::new();
    API.get_or_init(|| {
        Some(ModeApi {
            get_resources: next_symbol(c"drmModeGetResources")?,
            free_resources: next_symbol(c"drmModeFreeResources")?,
            get_crtc: next_symbol(c"drmModeGetCrtc")?,
            free_crtc: next_symbol(c"drmModeFreeCrtc")?,
        })
    })
    .as_ref()
}

fn crtc_period_ns(fd: c_int, index: u32) -> i64 {
    let Some(api) = mode_api() else {
        return FALLBACK_PERIOD_NS;
    };
    let mut period = FALLBACK_PERIOD_NS;
    unsafe {
        let resources = (api.get_resources)(fd);
        if resources.is_null() {
            return period;
        }
        let res = &*resources;
        if (index as c_int) < res.count_crtcs && !res.crtcs.is_null() {
            let crtc = (api.get_crtc)(fd, *res.crtcs.add(index as usize));
            if !crtc.is_null() {
                let mode = &*crtc;
                if mode.mode_valid != 0 {
                    if let Some(found) = refresh_period_ns(mode.clock, mode.htotal, mode.vtotal) {
                        period = found;
                    }
                }
                (api.free_crtc)(crtc);
            }
        }
        (api.free_resources)(resources);
    }
    period
}

// `time_t` and `c_long` are only `i64` on 64-bit targets.
#[allow(clippy::useless_conversion)]
fn monotonic_ns() -> i64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut now) };
    i64::from(now.tv_sec) * NANOS_PER_SEC + i64::from(now.tv_nsec)
}

fn sleep_until(at_ns: i64) {
    let deadline = libc::timespec {
        tv_sec: (at_ns / NANOS_PER_SEC) as libc::time_t,
        tv_nsec: (at_ns % NANOS_PER_SEC) as c_long,
    };
    while unsafe {
        libc::clock_nanosleep(
            libc::CLOCK_MONOTONIC,
            libc::TIMER_ABSTIME,
            &deadline,
            std::ptr::null_mut(),
        )
    } == libc::EINTR
    {}
}

fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("FORGE_VBLANK_DEBUG").is_some_and(|v| v != "0"))
}

fn errno() -> c_int {
    unsafe { *libc::__errno_location() }
}

fn set_errno(value: c_int) {
    unsafe { *libc::__errno_location() = value };
}

/// # Safety
/// Same contract as libdrm's `drmWaitVBlank`: `vbl` points to a valid `drmVBlank`.
#[no_mangle]
pub unsafe extern "C" fn drmWaitVBlank(fd: c_int, vbl: *mut DrmVBlank) -> c_int {
    let saved = errno();
    let request = unsafe { (*vbl).request };
    let result = match real_wait() {
        Some(real) => unsafe { real(fd, vbl) },
        None => {
            set_errno(libc::ENOSYS);
            -1
        }
    };
    let failure = errno();
    if debug_enabled() {
        static LOGGED: AtomicU32 = AtomicU32::new(0);
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 3 {
            eprintln!(
                "forge-vblank: drmWaitVBlank type={:#x} seq={} ret={result} errno={failure}",
                request.kind, request.sequence
            );
        }
        set_errno(failure);
    }
    if result == 0 || failure != libc::EOPNOTSUPP {
        return result;
    }
    if request.kind & (DRM_VBLANK_EVENT | DRM_VBLANK_SIGNAL) != 0 {
        return result;
    }
    let period = crtc_period_ns(fd, crtc_index(request.kind));
    let wait = emulate(monotonic_ns(), period, request.kind, request.sequence);
    sleep_until(wait.at_ns);
    unsafe {
        (*vbl).reply = VBlankReply {
            kind: request.kind,
            sequence: wait.sequence,
            tval_sec: (wait.at_ns / NANOS_PER_SEC) as c_long,
            tval_usec: ((wait.at_ns % NANOS_PER_SEC) / 1000) as c_long,
        };
    }
    set_errno(saved);
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    const PERIOD: i64 = 6_944_444;
    const NOW: i64 = 3_600 * NANOS_PER_SEC + 1_234_567;

    fn current() -> u32 {
        ((NOW / PERIOD) as u32).wrapping_add(COUNTER_BASE)
    }

    fn next_vblank_ns() -> i64 {
        (NOW / PERIOD + 1) * PERIOD
    }

    #[test]
    fn relative_one_waits_for_the_next_vblank() {
        let wait = emulate(NOW, PERIOD, DRM_VBLANK_RELATIVE, 1);
        assert_eq!(wait.sequence, current() + 1);
        assert_eq!(wait.at_ns, next_vblank_ns());
        assert!(wait.at_ns > NOW && wait.at_ns - NOW <= PERIOD);
    }

    #[test]
    fn relative_n_waits_n_periods() {
        let wait = emulate(NOW, PERIOD, DRM_VBLANK_RELATIVE, 3);
        assert_eq!(wait.sequence, current() + 3);
        assert_eq!(wait.at_ns, (NOW / PERIOD + 3) * PERIOD);
    }

    #[test]
    fn absolute_target_ahead_is_honoured() {
        let wait = emulate(NOW, PERIOD, 0, current() + 2);
        assert_eq!(wait.sequence, current() + 2);
        assert_eq!(wait.at_ns, (NOW / PERIOD + 2) * PERIOD);
    }

    #[test]
    fn absolute_target_just_missed_returns_now() {
        let wait = emulate(NOW, PERIOD, 0, current() - 1);
        assert_eq!(wait.sequence, current());
        assert!(wait.at_ns <= NOW);
    }

    #[test]
    fn absolute_one_far_behind_is_treated_as_relative() {
        let wait = emulate(NOW, PERIOD, 0, 1);
        assert_eq!(wait.sequence, current() + 1);
        assert_eq!(wait.at_ns, next_vblank_ns());
    }

    #[test]
    fn absolute_one_waits_even_right_after_boot() {
        let wait = emulate(2 * NANOS_PER_SEC, PERIOD, 0, 1);
        assert_eq!(wait.at_ns, (2 * NANOS_PER_SEC / PERIOD + 1) * PERIOD);
    }

    #[test]
    fn high_crtc_absolute_one_is_treated_as_relative() {
        let wait = emulate(NOW, PERIOD, 0x4, 1);
        assert_eq!(wait.sequence, current() + 1);
    }

    #[test]
    fn probe_with_sequence_zero_returns_immediately() {
        for kind in [0, 0x4, DRM_VBLANK_RELATIVE] {
            let wait = emulate(NOW, PERIOD, kind, 0);
            assert_eq!(wait.sequence, current());
            assert!(wait.at_ns <= NOW, "type {kind:#x} must not sleep");
        }
    }

    #[test]
    fn counter_wraps_without_stalling() {
        let now = (u32::MAX - COUNTER_BASE) as i64 * PERIOD + PERIOD / 2;
        let wait = emulate(now, PERIOD, DRM_VBLANK_RELATIVE, 1);
        assert_eq!(wait.at_ns, (now / PERIOD + 1) * PERIOD);
        assert_eq!(wait.sequence, 0);
    }

    #[test]
    fn period_comes_from_mode_timings() {
        // 2560x1440@144: 586.5 MHz pixel clock, 2720 x 1497 total.
        let period = refresh_period_ns(586_500, 2720, 1497).unwrap();
        assert!((6_900_000..7_000_000).contains(&period), "{period}");
        assert_eq!(refresh_period_ns(148_500, 2200, 1125), Some(16_666_667));
        assert_eq!(refresh_period_ns(148_500, 0, 1125), None);
        assert_eq!(refresh_period_ns(1, 2200, 1125), None);
    }

    #[test]
    fn crtc_index_reads_secondary_and_high_crtc_bits() {
        assert_eq!(crtc_index(0), 0);
        assert_eq!(crtc_index(0x4), 2);
        assert_eq!(crtc_index(DRM_VBLANK_SECONDARY), 1);
        assert_eq!(crtc_index(DRM_VBLANK_RELATIVE | (5 << 1)), 5);
    }
}
