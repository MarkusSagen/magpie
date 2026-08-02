//! `magpie-app` library surface — the non-UI, testable pieces of the Magpie
//! desktop app (config, image cache, view-model, grouping, quick-paste, shared
//! state, CLI). The Slint UI + runtime wiring live in the `magpie` binary.

pub mod app_state;
pub mod cli;
pub mod config;
pub mod format_time;
pub mod grouping;
pub mod image_cache;
pub mod merge_view;
pub mod paste_action;
pub mod retention;
pub mod stats_view;
pub mod viewmodel;
