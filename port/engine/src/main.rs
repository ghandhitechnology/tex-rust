//! pdfTeX 1.40.29 (TeX Live 2026) in Rust.
//!
//! `gen/` is translated from TeX Live's pdftex.web + change files by tools/w2rs;
//! `rt/` is the hand-ported C side (web2c runtime, kpathsea, pdfTeX's C library).

mod gen;
mod rt;

use gen::globals::Globals;

#[global_allocator]
static GLOBAL: rt::alloc::TexAlloc = rt::alloc::TexAlloc::new();

fn main() {
    rt::phase::mark("main");
    let args: Vec<String> = std::env::args().collect();
    // A panic means a bug or a not-yet-ported path: report it and exit with a failure code
    // (never abort, which can leave a Windows error dialog waiting).
    std::panic::set_hook(Box::new(|info| {
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let bt = std::backtrace::Backtrace::capture();
        let _ = writeln!(std::io::stderr(), "\r\n!pdftex-rs internal error: {}\r\n{}", info, bt);
        std::process::exit(3);
    }));
    // TeX recursion (expand, macro_call, ...) can be deep; give it a large stack.
    let child = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            rt::phase::mark("engine thread");
            rt::sampler::start_for_current_thread();
            let mut g: Box<Globals> = Box::default();
            rt::phase::mark("Globals::default");
            g.alloc_static_arrays();
            rt::phase::mark("alloc_static_arrays");
            g.run_main(args)
        })
        .expect("cannot start engine thread");
    let _ = child.join();
    std::process::exit(3);
}
