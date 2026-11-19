//! Cryptographically-secure random bytes from the OS CSPRNG, used to seed the
//! pure generators in `magpie_core::random` (passwords, UUIDs, ranges). The core
//! crate stays dependency-free and entropy-in; this is where the bytes come from.
use std::vec::Vec;

/// Fill `n` bytes from the OS CSPRNG. Falls back to a time-seeded, low-quality
/// source only if `getrandom` errors (should never happen on a supported OS) so
/// a random generator never hard-fails the UI.
pub fn random_bytes(n: usize) -> Vec<u8> {
    let mut buf = vec![0u8; n];
    if getrandom::getrandom(&mut buf).is_err() {
        // Extremely unlikely fallback: derive from the clock. Not for the DB key
        // (that path errors out instead) — only so a UI dice-roll degrades rather
        // than panics.
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9e3779b97f4a7c15);
        let mut x = seed ^ 0x9e3779b97f4a7c15;
        for b in buf.iter_mut() {
            // xorshift64
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = (x & 0xff) as u8;
        }
    }
    buf
}

/// A single random `u64` from the OS CSPRNG.
pub fn random_u64() -> u64 {
    let b = random_bytes(8);
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&b);
    u64::from_le_bytes(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_bytes_has_requested_len_and_varies() {
        let a = random_bytes(32);
        let b = random_bytes(32);
        assert_eq!(a.len(), 32);
        assert_eq!(b.len(), 32);
        assert_ne!(
            a, b,
            "two draws should differ with overwhelming probability"
        );
    }
}
