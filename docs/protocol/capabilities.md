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
| `sms.read` | Lists conversations and messages ([messages](sms.md)) | SMS permission |
| `sms.send` | Sends texts when a PC asks ([messages](sms.md)) | SMS permission |
| `call.state` | Reports incoming and ongoing calls ([calls](calls.md)) | phone permission |
| `call.control` | Answers, declines and silences calls when a PC asks ([calls](calls.md)) | phone permission |
| `call.incall` | Mutes, holds and presses keys on the call in progress ([calls](calls.md)) | Elevated, Android 12+ |
| `call.log` | Lists recent calls ([calls](calls.md)) | phone permission |
| `call.dial` | Places a call or opens the dialer when a PC asks ([calls](calls.md)) | phone permission |
| `contacts.read` | Lists and searches contacts ([contacts](contacts.md)) | contacts permission |
| `clip.write` | Accepts clipboard content from the PC ([clipboard service](clipboard.md)) | app update |
| `clip.share` | Sends the clipboard when the user taps "Send" | app update |
| `clip.read.auto` | Sends clipboard changes automatically, in the background | Elevated |
| `photos.read` | Announces new photos and screenshots, lists albums and items, serves thumbnails, and sends files on request ([photos](photos.md)) | photos permission |
| `mirror.capture` | Streams its screen (with the system's capture prompt at Basic; [mirroring](mirror.md)) | app update |
| `mirror.input` | Accepts touch, key and scroll input while mirrored | Assist |
| `mirror.virtual_display` | Runs apps on displays of their own, shown in windows on the PC ([mirroring](mirror.md)) | Elevated, Android 11+ |
| `mirror.audio` | Streams all audio while mirrored | Elevated |
| `mirror.audio.playback` | Streams audio from apps that allow playback capture | app update |
| `toggles.read` | Shares quick settings state ([toggles](toggles.md)) | app update |
| `toggles.ringer` | Changes the ringer mode between `ring` and `vibrate` ([toggles](toggles.md)) | app update |
| `toggles.volume` | Changes the media volume ([toggles](toggles.md)) | app update |
| `toggles.flashlight` | Turns the flashlight on or off ([toggles](toggles.md)) | hardware flash unit |
| `toggles.dnd` | Turns Do Not Disturb on or off and sets the ringer to `silent` ([toggles](toggles.md)) | Do Not Disturb access or Elevated |
| `toggles.brightness` | Changes the screen brightness ([toggles](toggles.md)) | Modify system settings or Elevated |
| `toggles.wifi` | Turns Wi-Fi on or off ([toggles](toggles.md)) | Elevated |
| `toggles.bluetooth` | Turns Bluetooth on or off ([toggles](toggles.md)) | Elevated |
| `camera.stream` | Streams a camera for webcam use | camera permission |

## Desktop

| ID | Meaning | Unlocked by |
|---|---|---|
| `pc.power` | Locks or sleeps on request ([actions](actions.md)) | app update |
| `pc.wake` | Shares network adapter addresses (`pc.wake_info`) so a paired phone can wake it with Wake-on-LAN ([actions](actions.md)) | app update |
| `photos.show` | Shows a phone's new photos and gallery ([photos](photos.md)) | app update |
| `call.show` | Shows a phone's calls, recent calls and dialer ([calls](calls.md)) | app update |
| `contacts.show` | Shows a phone's contacts ([contacts](contacts.md)) | app update |
| `sms.show` | Shows a phone's text messages ([messages](sms.md)) | app update |
| `toggles.show` | Shows a phone's quick settings and sends toggle changes ([toggles](toggles.md)) | app update |
| `mirror.view` | Shows a phone's screen ([mirroring](mirror.md)) | app update |
| `mirror.listen` | Plays a mirrored phone's sound ([mirroring](mirror.md)) | app update |
| `files.browse` | Lets a paired phone browse and fetch its files | app update |
| `input.inject` | Accepts pointer, keyboard and presentation input ([remote input](remote.md)) | app update |
| `recorder` | Saves and converts voice recordings from a phone ([recorder](recorder.md)) | app update |
| `deck.actions` | Runs Deck actions | app update |
| `addon.vcam` | The virtual camera add-on is installed | installing the add-on |
