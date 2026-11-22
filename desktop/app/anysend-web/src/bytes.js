export const utf8 = new TextEncoder();

export function join(...parts) {
  const bytes = new Uint8Array(parts.reduce((length, part) => length + part.length, 0));
  let offset = 0;
  for (const part of parts) {
    bytes.set(part, offset);
    offset += part.length;
  }
  return bytes;
}

export const equal = (left, right) =>
  left.length === right.length && left.every((byte, i) => byte === right[i]);
