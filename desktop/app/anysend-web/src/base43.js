// QR alphanumeric characters that survive unchanged in a URL fragment.
export const alphabet = '0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ$*+-./:';
const radix = 43n;

export function encode43(bytes) {
  // A leading hexadecimal digit preserves zero bytes without a length prefix.
  let integer = BigInt('0x1' + Buffer.from(bytes).toString('hex'));
  let encoded = '';
  while (integer) {
    encoded = alphabet[Number(integer % radix)] + encoded;
    integer /= radix;
  }
  return encoded;
}

export function decode43(encoded) {
  let integer = 0n;
  for (const digit of encoded) {
    integer = integer * radix + BigInt(alphabet.indexOf(digit));
  }
  return Uint8Array.from(integer.toString(16).slice(1).match(/../g) ?? [], (byte) => Number('0x' + byte));
}
