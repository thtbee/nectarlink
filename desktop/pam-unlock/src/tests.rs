// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io::Write,
    net::Shutdown,
    os::unix::net::UnixStream,
    thread,
    time::{Duration, Instant},
};

use nectarlink_protocol::{DeviceId, LinuxUnlockChallenge};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair},
};

use crate::relay::{read_signature, verify_signature};

#[test]
fn only_a_signature_over_this_attempt_authenticates() {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let challenge =
        LinuxUnlockChallenge { pc: DeviceId([1; 32]), phone: DeviceId([2; 32]), uid: 1000, nonce: [3; 32] };
    let signature = key.sign(&rng, &challenge.signing_bytes()).unwrap();
    let public_key = key.public_key().as_ref();
    assert!(verify_signature(public_key, &challenge, signature.as_ref()));
    assert!(!verify_signature(public_key, &challenge, &[1; 64]));
    let other_attempt = LinuxUnlockChallenge { nonce: [4; 32], ..challenge };
    assert!(!verify_signature(public_key, &other_attempt, signature.as_ref()));
}

#[test]
fn relay_rejects_malformed_messages() {
    for reply in [
        br#"{"version":1,"signature":[1,2]}"#.as_slice(),
        br#"{"version":2,"signature":[1,2]}"#,
        br#"{"version":1,"success":true}"#,
        br#"{"version":1,"signature":[1"#,
    ] {
        let (mut server, mut client) = UnixStream::pair().unwrap();
        server.write_all(reply).unwrap();
        server.shutdown(Shutdown::Write).unwrap();
        let result = read_signature(&mut client, Instant::now() + Duration::from_secs(2));
        if reply == br#"{"version":1,"signature":[1,2]}"# {
            assert_eq!(result.unwrap(), [1, 2]);
        } else {
            assert!(result.is_err());
        }
    }
}

#[test]
fn relay_deadline_counts_time_spent_reading_each_fragment() {
    let (mut server, mut client) = UnixStream::pair().unwrap();
    let worker = thread::spawn(move || {
        while server.write_all(b" ").is_ok() {
            thread::sleep(Duration::from_millis(5));
        }
    });
    let deadline = Instant::now() + Duration::from_millis(60);
    let err = read_signature(&mut client, deadline).unwrap_err();
    assert!(matches!(err.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock));
    assert!(Instant::now() < deadline + Duration::from_secs(1));
    drop(client);
    worker.join().unwrap();
}
