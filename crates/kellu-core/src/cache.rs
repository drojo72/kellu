//! Exact-match answer cache: the cheapest way to avoid spending a token.
//!
//! A hover or a completion asked twice over the same text costs nothing the second time. The key
//! is a SHA-256 of everything that shapes the request (task, route, sampling settings and the
//! already redacted messages), so the prompt itself is never stored: only the key and the cleaned
//! answer. Failures and empty answers are never cached. This version lives in memory for the life
//! of the process; nothing is written to disk.

use std::collections::{HashMap, VecDeque};

use kellu_security::hash::sha256_hex;

use crate::router::Clock;

/// Limits of an [`AnswerCache`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CacheSettings {
    /// Whether answers are cached at all.
    pub enabled: bool,
    /// Seconds an answer stays valid.
    pub ttl_s: f64,
    /// Most entries kept; the oldest go first.
    pub max_entries: usize,
    /// Most bytes of answers kept in total.
    pub max_bytes: usize,
    /// Longest single answer worth keeping, in bytes.
    pub max_entry_bytes: usize,
}

impl Default for CacheSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            ttl_s: 3600.0,
            max_entries: 256,
            max_bytes: 1024 * 1024,
            max_entry_bytes: 16 * 1024,
        }
    }
}

struct Entry {
    answer: String,
    stored_at: f64,
}

/// A bounded cache of answers by request key.
pub struct AnswerCache {
    settings: CacheSettings,
    clock: Clock,
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    bytes: usize,
}

impl std::fmt::Debug for AnswerCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnswerCache")
            .field("entries", &self.entries.len())
            .field("bytes", &self.bytes)
            .finish_non_exhaustive()
    }
}

/// The cache key for a request: a digest of its parts. Each part is length-prefixed so that
/// `("ab", "c")` and `("a", "bc")` cannot collide.
#[must_use]
pub fn key_for(parts: &[&str]) -> String {
    let mut material = String::new();
    for part in parts {
        material.push_str(&part.len().to_string());
        material.push(':');
        material.push_str(part);
        material.push('\n');
    }
    sha256_hex(material.as_bytes())
}

impl AnswerCache {
    /// An empty cache.
    #[must_use]
    pub fn new(settings: CacheSettings, clock: Clock) -> Self {
        Self {
            settings,
            clock,
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
        }
    }

    /// The cached answer for `key`, if there is a fresh one. An expired entry is dropped.
    pub fn get(&mut self, key: &str) -> Option<String> {
        if !self.settings.enabled {
            return None;
        }
        let now = (self.clock)();
        let fresh = self
            .entries
            .get(key)
            .map(|entry| now - entry.stored_at <= self.settings.ttl_s)?;
        if fresh {
            return self.entries.get(key).map(|entry| entry.answer.clone());
        }
        self.remove(key);
        None
    }

    /// Keep `answer` under `key`. Empty and oversized answers are not kept.
    pub fn put(&mut self, key: String, answer: &str) {
        let size = answer.len();
        if !self.settings.enabled
            || answer.trim().is_empty()
            || size > self.settings.max_entry_bytes
            || size > self.settings.max_bytes
        {
            return;
        }
        self.remove(&key);
        while !self.order.is_empty()
            && (self.entries.len() >= self.settings.max_entries
                || self.bytes.saturating_add(size) > self.settings.max_bytes)
        {
            if let Some(entry) = self
                .order
                .pop_front()
                .and_then(|oldest| self.entries.remove(&oldest))
            {
                self.bytes = self.bytes.saturating_sub(entry.answer.len());
            }
        }
        if self.settings.max_entries == 0 {
            return;
        }
        self.bytes = self.bytes.saturating_add(size);
        self.order.push_back(key.clone());
        self.entries.insert(
            key,
            Entry {
                answer: answer.to_owned(),
                stored_at: (self.clock)(),
            },
        );
    }

    /// Forget everything.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.bytes = 0;
    }

    /// Entries held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn remove(&mut self, key: &str) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes = self.bytes.saturating_sub(entry.answer.len());
            self.order.retain(|k| k != key);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    /// A clock the test moves by hand, in whole seconds.
    fn manual() -> (Clock, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(0));
        let reader = Arc::clone(&now);
        let clock: Clock = Arc::new(move || {
            #[allow(clippy::cast_precision_loss)]
            let seconds = reader.load(Ordering::SeqCst) as f64;
            seconds
        });
        (clock, now)
    }

    fn cache(settings: CacheSettings) -> (AnswerCache, Arc<AtomicU64>) {
        let (clock, now) = manual();
        (AnswerCache::new(settings, clock), now)
    }

    #[test]
    fn keys_depend_on_every_part_and_on_where_the_parts_split() {
        assert_eq!(key_for(&["a", "b"]), key_for(&["a", "b"]));
        assert_ne!(key_for(&["a", "b"]), key_for(&["a", "c"]));
        assert_ne!(key_for(&["ab", "c"]), key_for(&["a", "bc"]));
        assert_ne!(key_for(&["a", ""]), key_for(&["a"]));
        assert_eq!(key_for(&["x"]).len(), 64);
    }

    #[test]
    fn a_stored_answer_comes_back_until_it_expires() {
        let (mut cache, now) = cache(CacheSettings {
            ttl_s: 10.0,
            ..CacheSettings::default()
        });
        cache.put("k".into(), "answer");
        assert_eq!(cache.get("k").as_deref(), Some("answer"));
        now.store(10, Ordering::SeqCst);
        assert_eq!(
            cache.get("k").as_deref(),
            Some("answer"),
            "still fresh at the limit"
        );
        now.store(11, Ordering::SeqCst);
        assert_eq!(cache.get("k"), None);
        assert!(cache.is_empty(), "an expired entry is dropped");
    }

    #[test]
    fn empty_and_oversized_answers_are_not_kept() {
        let (mut cache, _) = cache(CacheSettings {
            max_entry_bytes: 4,
            ..CacheSettings::default()
        });
        cache.put("empty".into(), "");
        cache.put("blank".into(), " \n\t");
        cache.put("big".into(), "12345");
        assert!(cache.is_empty());
        cache.put("ok".into(), "1234");
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn the_oldest_entries_go_first_when_full() {
        let (mut cache, _) = cache(CacheSettings {
            max_entries: 2,
            ..CacheSettings::default()
        });
        cache.put("a".into(), "1");
        cache.put("b".into(), "2");
        cache.put("c".into(), "3");
        assert_eq!(cache.get("a"), None);
        assert_eq!(cache.get("b").as_deref(), Some("2"));
        assert_eq!(cache.get("c").as_deref(), Some("3"));
    }

    #[test]
    fn the_byte_cap_holds_and_replacing_a_key_does_not_leak_bytes() {
        let (mut cache, _) = cache(CacheSettings {
            max_bytes: 10,
            ..CacheSettings::default()
        });
        cache.put("a".into(), "123456");
        cache.put("b".into(), "123456");
        assert_eq!(cache.get("a"), None, "making room evicts the oldest");
        assert_eq!(cache.get("b").as_deref(), Some("123456"));
        for _ in 0..100 {
            cache.put("b".into(), "123456");
        }
        assert_eq!(cache.len(), 1);
        cache.put("c".into(), "1234");
        assert_eq!(cache.len(), 2, "10 bytes still fit: no leak from replacing");
    }

    #[test]
    fn a_disabled_cache_keeps_and_returns_nothing() {
        let (mut cache, _) = cache(CacheSettings {
            enabled: false,
            ..CacheSettings::default()
        });
        cache.put("a".into(), "1");
        assert!(cache.is_empty());
        assert_eq!(cache.get("a"), None);
    }

    #[test]
    fn zero_capacity_keeps_nothing_and_does_not_loop() {
        let (mut cache, _) = cache(CacheSettings {
            max_entries: 0,
            ..CacheSettings::default()
        });
        cache.put("a".into(), "1");
        assert!(cache.is_empty());
    }

    #[test]
    fn clear_forgets_everything() {
        let (mut cache, _) = cache(CacheSettings::default());
        cache.put("a".into(), "1");
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(cache.get("a"), None);
    }
}
