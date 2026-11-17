//! Start-up/shut-down phase timer for performance work: with TEXRS_PHASES=1 the engine
//! prints, at exit, the time since process start at each marked point. Off by default;
//! then `mark` is one relaxed atomic load.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static STATE: AtomicU8 = AtomicU8::new(0); // 0 unknown, 1 off, 2 on
static MARKS: Mutex<Vec<(&'static str, Instant)>> = Mutex::new(Vec::new());

fn on() -> bool {
    match STATE.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let v = std::env::var_os("TEXRS_PHASES").is_some();
            STATE.store(if v { 2 } else { 1 }, Ordering::Relaxed);
            v
        }
    }
}

pub fn mark(name: &'static str) {
    if on() {
        MARKS.lock().unwrap().push((name, Instant::now()));
    }
}

/// Process creation time, so the report includes loader time before main.
fn process_start() -> Option<Instant> {
    #[repr(C)]
    struct FileTime {
        lo: u32,
        hi: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(h: isize, c: *mut FileTime, e: *mut FileTime, k: *mut FileTime, u: *mut FileTime) -> i32;
        fn GetSystemTimePreciseAsFileTime(t: *mut FileTime);
    }
    unsafe {
        let mut c = FileTime { lo: 0, hi: 0 };
        let mut e = FileTime { lo: 0, hi: 0 };
        let mut k = FileTime { lo: 0, hi: 0 };
        let mut u = FileTime { lo: 0, hi: 0 };
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) == 0 {
            return None;
        }
        let mut now = FileTime { lo: 0, hi: 0 };
        GetSystemTimePreciseAsFileTime(&mut now);
        let t = |f: &FileTime| ((f.hi as u64) << 32) | f.lo as u64;
        let ago = t(&now).saturating_sub(t(&c)); // 100 ns units
        Instant::now().checked_sub(std::time::Duration::from_nanos(ago * 100))
    }
}

pub fn report() {
    if !on() {
        return;
    }
    let marks = MARKS.lock().unwrap();
    let Some(&(_, first)) = marks.first() else { return };
    let start = process_start().unwrap_or(first);
    let mut prev = start;
    let mut out = String::from("PHASES (ms since process creation, +delta)\n");
    for &(name, t) in marks.iter() {
        out += &format!("  {:8.2} +{:7.2}  {}\n", (t - start).as_secs_f64() * 1e3, (t - prev).as_secs_f64() * 1e3, name);
        prev = t;
    }
    eprint!("{out}");
}
