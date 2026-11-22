// SPDX-License-Identifier: GPL-3.0-or-later
import {gcm} from '@noble/ciphers/aes.js';
import {join, utf8} from './bytes.js';
import {batchSize, chunkBytes, indexBytes, maxFileBytes, readIndex, requestContext} from './file-protocol.js';

const uploadContext = utf8.encode('AnySend file upload v1');
const errorIndex = 0xffffffff;
const text = new TextDecoder();

function encrypt(key, aad, bytes) {
  const iv = crypto.getRandomValues(new Uint8Array(12));
  return join(iv, gcm(key, iv, aad).encrypt(bytes));
}

export async function transferFile(send, listen, key, view) {
  function exchange(requests, first, count) {
    return new Promise((resolve, reject) => {
      const chunks = new Array(count);
      let remaining = count;
      const timer = setTimeout(() => fail(Error('File transfer timed out')), 25000);
      function fail(error) {
        clearTimeout(timer);
        reject(error);
      }
      listen((packet) => {
        if (packet.length < 32) return;
        const index = readIndex(packet);
        const offset = index - first;
        if (index !== errorIndex && (offset < 0 || offset >= count || chunks[offset])) return;
        try {
          const data = gcm(key, packet.subarray(4, 16), packet.subarray(0, 4)).decrypt(packet.subarray(16));
          if (index === errorIndex) {
            fail(Error(text.decode(data)));
            return;
          }
          chunks[offset] = data;
          if (--remaining === 0) {
            clearTimeout(timer);
            resolve(chunks);
          }
        } catch {
          fail(Error('File authentication failed'));
        }
      });
      try {
        for (const request of requests) Promise.resolve(send(request)).catch(fail);
      } catch (error) {
        fail(error);
      }
    });
  }

  function pull(first, count) {
    return exchange([encrypt(key, requestContext, indexBytes(first))], first, count);
  }

  function push(index, bytes) {
    const header = indexBytes(index);
    return join([2], header, encrypt(key, join(uploadContext, header), bytes));
  }

  view.waiting();
  const [response] = await pull(0, 1);
  const metadata = JSON.parse(text.decode(response));
  if (metadata.direction === 'upload') {
    const file = await view.chooseFile(maxFileBytes);
    view.file(file.name, file.size);
    await exchange([push(0, utf8.encode(JSON.stringify({name: file.name, size: file.size})))], 0, 1);
    const count = Math.ceil(file.size / chunkBytes);
    for (let first = 1; first <= count; first += batchSize) {
      const end = Math.min(count + 1, first + batchSize);
      const packets = [];
      for (let index = first; index < end; index++) {
        const start = (index - 1) * chunkBytes;
        const content = new Uint8Array(await file.slice(start, start + chunkBytes).arrayBuffer());
        packets.push(push(index, content));
      }
      await exchange(packets, first, end - first);
      view.progress(Math.min(file.size, (end - 1) * chunkBytes));
    }
    view.sent();
    return;
  }
  const {name, size} = metadata;
  if (typeof name !== 'string' || !Number.isInteger(size) || size < 0 || size > maxFileBytes) {
    throw Error('Invalid file metadata');
  }
  view.file(name, size);
  const chunks = [];
  const count = Math.ceil(size / chunkBytes);
  let received = 0;
  for (let first = 1; first <= count; first += batchSize) {
    const batch = await pull(first, Math.min(batchSize, count - first + 1));
    for (const chunk of batch) {
      if (chunk.length !== Math.min(chunkBytes, size - received)) throw Error('Invalid chunk size');
      chunks.push(chunk);
      received += chunk.length;
    }
    view.progress(received);
  }
  const download = document.createElement('a');
  download.href = URL.createObjectURL(new Blob(chunks, {type: 'application/octet-stream'}));
  download.download = name;
  view.ready(download);
}
