import {join} from './bytes.js';

export function connectRelay(url, identity, receive, challengeHash, signal, disconnect) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url, 'iroh-relay-v2');
    socket.binaryType = 'arraybuffer';
    const send = (frame) => socket.send(frame);
    let connected = false;
    function fail(error) {
      clearTimeout(timeout);
      signal.removeEventListener('abort', abort);
      socket.onerror = socket.onclose = null;
      socket.close();
      if (connected) disconnect(error);
      else reject(error);
    }
    const abort = () => fail(signal.reason);
    const timeout = setTimeout(() => fail(Error('Relay timeout')), 10000);
    signal.addEventListener('abort', abort, {once: true});
    socket.onerror = socket.onclose = () => fail(Error('Relay offline'));
    socket.onmessage = ({data}) => {
      const frame = new Uint8Array(data);
      switch (frame[0]) {
        case 0: {
          if (frame.length !== 17) {
            fail(Error('Invalid relay challenge'));
            break;
          }
          // Tag, raw public key, postcard byte-string length, signature.
          const signature = identity.sign(challengeHash(frame.subarray(1)));
          send(join([1], identity.publicKey, [64], signature));
          break;
        }
        case 2:
          clearTimeout(timeout);
          connected = true;
          resolve({
            send: (peer, payload) => send(join([4], peer, [0], payload)),
          });
          break;
        case 3:
          fail(Error('Relay denied'));
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
