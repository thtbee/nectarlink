// SPDX-License-Identifier: GPL-3.0-or-later
import {blake3} from '@noble/hashes/blake3.js';
import nacl from 'tweetnacl';
import {decode43} from './base43.js';
import {equal, join, utf8} from './bytes.js';
import {indexBytes, lanChunkBytes, readIndex, relayChunkBytes} from './file-protocol.js';
import {transferFile} from './transfer.js';
import {createReceiverView} from './receiver-view.js';
import {connectRelay} from './relay.js';
import {fileKey, lanProbe, siteAck, siteClaim} from './site-protocol.js';

const context = utf8.encode('iroh-relay handshake v1 challenge signature');
const view = createReceiverView();

async function lanRequest(base, path, body, signal) {
  const response = await fetch(new URL(path, base), {
    method: 'POST',
    body,
    signal,
    credentials: 'omit',
    headers: {'Content-Type': 'application/octet-stream'},
    ...(base.origin === location.origin ? {} : {targetAddressSpace: 'local'}),
  });
  if (!response.ok) throw Error('No reply');
  return new Uint8Array(await response.arrayBuffer());
}

async function probe(base, peer, signal) {
  try {
    const challenge = crypto.getRandomValues(new Uint8Array(32));
    const signature = await lanRequest(base, '/probe', challenge,
      AbortSignal.any([signal, AbortSignal.timeout(10000)]));
    return signature.length === 64 && nacl.sign.detached.verify(join(lanProbe, challenge), signature, peer);
  } catch {
    return false;
  }
}

function connectLan(base, guest, onPacket, signal) {
  return async (packets) => {
    const body = join(guest, ...packets.flatMap((packet) => [indexBytes(packet.length), packet]));
    const content = await lanRequest(base, '/packet', body, signal);
    let offset = 0;
    while (offset < content.length) {
      if (offset + 4 > content.length) throw Error('Invalid packet');
      const length = readIndex(content.subarray(offset));
      offset += 4;
      if (length === 0 || offset + length > content.length) throw Error('Invalid packet');
      onPacket(content.subarray(offset, offset + length));
      offset += length;
    }
  };
}

function parseTicket(fragment, direct) {
  const ticket = decode43(fragment);
  if (ticket[0] !== 2 || (direct ? ticket.length !== 77 : ticket.length <= 77)) {
    throw Error('Invalid ticket');
  }
  const expires = ticket.subarray(1, 7).reduce((value, byte) => value * 256 + byte, 0);
  const peer = ticket.subarray(7, 39);
  const seed = ticket.subarray(39, 71);
  if (direct) return {expires, peer, seed, local: new URL(location.origin)};
  const [a, b, c, d] = ticket.subarray(71, 75);
  const port = ticket[75] * 256 + ticket[76];
  const privateIp = a === 10 || (a === 172 && b >= 16 && b < 32) || (a === 192 && b === 168);
  const local = privateIp && port ? new URL(`http://${a}.${b}.${c}.${d}:${port}/`) : null;
  const relay = new URL(new TextDecoder().decode(ticket.subarray(77)));
  if (relay.protocol !== 'wss:') throw Error('Invalid ticket');
  return {expires, peer, seed, local, relay};
}

function claim(send, listen, packet, signal) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    let timer;
    let attempts = 0;
    function finish(error, reply) {
      clearTimeout(timer);
      signal.removeEventListener('abort', abort);
      listen(null);
      if (error) reject(error);
      else resolve(reply);
    }
    const abort = () => finish(signal.reason);
    function attempt() {
      if (attempts++ === 4) {
        finish(Error('No reply'));
        return;
      }
      timer = setTimeout(attempt, 2500);
      send([packet]).catch(() => {});
    }
    signal.addEventListener('abort', abort, {once: true});
    listen((reply) => {
      if (reply.length === 1 || (reply.length === 97 && reply[0] === 1)) finish(null, reply);
    });
    attempt();
  });
}

async function transfer() {
  const abort = new AbortController();
  const {signal} = abort;
  let relay, timeout, onPacket;
  try {
    const direct = location.protocol === 'http:';
    const {expires, peer, seed, local, relay: relayUrl} = parseTicket(location.hash.slice(1), direct);
    history.replaceState(null, '', location.pathname);
    const remaining = expires - Date.now();
    if (remaining <= 0) throw Error('Ticket expired');
    timeout = setTimeout(() => abort.abort(Error('Ticket expired')), Math.min(remaining, 0x7fffffff));
    const receive = (packet) => onPacket?.(packet);
    const listen = (handler) => { onPacket = handler; };

    const {publicKey, secretKey} = nacl.sign.keyPair();
    const identity = {publicKey, sign: (bytes) => nacl.sign.detached(bytes, secretKey)};
    const useLan = direct || (local && await view.chooseLocal(signal) && await probe(local, peer, signal));
    signal.throwIfAborted();
    let send;
    if (useLan) {
      send = connectLan(local, publicKey, receive, signal);
    } else {
      relay = await connectRelay(relayUrl, identity, (sender, bytes) => {
        if (equal(sender, peer)) receive(bytes);
      }, (challenge) => blake3(challenge, {context}), signal, (error) => abort.abort(error));
      send = async (packets) => {
        signal.throwIfAborted();
        for (const packet of packets) relay.send(peer, packet);
      };
    }
    const route = [useLan ? 0 : 1];
    const exchange = nacl.box.keyPair();
    const ticket = nacl.sign.keyPair.fromSeed(seed);
    const signature = nacl.sign.detached(join(siteClaim, peer, publicKey, route, exchange.publicKey),
      ticket.secretKey);
    const reply = await claim(send, listen, join([1], exchange.publicKey, signature), signal);
    if (reply[0] === 0) throw Error('Ticket used or expired');
    const hostKey = reply.subarray(1, 33);
    if (!nacl.sign.detached.verify(join(siteAck, publicKey, route, exchange.publicKey, hostKey),
      reply.subarray(33), peer)) {
      throw Error('File authentication failed');
    }
    const shared = nacl.scalarMult(exchange.secretKey, hostKey);
    if (shared.every((byte) => byte === 0)) throw Error('File authentication failed');
    const key = nacl.hash(join(fileKey, peer, seed, publicKey, route, shared)).subarray(0, 32);
    await transferFile(send, listen, key, useLan ? lanChunkBytes : relayChunkBytes, view, signal);
  } catch (error) {
    view.error(error.message);
  } finally {
    clearTimeout(timeout);
    abort.abort();
  }
}

void transfer();
