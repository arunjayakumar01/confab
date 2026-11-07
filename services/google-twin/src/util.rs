//! Small shared helpers: Google-style timestamps, ids, a deterministic PRNG.

use chrono::{DateTime, SecondsFormat, Utc};

/// RFC 3339 with microseconds and a `Z`, the way Google prints `createTime`.
pub fn fmt_time(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Micros, true)
}

pub fn now_str() -> String {
    fmt_time(Utc::now())
}

pub fn parse_time(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

const ID_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-";

/// Eleven url-safe characters, the shape of Google's message and thread ids.
pub fn random_id() -> String {
    let bytes = uuid::Uuid::new_v4().into_bytes();
    bytes
        .iter()
        .take(11)
        .map(|b| ID_ALPHABET[(*b as usize) % ID_ALPHABET.len()] as char)
        .collect()
}

/// Opaque token material.
pub fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

/// `users/1001` → `1001`; anything without the prefix is returned unchanged.
pub fn user_id(name: &str) -> &str {
    name.strip_prefix("users/").unwrap_or(name)
}

/// Deterministic xorshift64* generator for the seed. Same sequence every start.
#[derive(Clone, Debug)]
pub struct DetRng(u64);

impl DetRng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn chance(&mut self, p: f64) -> bool {
        self.f64() < p
    }
    pub fn id(&mut self) -> String {
        (0..11)
            .map(|_| ID_ALPHABET[self.below(ID_ALPHABET.len() as u64) as usize] as char)
            .collect()
    }
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Length of `s` in UTF-16 code units, the index space JavaScript slices in.
pub fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic() {
        let mut a = DetRng::new(42);
        let mut b = DetRng::new(42);
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        assert_eq!(a.id().len(), 11);
    }

    #[test]
    fn time_round_trips() {
        let s = now_str();
        assert!(s.ends_with('Z'));
        assert!(parse_time(&s).is_some());
    }
}
