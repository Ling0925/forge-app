//! UI-generated idempotency ids for `Send.input_id` / `Steer.steer_id` (R10).
//!
//! The id is generated once, at the moment the user presses the key, and is kept
//! in `ComposerVm.in_flight` so a transport-level replay of the same request
//! reuses it. A new user action always gets a new id.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::{SystemTime, UNIX_EPOCH};

pub trait IdSource: Send {
    /// Returns a new, never-repeated id starting with `prefix`.
    fn next_id(&mut self, prefix: &str) -> String;
}

/// Production generator: 128 bits mixed from the OS-seeded std hasher, wall
/// clock and a counter. Uniqueness inside one app process is guaranteed by the
/// counter; the random part keeps ids unique across restarts. No extra crate.
pub struct RandomIds {
    counter: u64,
    seed: RandomState,
}

impl Default for RandomIds {
    fn default() -> Self {
        Self {
            counter: 0,
            seed: RandomState::new(),
        }
    }
}

impl IdSource for RandomIds {
    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or_default();
        let mut high = self.seed.build_hasher();
        high.write_u64(nanos);
        high.write_u64(self.counter);
        let high = high.finish();
        let mut low = RandomState::new().build_hasher();
        low.write_u64(high);
        low.write_u64(self.counter);
        format!("{prefix}-{high:016x}{:016x}-{}", low.finish(), self.counter)
    }
}

/// Deterministic generator for tests: `prefix-1`, `prefix-2`, ...
#[derive(Default)]
pub struct SequentialIds {
    counter: u64,
}

impl IdSource for SequentialIds {
    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn random_ids_do_not_repeat() {
        let mut ids = RandomIds::default();
        let set: HashSet<_> = (0..10_000).map(|_| ids.next_id("in")).collect();
        assert_eq!(set.len(), 10_000);
        assert!(set.iter().all(|id| id.starts_with("in-")));
    }

    #[test]
    fn sequential_ids() {
        let mut ids = SequentialIds::default();
        assert_eq!(ids.next_id("in"), "in-1");
        assert_eq!(ids.next_id("steer"), "steer-2");
    }
}
