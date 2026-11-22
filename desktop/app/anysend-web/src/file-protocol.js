export const chunkBytes = 48 * 1024;
export const batchSize = 16;
export const maxFileBytes = 128 * 1024 * 1024;
export const requestContext = new TextEncoder().encode('AnySend file request v1');

export function indexBytes(index) {
  const bytes = new Uint8Array(4);
  new DataView(bytes.buffer).setUint32(0, index);
  return bytes;
}

export const readIndex = (bytes) =>
  new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength).getUint32(0);
