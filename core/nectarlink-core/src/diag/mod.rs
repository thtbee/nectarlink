// SPDX-License-Identifier: MPL-2.0
//! Diagnostics that stay on this device until its user sends them: the
//! activity log every app writes, and the helpers that keep personal details
//! out of it.
//!
//! The log is private by construction rather than cleaned up afterwards
//! ([`layer`]): a value only reaches it when its field is one of a short list
//! of names known to carry codes, counts and short IDs, and every piece of
//! text written is stripped of this device's folders and network addresses
//! ([`redact`]). It's kept in two files of a fixed size ([`ring`]), so it
//! never grows past that however long the app runs.

pub mod crash;
pub mod health;
pub mod layer;
pub mod redact;
pub mod report;
pub mod ring;
pub mod send;

pub use layer::RingLayer;
pub use redact::{addr, init_roots, path, peer};
pub use ring::RingLog;
