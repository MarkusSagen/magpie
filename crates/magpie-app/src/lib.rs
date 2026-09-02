//! `magpie-app` library surface — the non-UI, testable pieces of the Magpie
//! desktop app (config, image cache, view-model, grouping, quick-paste, shared
//! state, CLI). The Slint UI + runtime wiring live in the `magpie` binary.

pub mod app_state;
pub mod backlinks;
pub mod cli;
pub mod color_view;
pub mod config;
pub mod diagnostics;
pub mod external_editor;
pub mod favicon;
pub mod format_time;
pub mod grouping;
pub mod image_cache;
pub mod mask_view;
pub mod merge_view;
pub mod notes_view;
pub mod paste_action;
pub mod reminders;
pub mod retention;
pub mod stats_view;
pub mod tasks;
pub mod viewmodel;
