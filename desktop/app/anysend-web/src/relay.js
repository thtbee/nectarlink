import {join} from './bytes.js';

export function connectRelay(url, identity, receive, challengeHash) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url, 'iroh-relay-v2');
    socket.binaryType = 'arraybuffer';
    const send = (frame) => socket.send(frame);
    const timeout = setTimeout(() => {
      reject(Error('Relay timeout'));
      socket.close();
    }, 10000);
    socket.onerror = socket.onclose = () => {
      clearTimeout(timeout);
      reject(Error('Relay offline'));
    };
    socket.onmessage = ({data}) => {
      const frame = new Uint8Array(data);
      switch (frame[0]) {
        case 0: {
          if (frame.length !== 17) {
            reject(Error('Invalid relay challenge'));
            socket.close();
            break;
          }
          // Tag, raw public key, postcard byte-string length, signature.
          const signature = identity.sign(challengeHash(frame.subarray(1)));
          send(join([1], identity.publicKey, [64], signature));
          break;
        }
        case 2:
          clearTimeout(timeout);
          resolve({
            send: (peer, payload) => send(join([4], peer, [0], payload)),
            close: () => socket.close(),
          });
          break;
        case 3:
          clearTimeout(timeout);
          reject(Error('Relay denied'));
          socket.close();
          break;
        case 6:
          if (frame.length > 34) receive(frame.subarray(1, 33), frame.subarray(34));
          break;
        case 9:
          frame[0] = 10;
          send(frame);
          break;
      }
    };
  });
}
