// SPDX-License-Identifier: GPL-3.0-or-later
import {utf8} from './bytes.js';

export const lanChunkBytes = 256 * 1024;
export const relayChunkBytes = 60 * 1024;
export const batchSize = 8;
export const maxFileBytes = 128 * 1024 * 1024;
export const requestContext = utf8.encode('AnySend file request v2');
export const uploadContext = utf8.encode('AnySend file upload v2');
export const completeContext = utf8.encode('AnySend file complete v2');
export const completeIndex = 0xfffffffe;
export const errorIndex = 0xffffffff;

export function indexBytes(index) {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, index);
  return bytes;
}

export const readIndex = (bytes) =>
  new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0);
