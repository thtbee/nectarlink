import {join} from './bytes.js';

export function connectRelay(url, identity, receive, challengeHash, signal, disconnect, peerGone) {
  signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const socket = new WebSocket(url, 'iroh-relay-v2');
    socket.binaryType = 'arraybuffer';
    function send(frame) {
      if (socket.readyState !== WebSocket.OPEN) throw Error('Relay offline');
      socket.send(frame);
    }
    let connected = false;
    let idle;
    function fail(error) {
      clearTimeout(timeout);
      clearTimeout(idle);
      signal.removeEventListener('abort', abort);
      socket.onerror = socket.onclose = socket.onmessage = null;
      socket.close();
      if (connected) disconnect(error);
      else reject(error);
    }
    const abort = () => fail(signal.reason);
    const timeout = setTimeout(() => fail(Error('Relay timeout')), 20000);
    signal.addEventListener('abort', abort, {once: true});
    socket.onerror = socket.onclose = () => fail(Error('Relay offline'));
    socket.onmessage = ({data}) => {
      clearTimeout(idle);
      idle = setTimeout(() => fail(Error('Relay stopped responding')), 45000);
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
        case 8:
          if (frame.length === 33) peerGone(frame.subarray(1));
          break;
        case 9:
          frame[0] = 10;
          send(frame);
          break;
      }
    };
  });
}
