// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use bytes::Bytes;
use iroh::{PublicKey, SecretKey, Signature};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey},
    agreement, hmac,
    rand::SystemRandom,
};
use sha2::{Digest, Sha512};

use super::Route;

const CLAIM: &[u8] = b"AnySend browser v2";
const ACK: &[u8] = b"AnySend sender v2";
const KEY: &[u8] = b"AnySend file key v2";
pub(super) const PROBE: &[u8] = b"AnySend LAN probe v2";
pub(super) const RELAY_PROBE: &[u8] = b"AnySend relay probe v3";
pub(super) const PROBE_PACKET: u8 = 5;
pub(super) const PULL: &[u8] = b"AnySend file request v2";
pub(super) const PUSH: &[u8] = b"AnySend file upload v2";
pub(super) const COMPLETE: &[u8] = b"AnySend file complete v2";
pub(super) const COMPLETE_INDEX: u32 = u32::MAX - 1;
pub(super) const ERROR_INDEX: u32 = u32::MAX;
pub(super) type Cipher = LessSafeKey;

pub(super) fn relay_probe(
    host: &SecretKey,
    ticket: &SecretKey,
    guest: PublicKey,
    probe: &[u8],
) -> Option<Bytes> {
    let (challenge, tag) = probe.split_at_checked(32)?;
    let message = [RELAY_PROBE, host.public().as_bytes(), guest.as_bytes(), challenge].concat();
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, &ticket.to_bytes()), &message, tag).ok()?;
    let signature = host.sign(&[RELAY_PROBE, challenge].concat());
    Some(Bytes::from([&[PROBE_PACKET], challenge, signature.to_bytes().as_slice()].concat()))
}

pub(super) fn verify_claim(
    host: PublicKey,
    ticket: PublicKey,
    guest: PublicKey,
    route: Route,
    claim: &[u8],
) -> Option<[u8; 32]> {
    let key: [u8; 32] = claim.get(..32)?.try_into().ok()?;
    let signature = Signature::from_bytes(claim.get(32..)?.try_into().ok()?);
    ticket
        .verify(&[CLAIM, host.as_bytes(), guest.as_bytes(), &[route.code()], &key].concat(), &signature)
        .ok()?;
    Some(key)
}

pub(super) fn accept_claim(
    host: &SecretKey,
    ticket: &SecretKey,
    guest: PublicKey,
    route: Route,
    guest_key: [u8; 32],
) -> Result<(Cipher, Bytes)> {
    let secret = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &SystemRandom::new())
        .map_err(|_| anyhow::anyhow!("Could not generate an AnySend key."))?;
    let public =
        secret.compute_public_key().map_err(|_| anyhow::anyhow!("Could not read the AnySend key."))?;
    let guest_public = agreement::UnparsedPublicKey::new(&agreement::X25519, guest_key);
    let key = agreement::agree_ephemeral(secret, &guest_public, |shared| {
        Sha512::new()
            .chain_update(KEY)
            .chain_update(host.public().as_bytes())
            .chain_update(ticket.to_bytes())
            .chain_update(guest.as_bytes())
            .chain_update([route.code()])
            .chain_update(shared)
            .finalize()
    })
    .map_err(|_| anyhow::anyhow!("Invalid AnySend key."))?;
    let ack = host.sign(&[ACK, guest.as_bytes(), &[route.code()], &guest_key, public.as_ref()].concat());
    let ack = Bytes::from([&[1], public.as_ref(), ack.to_bytes().as_slice()].concat());
    Ok((LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key[..32]).expect("32-byte AES key")), ack))
}

pub(super) fn read_index(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(bytes.get(..4)?.try_into().ok()?))
}

pub(super) fn open(cipher: &Cipher, frame: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    let (nonce, data) = frame.split_at_checked(12)?;
    let mut data = data.to_vec();
    let length = cipher
        .open_in_place(Nonce::try_assume_unique_for_key(nonce).ok()?, Aad::from(aad), &mut data)
        .ok()?
        .len();
    data.truncate(length);
    Some(data)
}

pub(super) fn seal(cipher: &Cipher, index: u32, plaintext: &[u8]) -> Result<Bytes> {
    let header = index.to_be_bytes();
    let nonce = rand::random::<[u8; 12]>();
    let mut packet = Vec::with_capacity(32 + plaintext.len());
    packet.extend_from_slice(&header);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(plaintext);
    let tag = cipher
        .seal_in_place_separate_tag(Nonce::assume_unique_for_key(nonce), Aad::from(header), &mut packet[16..])
        .ok()
        .context("Could not encrypt the file payload.")?;
    packet.extend_from_slice(tag.as_ref());
    Ok(packet.into())
}
