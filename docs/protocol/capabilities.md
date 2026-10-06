# Capability registry

Capability IDs a device may announce in `hello.caps` and `hello.update`
(protocol §5–6). A device announces a capability only when it can provide it
**right now**: with its current power level, permissions, add-ons and OS
version.

IDs are lowercase dotted strings (`[a-z0-9._-]`, at most 64 bytes). Receivers
ignore malformed IDs, IDs they don't know, and anything beyond 256 entries.
New IDs may be added in any release; an ID is never reused with a different
meaning.

The capability matrix (`docs/architecture/capabilities.md`) turns these into
feature states. "Unlocked by" is what the matrix offers the user when the
capability is missing.

## Both devices

| ID | Meaning | Unlocked by |
|---|---|---|
| `core.ping` | Answers `ping` | always offered |
| `device.battery` | Sends `event.battery` | always offered |
| `device.ring` | Rings on `device.ring`, even on silent | always offered |
| `files.transfer` | Sends and receives files ([files service](files.md)) | app update |
| `link.open` | Opens web links sent to it ([actions](actions.md)) | app update |
| `clip.image` | Accepts images on its clipboard ([clipboard service](clipboard.md)) | app update |
| `media.control` | Shares its media players for remote control ([media service](media.md); phone: needs notification access) | phone: notification access · desktop: app update |
| `media.remote` | Shows and controls other devices' media players | app update |

## Phone

| ID | Meaning | Unlocked by |
|---|---|---|
| `notify.mirror` | Forwards notifications ([notifications service](notifications.md)) | notification access |
| `notify.reply` | Accepts inline replies and actions on notifications | notification access |
| `notify.sensitive` | Forwards notifications Android hides as sensitive (e.g. one-time codes) | Elevated |
| `sms.read` | Reads SMS conversations | SMS permission |
| `sms.send` | Sends SMS | SMS permission |
| `call.state` | Reports incoming and ongoing calls | phone permission |
| `clip.write` | Accepts clipboard content from the PC ([clipboard service](clipboard.md)) | app update |
| `clip.share` | Sends the clipboard when the user taps "Send" | app update |
| `clip.read.auto` | Sends clipboard changes automatically, in the background | Elevated |
| `photos.read` | Lists and sends recent photos and screenshots | photos permission |
| `mirror.capture` | Streams its screen (with the system's capture prompt at Basic) | app update |
| `mirror.input` | Accepts touch, key and scroll input while mirrored | Assist |
| `mirror.virtual_display` | Runs apps on a separate virtual display (app windows) | Elevated |
| `mirror.audio` | Streams all audio while mirrored | Elevated |
| `mirror.audio.playback` | Streams audio from apps that allow playback capture | app update |
| `camera.stream` | Streams a camera for webcam use | camera permission |

## Desktop

| ID | Meaning | Unlocked by |
|---|---|---|
| `pc.power` | Locks or sleeps on request ([actions](actions.md)) | app update |
| `files.browse` | Lets a paired phone browse and fetch its files | app update |
| `input.inject` | Accepts pointer and keyboard input (touchpad, air mouse, voice typing) | app update |
| `deck.actions` | Runs Deck actions | app update |
| `addon.vcam` | The virtual camera add-on is installed | installing the add-on |
