// SPDX-License-Identifier: GPL-3.0-or-later
import nacl from 'tweetnacl';
import {hmac} from '@noble/hashes/hmac.js';
import {sha256} from '@noble/hashes/sha2.js';
import {equal, join, utf8} from './bytes.js';
import {connectRelay} from './relay.js';

const probeContext = utf8.encode('AnySend relay probe v3');
const probePacket = 5;
const routeIdle = 12000;
const average = (previous, sample) => previous ? 0.75 * previous + 0.25 * sample : sample;

export function createRelayPool(urls, verifyPaths, identity, peer, seed, receive, challengeHash, signal) {
  const watchers = new Set();
  const maxBatches = verifyPaths ? 4 : 16;
  const paths = urls.map((url) => ({
    url, ready: false, connection: null, batches: 0, pending: 0, rtt: 0, chunkTime: 0,
    lastSeen: 0, lastProgress: 0, penaltyUntil: 0, used: false,
  }));

  let notifying = false;
  function notifyWatchers() {
    if (notifying) return;
    notifying = true;
    queueMicrotask(() => {
      notifying = false;
      for (const notify of watchers) notify();
    });
  }

  function setReady(path, ready) {
    if (path.ready === ready) return;
    path.ready = ready;
    notifyWatchers();
  }

  async function maintain(path) {
    let timer, retry;
    let probes = [];
    let delay = 1000;
    function disconnect() {
      clearTimeout(timer);
      path.connection = null;
      setReady(path, false);
      if (!signal.aborted) {
        retry = setTimeout(open, delay);
        delay = Math.min(2 * delay, 15000);
      }
    }

    function probe() {
      if (!path.connection || signal.aborted) return;
      const now = performance.now();
      if (now - path.lastSeen >= routeIdle) setReady(path, false);
      const challenge = crypto.getRandomValues(new Uint8Array(32));
      probes = probes.filter(({started}) => now - started < 6000);
      probes.push({challenge, started: now});
      try {
        const tag = hmac(sha256, seed, join(probeContext, peer, identity.publicKey, challenge));
        path.connection.send(peer, join([probePacket], challenge, tag));
      } catch {
        // The socket's close handler schedules the reconnect.
      }
      timer = setTimeout(probe, path.ready ? 2500 : 750);
    }

    function onPacket(sender, packet) {
      if (!equal(sender, peer)) return;
      if (verifyPaths && packet.length === 97 && packet[0] === probePacket) {
        const challenge = packet.subarray(1, 33);
        const index = probes.findIndex((probe) => equal(probe.challenge, challenge));
        if (index >= 0 &&
            nacl.sign.detached.verify(join(probeContext, challenge), packet.subarray(33), peer)) {
          path.lastSeen = performance.now();
          path.rtt = average(path.rtt, path.lastSeen - probes[index].started);
          probes.splice(0, index + 1);
          setReady(path, true);
        }
        return;
      }
      receive(packet, path);
    }

    async function open() {
      if (signal.aborted) return;
      try {
        path.connection = await connectRelay(path.url, identity, onPacket, challengeHash, signal,
          disconnect, (sender) => {
            if (verifyPaths && equal(sender, peer)) {
              probes = [];
              path.lastSeen = 0;
              setReady(path, false);
              clearTimeout(timer);
              probe();
            }
          });
      } catch {
        disconnect();
        return;
      }
      if (signal.aborted) return;
      delay = 1000;
      probes = [];
      path.used = false;
      path.lastSeen = performance.now();
      if (verifyPaths) probe();
      else setReady(path, true);
    }

    signal.addEventListener('abort', () => {
      clearTimeout(timer);
      clearTimeout(retry);
    }, {once: true});
    await open();
  }

  function watch(notify) {
    watchers.add(notify);
    return () => watchers.delete(notify);
  }

  function waitForPath(waitSignal) {
    waitSignal.throwIfAborted();
    if (paths.some((path) => path.ready)) return;
    return new Promise((resolve, reject) => {
      const unwatch = watch(() => {
        if (paths.some((path) => path.ready)) {
          cleanup();
          resolve();
        }
      });
      function cleanup() {
        unwatch();
        waitSignal.removeEventListener('abort', abort);
      }
      function abort() {
        cleanup();
        reject(waitSignal.reason);
      }
      waitSignal.addEventListener('abort', abort, {once: true});
    });
  }

  function cost(path, chunks, now) {
    const penalty = path.penaltyUntil > now ? 2 : 1;
    return penalty * ((path.rtt || 250) + (path.pending + chunks) * (path.chunkTime || 75));
  }

  function pick(chunks, excluded, preferUnused = false) {
    const now = performance.now();
    let best;
    for (const path of paths) {
      if (!path.ready || path === excluded || path.batches >= maxBatches) continue;
      if (best && preferUnused && path.used !== best.used) {
        if (!path.used) best = path;
      } else if (!best || cost(path, chunks, now) < cost(best, chunks, now)) {
        best = path;
      }
    }
    return best;
  }

  function claim(packets) {
    for (const path of paths) {
      if (!path.ready) continue;
      try {
        for (const packet of packets) path.connection.send(peer, packet);
      } catch {
        // Other ready routes may still carry the claim.
      }
    }
  }

  function reserve(chunks, previous, measure) {
    const path = pick(chunks, previous, measure) ?? pick(chunks, null, measure);
    if (!path) return null;
    const connection = path.connection;
    const started = performance.now();
    const ahead = path.pending;
    let lastAck = started, acknowledged = 0;
    if (measure) path.used = true;
    path.batches++;
    path.pending += chunks;
    return {
      path,
      get online() { return path.ready && path.connection === connection; },
      send(packets) {
        if (!this.online) throw Error('Relay offline');
        for (const packet of packets) connection.send(peer, packet);
      },
      ack(source) {
        if (!source) return;
        const now = performance.now();
        source.lastSeen = now;
        source.lastProgress = now;
        if (source === path) {
          lastAck = now;
          acknowledged++;
        }
      },
      retry(missing, backoff) {
        if (!this.online) return true;
        const now = performance.now();
        const alternate = pick(missing, path);
        const rtt = path.rtt || 250;
        if (alternate && now - started >= Math.max(2000, (alternate.rtt || 250) * 4)) {
          const remaining = Math.max(0, path.pending - chunks);
          const sample = acknowledged ? (lastAck - started - rtt) / (ahead + acknowledged) : 0;
          const estimate = rtt + (remaining + missing) * Math.max(sample, path.chunkTime || 75);
          const otherCost = cost(alternate, missing, now);
          const stalled = now - lastAck >= 2000;
          const slow = acknowledged >= 2 && sample > 2 * (alternate.chunkTime || 75);
          if ((stalled || slow) && estimate > 2 * otherCost ||
              (now - Math.max(started, path.lastProgress) >= 3000 && otherCost < now - lastAck)) {
            path.penaltyUntil = now + 8000;
            return true;
          }
        }
        return now - Math.max(started, path.lastProgress) >= Math.max(backoff, 4 * rtt) ||
          now - lastAck >= Math.max(30000, backoff, 2 * cost(path, 0, now));
      },
      release(complete) {
        path.batches--;
        path.pending -= chunks;
        if (measure && complete && acknowledged === chunks) {
          const sample = Math.max(1, lastAck - started - path.rtt) / (ahead + chunks);
          path.chunkTime = average(path.chunkTime, sample);
        }
        notifyWatchers();
      },
    };
  }

  for (const path of paths) void maintain(path);
  return {size: paths.length, waitForPath, watch, claim, reserve};
}
