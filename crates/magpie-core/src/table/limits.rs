//! "Large file" caps (spec §4): rows/bytes/columns held for interactive
//! viewing. Export always re-reads the original source without these caps —
//! only `table::parse` (interactive load) is capped.

/// Max data rows parsed/held for interactive viewing. A source with more rows
/// stops being read at this count (`Table::truncated` is set), not
/// parsed-then-discarded.
pub const MAX_ROWS: usize = 50_000;

/// Max source bytes considered before parsing starts at all. Larger inputs are
/// rejected with `ParseError::TooLarge` rather than truncated.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

/// Max columns kept. Extra columns (past the first `MAX_COLUMNS`) are dropped
/// with a warning — guards the JSON array-of-objects union-of-keys path against
/// a pathological "every row has unique keys" input.
pub const MAX_COLUMNS: usize = 500;
