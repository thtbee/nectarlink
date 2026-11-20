// SPDX-License-Identifier: GPL-3.0-or-later
//! What the credential provider (in LogonUI, as SYSTEM) and the signed-in user's
//! NectarLink say over the unlock pipe (docs/adr/0009-unlock-with-phone.md).
//!
//! One exchange per connection: the provider sends a request naming the Windows
//! user; NectarLink asks the enrolled phone and answers with a status and, when
//! the phone's owner confirmed, the password. Pure code, so it's tested on every
//! platform. The desktop app includes this file as its own module (it doesn't
//! link the DLL's crate), so it depends on nothing but `std`.

/// CLSID of the NectarLink unlock credential provider.
pub const CLSID_TEXT: &str = "{BC7B4BCF-B24C-4C5B-9E55-CB09E19A8799}";

/// The pipe for Windows session `session`: one per session, since each has
/// its own LogonUI and its own signed-in NectarLink.
pub fn pipe_name(session: u32) -> String {
    format!(r"\\.\pipe\NectarLink.Unlock.{session}")
}

/// Starts every request, so a stray connection isn't read as one.
const MAGIC: &[u8; 4] = b"NLU1";
/// Starts a notice that Windows refused the password it was given.
const REJECTED: &[u8; 4] = b"NLR1";
/// Longest security identifier accepted.
pub const MAX_USER: usize = 184;
/// Longest answer accepted: the password sealed to the machine key
/// (`system_seal::MAX_SEALED`), with room to spare.
pub const MAX_PASSWORD: usize = 2048;
/// How long the provider waits for an answer: the phone has 30 s, plus margin.
pub const ANSWER_TIMEOUT_MS: u32 = 40_000;

/// How an unlock request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    /// Here's the password.
    Ok = 0,
    /// Declined on the phone, or not confirmed in time.
    Declined = 1,
    /// This user isn't set up to unlock with a phone (or the phone's key is
    /// gone and it must be set up again).
    NotEnrolled = 2,
    /// The phone isn't connected on this network.
    PhoneAway = 3,
    /// Too many failed tries; it works again in a while.
    LockedOut = 4,
    /// Anything else.
    Failed = 5,
    /// Nobody has typed the password since a restart: it's needed once first.
    PasswordFirst = 6,
}

impl Status {
    fn from_byte(b: u8) -> Option<Status> {
        Some(match b {
            0 => Status::Ok,
            1 => Status::Declined,
            2 => Status::NotEnrolled,
            3 => Status::PhoneAway,
            4 => Status::LockedOut,
            5 => Status::Failed,
            6 => Status::PasswordFirst,
            _ => return None,
        })
    }
}

/// Whether `user` looks like a Windows security identifier.
pub fn user_ok(user: &str) -> bool {
    !user.is_empty()
        && user.len() <= MAX_USER
        && user.starts_with("S-")
        && user.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// The request for `user` (a security identifier).
pub fn request(user: &str) -> Option<Vec<u8>> {
    naming(MAGIC, user)
}

/// The user a request names, if it's a whole, valid request.
pub fn parse_request(bytes: &[u8]) -> Option<String> {
    named(MAGIC, bytes)
}

/// The notice that Windows refused `user`'s password from the phone (it changed
/// since setting up). No answer follows.
pub fn rejected(user: &str) -> Option<Vec<u8>> {
    naming(REJECTED, user)
}

/// The user a rejection notice names, if it's a whole, valid one.
pub fn parse_rejected(bytes: &[u8]) -> Option<String> {
    named(REJECTED, bytes)
}

fn naming(magic: &[u8; 4], user: &str) -> Option<Vec<u8>> {
    if !user_ok(user) {
        return None;
    }
    let mut out = Vec::with_capacity(magic.len() + 1 + user.len());
    out.extend_from_slice(magic);
    out.push(user.len() as u8);
    out.extend_from_slice(user.as_bytes());
    Some(out)
}

fn named(magic: &[u8; 4], bytes: &[u8]) -> Option<String> {
    let rest = bytes.strip_prefix(magic)?;
    let (&len, user) = rest.split_first()?;
    if user.len() != usize::from(len) {
        return None;
    }
    let user = std::str::from_utf8(user).ok()?;
    user_ok(user).then(|| user.to_owned())
}

/// An answer: `status`, and the password with [`Status::Ok`]. Wipe the result
/// once it's written.
pub fn answer(status: Status, password: Option<&[u8]>) -> Vec<u8> {
    let password = if status == Status::Ok { password.unwrap_or_default() } else { &[] };
    let password = &password[..password.len().min(MAX_PASSWORD)];
    let mut out = Vec::with_capacity(1 + 4 + password.len());
    out.push(status as u8);
    out.extend_from_slice(&(password.len() as u32).to_le_bytes());
    out.extend_from_slice(password);
    out
}

/// The status and password in an answer; `None` if it isn't one.
pub fn parse_answer(bytes: &[u8]) -> Option<(Status, Vec<u8>)> {
    let (&status, rest) = bytes.split_first()?;
    let status = Status::from_byte(status)?;
    let len = u32::from_le_bytes(rest.get(..4)?.try_into().ok()?) as usize;
    let password = rest.get(4..)?;
    if password.len() != len || len > MAX_PASSWORD || (status != Status::Ok && len != 0) {
        return None;
    }
    Some((status, password.to_vec()))
}

/// Overwrites `bytes` so a secret doesn't linger in memory.
pub fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(bytes);
}

/// `DOMAIN\user` split for Kerberos: `("MicrosoftAccount", "me@example.com")`
/// or `("PC-NAME", "bee")`. A name without a domain gets `"."` (this PC).
pub fn split_qualified(qualified: &str) -> (&str, &str) {
    match qualified.split_once('\\') {
        Some((domain, user)) if !domain.is_empty() && !user.is_empty() => (domain, user),
        _ => (".", qualified),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejection_notice_is_never_read_as_a_request() {
        let sid = "S-1-5-21-1-2-3-1001";
        let notice = rejected(sid).unwrap();
        assert_eq!(parse_rejected(&notice).as_deref(), Some(sid));
        assert_eq!(parse_request(&notice), None, "a notice never asks for a password");
        assert_eq!(parse_rejected(&request(sid).unwrap()), None);
        assert_eq!(rejected("not a sid"), None);
        assert_eq!(parse_rejected(&notice[..notice.len() - 1]), None);
    }

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn requests_round_trip_and_reject_anything_else() {
        let r = request(SID).unwrap();
        assert_eq!(parse_request(&r).as_deref(), Some(SID));
        assert!(request("not a sid").is_none());
        assert!(request(&format!("S-{}", "1".repeat(MAX_USER))).is_none());
        assert!(parse_request(b"NLU1").is_none());
        assert!(parse_request(&r[..r.len() - 1]).is_none(), "cut short");
        let mut extra = r.clone();
        extra.push(b'1');
        assert!(parse_request(&extra).is_none(), "trailing bytes");
        assert!(parse_request(b"XXXX\x03S-1").is_none());
    }

    #[test]
    fn answers_carry_a_password_only_when_ok() {
        let a = answer(Status::Ok, Some("pässwörd".as_bytes()));
        assert_eq!(parse_answer(&a), Some((Status::Ok, "pässwörd".as_bytes().to_vec())));
        let d = answer(Status::Declined, Some(b"leak"));
        assert_eq!(parse_answer(&d), Some((Status::Declined, Vec::new())), "never sent on refusal");
        for s in
            [Status::NotEnrolled, Status::PhoneAway, Status::LockedOut, Status::Failed, Status::PasswordFirst]
        {
            assert_eq!(parse_answer(&answer(s, None)).map(|(s, _)| s), Some(s));
        }
        assert!(parse_answer(&[9, 0, 0, 0, 0]).is_none(), "unknown status");
        assert!(parse_answer(&[1, 1, 0, 0, 0, b'x']).is_none(), "a password on a refusal");
        assert!(parse_answer(&[0, 5, 0, 0, 0, b'x']).is_none(), "wrong length");
    }

    #[test]
    fn qualified_names_split_at_the_first_backslash() {
        assert_eq!(
            split_qualified(r"MicrosoftAccount\me@example.com"),
            ("MicrosoftAccount", "me@example.com")
        );
        assert_eq!(split_qualified(r"PC-NAME\bee"), ("PC-NAME", "bee"));
        assert_eq!(split_qualified("bee"), (".", "bee"));
        assert_eq!(split_qualified(r"\bee"), (".", r"\bee"));
    }

    #[test]
    fn pipe_names_are_per_session() {
        assert_eq!(pipe_name(1), r"\\.\pipe\NectarLink.Unlock.1");
        assert_ne!(pipe_name(1), pipe_name(2));
    }
}
