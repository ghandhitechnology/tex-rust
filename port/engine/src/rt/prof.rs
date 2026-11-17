//! Opt-in instrumenting profiler (`cargo build --release --features prof`): every
//! translated Pascal routine records call count, inclusive and self time (rdtsc ticks).
//! The table is written to `texrs-prof.txt` in the working directory at exit.

#![allow(static_mut_refs)]

const MAX: usize = 1024;
static mut CALLS: [u64; MAX] = [0; MAX];
static mut INCL: [u64; MAX] = [0; MAX];
static mut SELF_T: [u64; MAX] = [0; MAX];
static mut STACK: Vec<(usize, u64, u64)> = Vec::new(); // (id, start, child ticks)
static mut ACTIVE: [u32; MAX] = [0; MAX];

#[inline(always)]
fn now() -> u64 {
    unsafe { core::arch::x86_64::_rdtsc() }
}

pub struct Guard;

#[inline(always)]
pub fn enter(id: usize) -> Guard {
    unsafe {
        CALLS[id] += 1;
        ACTIVE[id] += 1;
        STACK.push((id, now(), 0));
    }
    Guard
}

impl Drop for Guard {
    #[inline(always)]
    fn drop(&mut self) {
        unsafe {
            let (id, start, child) = STACK.pop().unwrap();
            let el = now() - start;
            ACTIVE[id] -= 1;
            if ACTIVE[id] == 0 {
                INCL[id] += el; // count recursion once
            }
            SELF_T[id] += el - child;
            if let Some(top) = STACK.last_mut() {
                top.2 += el;
            }
        }
    }
}

pub fn report(names: &[&str]) {
    unsafe {
        let total: u64 = SELF_T.iter().sum();
        let mut idx: Vec<usize> = (0..names.len()).filter(|&i| CALLS[i] > 0).collect();
        idx.sort_by_key(|&i| std::cmp::Reverse(SELF_T[i]));
        let mut out = format!("{:<28} {:>12} {:>8} {:>8}\n", "routine", "calls", "self%", "incl%");
        for i in idx.iter().take(80) {
            out.push_str(&format!(
                "{:<28} {:>12} {:>7.2}% {:>7.2}%\n",
                names[*i],
                CALLS[*i],
                SELF_T[*i] as f64 * 100.0 / total.max(1) as f64,
                INCL[*i] as f64 * 100.0 / total.max(1) as f64
            ));
        }
        let _ = std::fs::write("texrs-prof.txt", out);
    }
}
