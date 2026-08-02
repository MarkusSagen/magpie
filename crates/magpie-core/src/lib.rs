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

pub mod detect;
pub mod metrics;
pub mod model;
pub mod retention;
pub mod search;
pub mod slots;
pub mod stats;
pub mod store;

pub use detect::Kind;
pub use model::{AppInfo, CaptureEvent, Content, Entry};
pub use retention::{Removed, RetentionPolicy};
pub use search::{default_query, SearchMode, SearchQuery, Sort, TimeRange};
pub use stats::{AppCount, DayBucket, KindCount, MostCopied, Stats, StatsRange, Totals};
pub use store::{open, open_in_memory, ImageStore, Ingested, Store};
