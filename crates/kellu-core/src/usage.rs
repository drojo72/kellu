//! Token counting: what a request was estimated to cost, what the service said it cost, and the
//! ratio between the two.
//!
//! The service's own count is the one that is recorded and shown. The estimate (see `plan`) only
//! plans, so it is compared with the reported prompt size of every answer and the ratio, kept per
//! provider and model, scales the next estimate: a model that counts Spanish or code differently
//! from the rule of thumb stops being planned with the wrong number.
//!
//! This is a session counter in memory. It holds numbers only: never a prompt, an answer or a key.
//! The persistent ledger with costs and caps is a separate step.

use std::collections::BTreeMap;

/// The scale that changes nothing, in thousandths.
pub const NEUTRAL_MILLI: u32 = 1000;
/// Answers with a reported prompt size needed before the ratio is trusted.
const MIN_SAMPLES: u64 = 5;
/// Bounds of the ratio: a wild one is a bug or a cheating service, not a calibration.
const MIN_MILLI: u64 = 500;
const MAX_MILLI: u64 = 3000;
/// Once a model has this many samples the older half of the weight is dropped, so a long session
/// follows a model that changes under the same name.
const DECAY_AT: u64 = 200;

/// What one answered request tells the counter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample<'a> {
    /// Provider name from the configuration.
    pub provider: &'a str,
    /// The model configured for that provider (not the one a router service picked behind it).
    pub model: &'a str,
    /// What the planner estimated for the prompt.
    pub estimated_prompt: u32,
    /// Prompt tokens as the service reported them.
    pub prompt_tokens: Option<u64>,
    /// Answer tokens as the service reported them.
    pub completion_tokens: Option<u64>,
}

/// Counts, as the services reported them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Requests that reached a model and were answered.
    pub requests: u64,
    /// Of those, the ones whose service reported no token counts at all.
    pub unreported: u64,
    /// Prompt tokens reported.
    pub prompt_tokens: u64,
    /// Answer tokens reported.
    pub completion_tokens: u64,
}

impl Totals {
    fn add(&mut self, prompt: Option<u64>, completion: Option<u64>) {
        self.requests = self.requests.saturating_add(1);
        if prompt.is_none() && completion.is_none() {
            self.unreported = self.unreported.saturating_add(1);
        }
        self.prompt_tokens = self.prompt_tokens.saturating_add(prompt.unwrap_or(0));
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(completion.unwrap_or(0));
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct Ratio {
    samples: u64,
    estimated: u64,
    reported: u64,
}

impl Ratio {
    fn add(&mut self, estimated: u32, reported: u64) {
        self.samples += 1;
        self.estimated = self.estimated.saturating_add(u64::from(estimated));
        self.reported = self.reported.saturating_add(reported);
        if self.samples >= DECAY_AT {
            self.samples /= 2;
            self.estimated /= 2;
            self.reported /= 2;
        }
    }

    fn milli(&self) -> u32 {
        if self.samples < MIN_SAMPLES || self.estimated == 0 {
            return NEUTRAL_MILLI;
        }
        let milli =
            (self.reported.saturating_mul(1000) / self.estimated).clamp(MIN_MILLI, MAX_MILLI);
        u32::try_from(milli).unwrap_or(NEUTRAL_MILLI)
    }
}

/// A copy of the counters, for showing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageSnapshot {
    /// Everything that reached a model.
    pub total: Totals,
    /// Requests answered from the cache instead.
    pub cache_hits: u64,
    /// Estimated prompt tokens those cached answers did not send.
    pub cache_saved_tokens: u64,
    /// Per `provider/model`, sorted by name.
    pub models: Vec<(String, Totals)>,
}

impl UsageSnapshot {
    /// Whether nothing happened yet.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.total.requests == 0 && self.cache_hits == 0
    }

    /// The lines to show a person, as the services reported the numbers.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        if self.is_empty() {
            return vec!["no model requests yet".to_owned()];
        }
        let total = &self.total;
        let mut lines = vec![format!(
            "model requests: {} ({} prompt + {} answer tokens, as the services reported them)",
            total.requests, total.prompt_tokens, total.completion_tokens
        )];
        if total.unreported > 0 {
            lines.push(format!(
                "{} request(s) came back without a token count from the service",
                total.unreported
            ));
        }
        if self.cache_hits > 0 {
            lines.push(format!(
                "answered from the cache: {} (about {} prompt tokens not sent)",
                self.cache_hits, self.cache_saved_tokens
            ));
        }
        for (name, totals) in &self.models {
            lines.push(format!(
                "  {name}: {} request(s), {} prompt + {} answer",
                totals.requests, totals.prompt_tokens, totals.completion_tokens
            ));
        }
        lines
    }
}

/// The counters of one session.
#[derive(Debug, Default)]
pub struct UsageTracker {
    total: Totals,
    models: BTreeMap<String, (Totals, Ratio)>,
    cache_hits: u64,
    cache_saved_tokens: u64,
}

fn key(provider: &str, model: &str) -> String {
    format!("{provider}/{model}")
}

impl UsageTracker {
    /// An empty counter.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one answered request.
    pub fn record(&mut self, sample: Sample<'_>) {
        self.total
            .add(sample.prompt_tokens, sample.completion_tokens);
        let entry = self
            .models
            .entry(key(sample.provider, sample.model))
            .or_default();
        entry.0.add(sample.prompt_tokens, sample.completion_tokens);
        if let Some(reported) = sample.prompt_tokens
            && sample.estimated_prompt > 0
            && reported > 0
        {
            entry.1.add(sample.estimated_prompt, reported);
        }
    }

    /// Count a request that the cache answered, with the estimated prompt it did not send.
    pub fn note_cache_hit(&mut self, estimated_prompt: u32) {
        self.cache_hits = self.cache_hits.saturating_add(1);
        self.cache_saved_tokens = self
            .cache_saved_tokens
            .saturating_add(u64::from(estimated_prompt));
    }

    /// How to scale an estimate for this model, in thousandths: [`NEUTRAL_MILLI`] until enough
    /// answers have reported their prompt size.
    #[must_use]
    pub fn calibration_milli(&self, provider: &str, model: &str) -> u32 {
        self.models
            .get(&key(provider, model))
            .map_or(NEUTRAL_MILLI, |(_, ratio)| ratio.milli())
    }

    /// A copy of the counters.
    #[must_use]
    pub fn snapshot(&self) -> UsageSnapshot {
        UsageSnapshot {
            total: self.total,
            cache_hits: self.cache_hits,
            cache_saved_tokens: self.cache_saved_tokens,
            models: self
                .models
                .iter()
                .map(|(name, (totals, _))| (name.clone(), *totals))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample<'a>(model: &'a str, estimated: u32, prompt: Option<u64>) -> Sample<'a> {
        Sample {
            provider: "p",
            model,
            estimated_prompt: estimated,
            prompt_tokens: prompt,
            completion_tokens: Some(10),
        }
    }

    #[test]
    fn totals_add_what_the_services_reported() {
        let mut tracker = UsageTracker::new();
        tracker.record(sample("m", 100, Some(120)));
        tracker.record(sample("m", 100, Some(80)));
        let snapshot = tracker.snapshot();
        assert_eq!(snapshot.total.requests, 2);
        assert_eq!(snapshot.total.prompt_tokens, 200);
        assert_eq!(snapshot.total.completion_tokens, 20);
        assert_eq!(snapshot.total.unreported, 0);
    }

    #[test]
    fn a_request_without_counts_is_counted_but_flagged() {
        let mut tracker = UsageTracker::new();
        tracker.record(Sample {
            provider: "p",
            model: "m",
            estimated_prompt: 50,
            prompt_tokens: None,
            completion_tokens: None,
        });
        let snapshot = tracker.snapshot();
        assert_eq!((snapshot.total.requests, snapshot.total.unreported), (1, 1));
        assert!(
            snapshot
                .lines()
                .join("\n")
                .contains("without a token count")
        );
    }

    #[test]
    fn models_are_counted_apart() {
        let mut tracker = UsageTracker::new();
        tracker.record(sample("a", 10, Some(10)));
        tracker.record(sample("b", 10, Some(10)));
        tracker.record(sample("b", 10, Some(10)));
        let names: Vec<_> = tracker
            .snapshot()
            .models
            .into_iter()
            .map(|(name, totals)| (name, totals.requests))
            .collect();
        assert_eq!(names, [("p/a".to_owned(), 1), ("p/b".to_owned(), 2)]);
    }

    #[test]
    fn calibration_waits_for_enough_samples() {
        let mut tracker = UsageTracker::new();
        for _ in 0..4 {
            tracker.record(sample("m", 100, Some(200)));
        }
        assert_eq!(tracker.calibration_milli("p", "m"), NEUTRAL_MILLI);
        tracker.record(sample("m", 100, Some(200)));
        assert_eq!(tracker.calibration_milli("p", "m"), 2000);
        assert_eq!(tracker.calibration_milli("p", "other"), NEUTRAL_MILLI);
        assert_eq!(tracker.calibration_milli("q", "m"), NEUTRAL_MILLI);
    }

    #[test]
    fn calibration_follows_the_ratio_in_both_directions_and_is_bounded() {
        let ratio = |estimated: u32, reported: u64| {
            let mut tracker = UsageTracker::new();
            for _ in 0..10 {
                tracker.record(sample("m", estimated, Some(reported)));
            }
            tracker.calibration_milli("p", "m")
        };
        assert_eq!(ratio(100, 70), 700, "the estimate was too pessimistic");
        assert_eq!(ratio(100, 100), 1000);
        assert_eq!(ratio(100, 100_000), 3000, "capped above");
        assert_eq!(ratio(100, 1), 500, "capped below");
    }

    #[test]
    fn calibration_ignores_answers_that_cannot_inform_it() {
        let mut tracker = UsageTracker::new();
        for _ in 0..10 {
            tracker.record(sample("m", 0, Some(100)));
            tracker.record(sample("m", 100, None));
            tracker.record(sample("m", 100, Some(0)));
        }
        assert_eq!(tracker.calibration_milli("p", "m"), NEUTRAL_MILLI);
    }

    #[test]
    fn old_samples_fade_so_a_changing_model_is_followed() {
        let mut tracker = UsageTracker::new();
        for _ in 0..1000 {
            tracker.record(sample("m", 100, Some(100)));
        }
        for _ in 0..1000 {
            tracker.record(sample("m", 100, Some(200)));
        }
        assert!(tracker.calibration_milli("p", "m") >= 1900);
    }

    #[test]
    fn cache_hits_are_counted_with_what_they_saved() {
        let mut tracker = UsageTracker::new();
        tracker.note_cache_hit(300);
        tracker.note_cache_hit(200);
        let snapshot = tracker.snapshot();
        assert_eq!((snapshot.cache_hits, snapshot.cache_saved_tokens), (2, 500));
        assert!(!snapshot.is_empty());
        assert!(
            snapshot
                .lines()
                .join("\n")
                .contains("about 500 prompt tokens not sent")
        );
    }

    #[test]
    fn an_empty_counter_says_so_and_nothing_overflows() {
        let mut tracker = UsageTracker::new();
        assert!(tracker.snapshot().is_empty());
        assert_eq!(tracker.snapshot().lines(), ["no model requests yet"]);
        tracker.record(sample("m", u32::MAX, Some(u64::MAX)));
        tracker.record(sample("m", u32::MAX, Some(u64::MAX)));
        assert_eq!(tracker.snapshot().total.prompt_tokens, u64::MAX);
        tracker.note_cache_hit(u32::MAX);
        assert!(tracker.calibration_milli("p", "m") <= 3000);
    }
}
