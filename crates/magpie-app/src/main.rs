// v1 wires a subset of a deliberately complete data model + helpers: section
// grouping, thumbnails, config save, and the full filter/sort space are ready
// for the Phase-1 UI but not all rendered by the minimal launcher yet.
#![allow(dead_code)]

slint::include_modules!();

mod app_state;
mod cli;
mod config;
mod grouping;
mod image_cache;
mod paste_action;
mod runtime;
mod viewmodel;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = cli::run_command(cli::parse_args(&args));
    if code >= 0 {
        std::process::exit(code);
    }
    // code == -1: launch the GUI.
    runtime::start();
}
