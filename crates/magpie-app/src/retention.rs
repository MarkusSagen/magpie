use crate::config::Config;
use magpie_core::RetentionPolicy;

const DAY_MS: i64 = 86_400_000;
const MB: i64 = 1_048_576;

/// Build a core `RetentionPolicy` from the app config (days -> ms, MB -> bytes).
pub fn policy_from_config(cfg: &Config) -> RetentionPolicy {
    RetentionPolicy {
        max_entries: cfg.max_entries,
        max_age_ms: cfg.max_age_days.map(|d| d * DAY_MS),
        max_image_bytes: cfg.max_image_mb.map(|m| m * MB),
    }
}
