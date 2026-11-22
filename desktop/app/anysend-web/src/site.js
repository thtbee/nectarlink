// SPDX-License-Identifier: GPL-3.0-or-later
import {blake3} from '@noble/hashes/blake3.js';
import nacl from 'tweetnacl';
import {decode43} from './base43.js';
import {equal, join, utf8} from './bytes.js';
import {transferFile} from './transfer.js';
import {createReceiverView} from './receiver-view.js';
import {connectRelay} from './relay.js';
import {siteAck, siteClaim} from './site-protocol.js';

const context = utf8.encode('iroh-relay handshake v1 challenge signature');
const view = createReceiverView();

function connectLan(onPacket) {
  const abort = new AbortController();
  return {
    async send(_peer, packet) {
      const response = await fetch('./packet', {
        method: 'POST',
        body: packet,
        signal: abort.signal,
        headers: {'Content-Type': 'application/octet-stream'},
      });
      if (!response.ok) throw Error('No reply');
      const content = new DataView(await response.arrayBuffer());
      let offset = 0;
      while (offset < content.byteLength) {
        if (offset + 4 > content.byteLength) throw Error('Invalid ticket');
        const length = content.getUint32(offset);
        offset += 4;
        if (offset + length > content.byteLength) throw Error('Invalid ticket');
        onPacket(new Uint8Array(content.buffer, offset, length));
        offset += length;
      }
    },
    close: () => abort.abort(),
  };
}

async function transfer() {
  let connection, timeout, onPacket;
  try {
    const ticket = decode43(location.hash.slice(1));
    const lan = location.protocol === 'http:';
    if (lan ? ticket.length !== 70 : ticket.length <= 70) throw Error('Invalid ticket');
    history.replaceState(null, '', location.pathname);
    const expires = ticket.subarray(0, 6).reduce((value, byte) => value * 256 + byte, 0);
    if (Date.now() >= expires) throw Error('Ticket expired');
    const peer = ticket.subarray(6, 38);
    const seed = ticket.subarray(38, 70);
    const {publicKey, secretKey} = nacl.sign.keyPair.fromSeed(seed);
    const identity = {publicKey, sign: (bytes) => nacl.sign.detached(bytes, secretKey)};
    if (lan) {
      connection = connectLan((bytes) => onPacket(bytes));
    } else {
      const relay = new URL(new TextDecoder().decode(ticket.subarray(70)));
      if (relay.protocol !== 'wss:') throw Error('Invalid ticket');
      connection = await connectRelay(relay, identity, (sender, bytes) => {
        if (equal(sender, peer) && onPacket) onPacket(bytes);
      }, (challenge) => blake3(challenge, {context}));
    }
    const send = (bytes) => connection.send(peer, bytes);
    const reply = await new Promise((resolve, reject) => {
      onPacket = resolve;
      timeout = setTimeout(() => reject(Error('No reply')), 15000);
      Promise.resolve(send(identity.sign(join(siteClaim, peer)))).catch(reject);
    });
    clearTimeout(timeout);
    if (reply[0] === 0) throw Error('Ticket used or expired');
    if (reply[0] !== 1 || reply.length !== 65 ||
        !nacl.sign.detached.verify(join(siteAck, publicKey), reply.subarray(1), peer)) {
      throw Error('File authentication failed');
    }
    const key = nacl.hash(join(peer, seed)).subarray(0, 32);
    await transferFile(send, (callback) => { onPacket = callback; }, key, view);
  } catch (error) {
    view.error(error.message);
  } finally {
    clearTimeout(timeout);
    connection?.close();
  }
}

void transfer();
