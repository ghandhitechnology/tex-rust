//! In-process sampling profiler for performance work (Windows x64).
//!
//! With TEXRS_SAMPLE=<file>, a helper thread suspends the engine thread about every
//! 100 µs, records its instruction pointer and the first return addresses on its stack,
//! and at exit writes them, with the list of loaded modules, to <file>.
//! tools/sampler.py --inproc resolves and aggregates them. Without the variable nothing
//! happens. The helper never allocates while the engine thread is suspended (that thread
//! might hold the heap lock).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

const STACK_WORDS: usize = 48;
const MAX_SAMPLES: usize = 2_000_000;

struct Sample {
    rip: u64,
    stack: [u64; STACK_WORDS],
}

static STOP: AtomicBool = AtomicBool::new(false);
static RESULT: Mutex<Option<(String, std::thread::JoinHandle<Vec<Sample>>)>> = Mutex::new(None);

#[allow(non_snake_case)]
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> isize;
    fn GetCurrentThread() -> isize;
    fn DuplicateHandle(sp: isize, sh: isize, tp: isize, th: *mut isize, access: u32, inherit: i32, opts: u32) -> i32;
    fn SuspendThread(h: isize) -> u32;
    fn ResumeThread(h: isize) -> u32;
    fn GetThreadContext(h: isize, ctx: *mut u8) -> i32;
    fn QueryPerformanceCounter(t: *mut i64) -> i32;
    fn QueryPerformanceFrequency(t: *mut i64) -> i32;
    fn K32EnumProcessModules(p: isize, mods: *mut isize, cb: u32, need: *mut u32) -> i32;
    fn K32GetModuleInformation(p: isize, m: isize, info: *mut [usize; 3], cb: u32) -> i32;
    fn K32GetModuleFileNameExW(p: isize, m: isize, name: *mut u16, n: u32) -> u32;
    fn VirtualQuery(addr: *const u8, info: *mut [usize; 6], len: usize) -> usize;
}

/// Start sampling the calling thread if TEXRS_SAMPLE is set.
pub fn start_for_current_thread() {
    let Some(path) = std::env::var_os("TEXRS_SAMPLE") else { return };
    let mut h: isize = 0;
    unsafe {
        if DuplicateHandle(GetCurrentProcess(), GetCurrentThread(), GetCurrentProcess(), &mut h, 0, 0, 2) == 0 {
            return;
        }
    }
    let join = std::thread::spawn(move || run(h));
    *RESULT.lock().unwrap() = Some((path.to_string_lossy().into_owned(), join));
}

fn run(h: isize) -> Vec<Sample> {
    let mut samples: Vec<Sample> = Vec::with_capacity(MAX_SAMPLES);
    // CONTEXT (x64) is 1232 bytes and must be 16-byte aligned
    let mut buf = vec![0u8; 1232 + 16];
    let off = buf.as_ptr().align_offset(16);
    let mut freq = 0i64;
    unsafe { QueryPerformanceFrequency(&mut freq) };
    let interval = (freq / 10_000).max(1); // 100 µs
    let mut next = 0i64;
    while !STOP.load(Ordering::Relaxed) && samples.len() < MAX_SAMPLES {
        let mut now = 0i64;
        unsafe { QueryPerformanceCounter(&mut now) };
        if now < next {
            std::hint::spin_loop();
            continue;
        }
        next = now + interval;
        let ctx = unsafe { buf.as_mut_ptr().add(off) };
        unsafe {
            std::ptr::write_unaligned(ctx.add(0x30) as *mut u32, 0x0010_0001); // CONTEXT_CONTROL
            if SuspendThread(h) == u32::MAX {
                break;
            }
            let ok = GetThreadContext(h, ctx) != 0;
            let mut s = Sample { rip: 0, stack: [0; STACK_WORDS] };
            if ok {
                s.rip = std::ptr::read_unaligned(ctx.add(0xF8) as *const u64);
                let rsp = std::ptr::read_unaligned(ctx.add(0x98) as *const u64) as *const u64;
                // copy only stack words inside committed memory (the stack's top region)
                let mut mbi = [0usize; 6];
                if VirtualQuery(rsp as *const u8, &mut mbi, std::mem::size_of_val(&mbi)) != 0 {
                    let end = mbi[0] + mbi[3]; // BaseAddress + RegionSize
                    let n = ((end.saturating_sub(rsp as usize)) / 8).min(STACK_WORDS);
                    std::ptr::copy_nonoverlapping(rsp, s.stack.as_mut_ptr(), n);
                }
            }
            ResumeThread(h);
            if ok {
                samples.push(s);
            }
        }
    }
    samples
}

/// Stop sampling and write the samples (called at exit).
pub fn finish() {
    let Some((path, join)) = RESULT.lock().unwrap().take() else { return };
    STOP.store(true, Ordering::Relaxed);
    let samples = join.join().unwrap_or_default();
    let mut out = String::new();
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    unsafe {
        let p = GetCurrentProcess();
        let mut mods = [0isize; 512];
        let mut need = 0u32;
        if K32EnumProcessModules(p, mods.as_mut_ptr(), std::mem::size_of_val(&mods) as u32, &mut need) != 0 {
            for &m in &mods[..(need as usize / 8).min(512)] {
                let mut info = [0usize; 3];
                let mut name = [0u16; 520];
                if K32GetModuleInformation(p, m, &mut info, std::mem::size_of_val(&info) as u32) != 0 {
                    let n = K32GetModuleFileNameExW(p, m, name.as_mut_ptr(), 520) as usize;
                    // MODULEINFO.SizeOfImage is a DWORD followed by padding
                    let size = info[1] & 0xFFFF_FFFF;
                    out += &format!("M {:x} {:x} {}\n", info[0], size, String::from_utf16_lossy(&name[..n]));
                    ranges.push((info[0] as u64, info[0] as u64 + size as u64));
                }
            }
        }
    }
    for s in &samples {
        out += &format!("S {:x}", s.rip);
        for w in &s.stack {
            if ranges.iter().any(|&(a, b)| a <= *w && *w < b) {
                out += &format!(" {:x}", w);
            }
        }
        out.push('\n');
    }
    let _ = std::fs::write(path, out);
}
