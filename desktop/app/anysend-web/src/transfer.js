// SPDX-License-Identifier: GPL-3.0-or-later
import {gcm} from '@noble/ciphers/aes.js';
import {join, utf8} from './bytes.js';
import {
  batchSize, completeContext, completeIndex, errorIndex, indexBytes, maxFileBytes,
  readIndex, requestContext, uploadContext,
} from './file-protocol.js';

const inFlightBatches = 4;
const text = new TextDecoder();

async function createCipher(key) {
  const native = globalThis.crypto.subtle;
  const imported = await native?.importKey('raw', key, 'AES-GCM', false, ['encrypt', 'decrypt']);
  return {
    async encrypt(aad, bytes) {
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const data = imported
        ? new Uint8Array(await native.encrypt({name: 'AES-GCM', iv, additionalData: aad}, imported, bytes))
        : gcm(key, iv, aad).encrypt(bytes);
      return join(iv, data);
    },
    async decrypt(packet) {
      const iv = packet.subarray(4, 16);
      const aad = packet.subarray(0, 4);
      const bytes = packet.subarray(16);
      return imported
        ? new Uint8Array(await native.decrypt({name: 'AES-GCM', iv, additionalData: aad}, imported, bytes))
        : gcm(key, iv, aad).decrypt(bytes);
    },
  };
}

export async function transferFile(send, listen, key, chunkBytes, view, signal) {
  const cipher = await createCipher(key);
  const exchanges = new Map();
  const activityTimers = new Set();
  listen(async (packet) => {
    if (packet.length < 32) return;
    const index = readIndex(packet);
    const accept = exchanges.get(index);
    if (!accept && index !== errorIndex) return;
    try {
      const data = await cipher.decrypt(packet);
      if (index === errorIndex) {
        for (const handle of new Set(exchanges.values())) handle(errorIndex, Error(text.decode(data)));
      } else {
        accept(index, data);
      }
    } catch {
      // The relay can drop data. Unauthenticated frames have the same effect as dropped frames.
    }
  });

  function exchange(first, count, sendPending) {
    signal.throwIfAborted();
    return new Promise((resolve, reject) => {
      const missing = new Set();
      const chunks = new Array(count);
      let timer, finished = false, retries = 0;
      function finish(error) {
        if (finished) return;
        finished = true;
        clearTimeout(timer);
        activityTimers.delete(retryTimer);
        signal.removeEventListener('abort', abort);
        for (const index of missing) exchanges.delete(index);
        if (error) reject(error);
        else resolve(chunks);
      }
      const abort = () => finish(signal.reason);
      function accept(index, data) {
        if (index === errorIndex) {
          finish(data);
          return;
        }
        if (finished || !missing.delete(index)) return;
        exchanges.delete(index);
        chunks[index - first] = data;
        if (!missing.size) finish();
        // Other batches can be queued behind this one on a throttled relay.
        for (const refresh of activityTimers) refresh();
      }
      function retryTimer() {
        clearTimeout(timer);
        timer = setTimeout(() => {
          if (retries === 3) finish(Error('File transfer timed out'));
          else {
            retries++;
            attempt();
          }
        }, 6000 * 2 ** retries);
      }
      function attempt() {
        retryTimer();
        sendPending([...missing]).catch(() => {});
      }
      for (let index = first; index < first + count; index++) {
        missing.add(index);
        exchanges.set(index, accept);
      }
      signal.addEventListener('abort', abort, {once: true});
      activityTimers.add(retryTimer);
      attempt();
    });
  }

  async function request(first, count) {
    const frame = await cipher.encrypt(requestContext, join(indexBytes(first), indexBytes(count)));
    await send([join([2], frame)]);
  }

  function pull(first, count) {
    return exchange(first, count, async (missing) => {
      let start = missing[0], end = start;
      const requests = [];
      for (const index of missing.slice(1)) {
        if (index === end + 1) end = index;
        else {
          requests.push(request(start, end - start + 1));
          start = end = index;
        }
      }
      requests.push(request(start, end - start + 1));
      await Promise.all(requests);
    });
  }

  async function push(index, bytes) {
    const header = indexBytes(index);
    const frame = await cipher.encrypt(join(uploadContext, header), bytes);
    return join([3], header, frame);
  }

  async function batches(count, process) {
    let next = 1;
    async function worker() {
      while (next <= count) {
        signal.throwIfAborted();
        const first = next;
        next += batchSize;
        await process(first, Math.min(batchSize, count - first + 1));
      }
    }
    const workers = Math.min(inFlightBatches, Math.ceil(count / batchSize));
    await Promise.all(Array.from({length: workers}, worker));
  }

  view.waiting();
  const [response] = await pull(0, 1);
  const metadata = JSON.parse(text.decode(response));
  if (metadata.direction === 'upload') {
    const file = await view.chooseFile(maxFileBytes, signal);
    view.file(file.name, file.size);
    const info = await push(0, utf8.encode(JSON.stringify({name: file.name, size: file.size})));
    await exchange(0, 1, () => send([info]));
    let sent = 0;
    await batches(Math.ceil(file.size / chunkBytes), async (first, count) => {
      const start = (first - 1) * chunkBytes;
      const data = new Uint8Array(await file.slice(start, start + count * chunkBytes).arrayBuffer());
      const packets = await Promise.all(Array.from({length: count}, (_, i) =>
        push(first + i, data.subarray(i * chunkBytes, (i + 1) * chunkBytes))));
      await exchange(first, count, (missing) => send(missing.map((index) => packets[index - first])));
      sent += data.length;
      view.progress(sent);
    });
    view.sent();
    return;
  }
  const {name, size} = metadata;
  if (typeof name !== 'string' || !Number.isInteger(size) || size < 0 || size > maxFileBytes) {
    throw Error('Invalid file metadata');
  }
  view.file(name, size);
  const count = Math.ceil(size / chunkBytes);
  const chunks = new Array(count);
  let received = 0;
  await batches(count, async (first, length) => {
    const batch = await pull(first, length);
    for (let i = 0; i < batch.length; i++) {
      const offset = first + i - 1;
      const chunk = batch[i];
      if (chunk.length !== Math.min(chunkBytes, size - offset * chunkBytes)) throw Error('Invalid chunk size');
      chunks[offset] = chunk;
      received += chunk.length;
    }
    view.progress(received);
  });
  const completion = join([4], await cipher.encrypt(completeContext, new Uint8Array()));
  await exchange(completeIndex, 1, () => send([completion]));
  const download = document.createElement('a');
  download.href = URL.createObjectURL(new Blob(chunks, {type: 'application/octet-stream'}));
  download.download = name;
  view.ready(download);
}
