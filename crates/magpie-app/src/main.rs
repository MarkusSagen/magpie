// Removed in the final runtime-wiring task once every module is consumed by main.
#![allow(dead_code)]

slint::include_modules!();

fn main() {
    println!("magpie-app placeholder");
}

mod app_state;
mod config;
mod grouping;
mod image_cache;
mod paste_action;
mod viewmodel;
