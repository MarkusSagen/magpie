// The debug binary's DWARF unwind tables exceed the 16MB compact-unwind limit (large
// generated Slint code + line-tables debuginfo), so the linker prints a benign
// "__eh_frame section too large" note. It doesn't affect correctness — silence the lint.
#![allow(linker_messages)]

slint::include_modules!();

mod runtime;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code =
        magpie_app::cli::run_command(magpie_app::cli::parse_args(&args), &runtime::data_dir());
    if code >= 0 {
        std::process::exit(code);
    }
    // code == -1: launch the GUI.
    runtime::start();
}
