// SPDX-License-Identifier: GPL-3.0-or-later
//! Versions and the release channels they belong to.
//!
//! A stable release is `1.2.3`; a nightly is `1.2.3-nightly.N`, a build on
//! the way to `1.2.3`, so every nightly of a release sorts before it and
//! after the release before. Pre-release parts compare as semver does:
//! number by number (`nightly.10` is after `nightly.9`).

use std::cmp::Ordering;

/// Which builds an install follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Releases only.
    Stable,
    /// A build from `main` every couple of hours, and releases.
    Nightly,
}

impl Channel {
    /// Its name in settings, feeds and logs: `stable` or `nightly`.
    pub fn as_str(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Nightly => "nightly",
        }
    }

    pub fn parse(s: &str) -> Option<Channel> {
        match s.trim() {
            "stable" => Some(Channel::Stable),
            "nightly" => Some(Channel::Nightly),
            _ => None,
        }
    }
}

/// A parsed version: its three numbers and its pre-release part, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Version<'a> {
    numbers: (u64, u64, u64),
    pre: Option<&'a str>,
}

fn parse(v: &str) -> Option<Version<'_>> {
    // Build metadata (`+…`) doesn't take part in ordering.
    let v = v.split_once('+').map_or(v, |(v, _)| v);
    let (numbers, pre) = match v.split_once('-') {
        Some((n, p)) if !p.is_empty() => (n, Some(p)),
        Some(_) => return None,
        None => (v, None),
    };
    let mut parts = numbers.split('.').map(str::parse::<u64>);
    let numbers = (parts.next()?.ok()?, parts.next()?.ok()?, parts.next().unwrap_or(Ok(0)).ok()?);
    if parts.next().is_some() {
        return None;
    }
    Some(Version { numbers, pre })
}

/// Semver order of two pre-release parts: field by field, numbers as
/// numbers and before words, a shorter list first when the rest is equal.
fn compare_pre(a: &str, b: &str) -> Ordering {
    let mut a = a.split('.');
    let mut b = b.split('.');
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

/// The order of two versions, or `None` if either isn't one.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let (a, b) = (parse(a)?, parse(b)?);
    Some(a.numbers.cmp(&b.numbers).then_with(|| match (a.pre, b.pre) {
        (None, None) => Ordering::Equal,
        // A release is newer than its pre-releases.
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(x), Some(y)) => compare_pre(x, y),
    }))
}

/// Whether `candidate` is a newer version than `current`.
pub fn is_newer(candidate: &str, current: &str) -> bool {
    compare(candidate, current) == Some(Ordering::Greater)
}

/// The nightly build number of `1.2.3-nightly.N`.
pub fn nightly_build(version: &str) -> Option<u32> {
    let pre = parse(version)?.pre?;
    let (word, n) = pre.split_once('.')?;
    (word == "nightly").then(|| n.parse().ok()).flatten()
}

/// The channel a build belongs to: nightlies to Nightly, everything else to Stable.
pub fn channel_of(version: &str) -> Channel {
    if nightly_build(version).is_some() { Channel::Nightly } else { Channel::Stable }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare() {
        assert!(is_newer("0.1.0", "0.0.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(is_newer("0.2.0-beta.2", "0.2.0-beta.1"));
        assert!(!is_newer("0.0.1", "0.0.1"));
        assert!(!is_newer("0.0.1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(!is_newer("0.1.0", "garbage"));
        assert!(!is_newer("0.1.0-", "0.0.1"));
        assert!(!is_newer("1.2.3.4", "0.0.1"));
    }

    #[test]
    fn nightlies_sort_by_number_between_releases() {
        // Numbers, not text: 10 comes after 9.
        assert!(is_newer("0.1.5-nightly.10", "0.1.5-nightly.9"));
        assert!(!is_newer("0.1.5-nightly.9", "0.1.5-nightly.10"));
        // A nightly is newer than the release before and older than its own.
        assert!(is_newer("0.1.5-nightly.1", "0.1.4"));
        assert!(is_newer("0.1.5", "0.1.5-nightly.42"));
        assert!(!is_newer("0.1.5-nightly.42", "0.1.5"));
        assert!(is_newer("0.2.0", "0.1.5-nightly.42"));
        // Build metadata doesn't change the order.
        assert_eq!(compare("0.1.5+abc", "0.1.5+def"), Some(Ordering::Equal));
    }

    #[test]
    fn channels_come_from_the_version() {
        assert_eq!(channel_of("0.1.5-nightly.3"), Channel::Nightly);
        assert_eq!(channel_of("0.1.5"), Channel::Stable);
        assert_eq!(channel_of("0.2.0-beta.1"), Channel::Stable);
        assert_eq!(nightly_build("0.1.5-nightly.42"), Some(42));
        assert_eq!(nightly_build("0.1.5-nightly"), None);
        assert_eq!(nightly_build("0.1.5"), None);
        assert_eq!(Channel::parse(" nightly\n"), Some(Channel::Nightly));
        assert_eq!(Channel::parse("beta"), None);
    }
}
