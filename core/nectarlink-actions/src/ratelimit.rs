// SPDX-License-Identifier: MPL-2.0
//! Per-client limits, so a runaway or hijacked agent can't flood the phone,
//! the user's contacts or the confirmation window.

use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

use crate::def::RateClass;

/// (window, most calls in it) pairs per class.
fn limits(class: RateClass) -> &'static [(Duration, usize)] {
    const MIN: Duration = Duration::from_secs(60);
    const DAY: Duration = Duration::from_secs(86_400);
    match class {
        RateClass::Read => &[(MIN, 120)],
        RateClass::Write => &[(MIN, 30)],
        RateClass::Sensitive => &[(MIN, 5), (DAY, 30)],
    }
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    calls: HashMap<(String, ClassKey), VecDeque<Instant>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ClassKey(u8);

impl From<RateClass> for ClassKey {
    fn from(c: RateClass) -> Self {
        ClassKey(match c {
            RateClass::Read => 0,
            RateClass::Write => 1,
            RateClass::Sensitive => 2,
        })
    }
}

impl RateLimiter {
    /// Counts a call, or says how long to wait before the next one may go.
    pub fn check(&mut self, client: &str, class: RateClass, now: Instant) -> Result<(), Duration> {
        let rules = limits(class);
        let longest = rules.iter().map(|(w, _)| *w).max().unwrap_or_default();
        let calls = self.calls.entry((client.to_owned(), class.into())).or_default();
        while calls.front().is_some_and(|t| now.duration_since(*t) >= longest) {
            calls.pop_front();
        }
        for (window, max) in rules {
            let in_window: Vec<&Instant> =
                calls.iter().filter(|t| now.duration_since(**t) < *window).collect();
            if in_window.len() >= *max {
                let oldest = in_window[in_window.len() - max];
                return Err(*window - now.duration_since(*oldest));
            }
        }
        calls.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_calls_are_capped_per_minute_and_recover() {
        let mut limiter = RateLimiter::default();
        let start = Instant::now();
        for _ in 0..5 {
            limiter.check("a", RateClass::Sensitive, start).unwrap();
        }
        let wait = limiter.check("a", RateClass::Sensitive, start).unwrap_err();
        assert!(wait <= Duration::from_secs(60) && wait > Duration::from_secs(59));
        // Another client has its own budget.
        limiter.check("b", RateClass::Sensitive, start).unwrap();
        // A minute later the first one may go again.
        limiter.check("a", RateClass::Sensitive, start + Duration::from_secs(61)).unwrap();
    }

    #[test]
    fn reads_have_a_larger_budget_than_writes() {
        let mut limiter = RateLimiter::default();
        let now = Instant::now();
        for _ in 0..30 {
            limiter.check("a", RateClass::Write, now).unwrap();
        }
        assert!(limiter.check("a", RateClass::Write, now).is_err());
        for _ in 0..120 {
            limiter.check("a", RateClass::Read, now).unwrap();
        }
        assert!(limiter.check("a", RateClass::Read, now).is_err());
    }
}
