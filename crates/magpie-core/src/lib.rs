//! `magpie-core` — storage, content-type detection, and search for the Magpie
//! clipboard manager. Platform- and UI-agnostic; time is injected as
//! `copied_at_ms`, and image bytes are written through the `ImageStore` trait.
//!
//! ```
//! use magpie_core::{open_in_memory, CaptureEvent, Content, default_query, ImageStore};
//! struct NoImages;
//! impl ImageStore for NoImages {
//!     fn put(&self, h: &str, _b: &[u8]) -> std::io::Result<String> { Ok(h.into()) }
//! }
//! let s = open_in_memory().unwrap();
//! s.ingest(&CaptureEvent { content: Content::Text("hi".into()), source_app: None, copied_at_ms: 1 }, &NoImages).unwrap();
//! assert_eq!(s.search(&default_query()).unwrap().len(), 1);
//! ```

pub mod bookmarks;
pub mod delete;
pub mod detect;
pub mod edit;
pub mod export;
pub mod mask_support;
pub mod merge;
pub mod metrics;
pub mod model;
pub mod notes;
pub mod retention;
pub mod search;
pub mod slots;
pub mod stats;
pub mod store;
pub mod tags;
pub mod vault;

pub use bookmarks::{read_firefox_bookmarks, Bookmark};
pub use detect::Kind;
pub use export::{backup_db, export_clipboard_jsonl, export_markdown};
pub use model::{AppInfo, CaptureEvent, Content, Entry};
pub use notes::Note;
pub use retention::{Removed, RetentionPolicy};
pub use search::{default_query, SearchMode, SearchQuery, Sort, TimeRange};
pub use stats::{AppCount, DayBucket, KindCount, MostCopied, Stats, StatsRange, Totals};
pub use store::{open, open_in_memory, ImageStore, Ingested, Store};
pub use vault::{resolve_action, sync_vault, SyncReport, VaultAction};
