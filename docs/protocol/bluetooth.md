# Bluetooth link

When a phone and a PC reach each other no other way (no shared network, no
mobile data, no cable), the PC can carry the session over a Bluetooth link to
a phone it's bonded with. Nothing new goes over the air: the link carries the
same QUIC session, with the same TLS identities, as the USB tunnel does.

## Who does what

- The phone listens for RFCOMM links on service UUID
  `a3f6b0e2-5c1d-4b7e-9e2a-7d4c1f8b6e53` (secure RFCOMM, so the PC has to be
  bonded in the phone's Bluetooth settings). It pipes each link into a
  loopback TCP port the core opens (`Node::start_bluetooth_listener`).
- The PC connects to that service on phones it's bonded with, only while the
  setting is on and a paired phone is offline (`Node::bluetooth_wanted`), and
  exposes the link on a loopback TCP port it hands to
  `Node::connect_bluetooth`. Windows uses Winsock RFCOMM (`AF_BTH`, the
  service UUID, port 0); Linux registers a BlueZ `Profile1` client for the
  UUID and calls `Device1.ConnectProfile`.

## On the link

The same framing as the USB tunnel ([elevated-usb.md](elevated-usb.md) for the
USB side): an `NLQU` hello each way (magic, `u16` big-endian length, JSON with
`v`, `id`, `name` and `usb_only`, always false here), then QUIC datagrams as `u16` big-endian length-prefixed
frames. Each side bridges the frames to a loopback UDP socket for its QUIC
endpoint.

## Rules

- Only devices already paired in NectarLink, with reconnecting on: an
  unpaired peer's hello ends the link, and nothing pairs over it. A Bluetooth
  bond never stands in for NectarLink's pairing.
- A USB tunnel wins: no link is set up beside one, and a cable replaces a link.
- The connection uses any other path it finds (the local network) as soon as
  that works; the session reports `ConnectionPath::Bluetooth` only while the
  link is the path in use.
- While on Bluetooth, only light messages go either way. No data streams, no
  QUIC datagrams, and none of the `mirror.`, `photos.`, `storage.`,
  `webcam.`, `camera.`, `pointer.`, `mic.`, `stream.`, `files.`, `audio.` or
  `elevated.` messages, `media.watch`, `media.file`, `call.audio.*` or
  `sms.part`. A request for one is answered with `unsupported`.
