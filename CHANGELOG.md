# Changelog

All notable changes to NectarLink. Versions follow
[semantic versioning](https://semver.org); before 1.0, a minor version may
change things that need a step from you, and its notes say so.

## 0.1.6 (unreleased)

### New

- **Test call audio.** When someone on a call can't hear you properly, **Settings › Calls through
  this PC › Test call audio** (Windows and macOS) opens a fake call, marked as a test, that runs the
  sound of a call with no call going on. With your phone connected, your microphone goes to the
  phone, plays on its speaker and comes back to this PC's output (the phone needs the app version
  with this; older ones, or no phone, get a 5-second record-and-play-back on this PC instead, and
  the test says which ran). It shows the microphone it records from, the level meter, clipping, how
  many pieces of sound were sent, dropped, late or lost, and the round trip. See
  [Testing the sound without a call](docs/guide/calls.md#testing-the-sound-without-a-call).
- **Nectar Island on the Mac.** The pill for calls, transfers and new notifications sits just
  below the menu bar, on every Space, and stays when another app is in front or NectarLink has no
  window open. It doesn't show over full-screen apps.
- **Swipe between your devices on the phone's Home.** With two or more paired devices, their cards
- **Talk on a phone call through the Mac.** The **On PC** button on a call now works on macOS as
  on Windows: the other party plays on the Mac's sound output, headphones included, and the Mac's
  microphone goes into the call. macOS asks once to let NectarLink use the microphone. Pick the
  microphone in Settings › Camera & Audio. Needs Elevated on an Android 13 or newer phone, as
  on Windows.
- **Swipe between your devices on the phone's Home.** With two or more paired devices, their cards
  sit side by side, PCs first: swipe to go from one to the next, with dots for where you are.
  Thanks to [@kedar284](https://github.com/kedar284) ([#651](https://github.com/thtbee/nectarlink/issues/651)).
- **Use your phone as a touchpad and keyboard for your tablet.** A paired tablet (or another phone)
  with Elevated shows **Control** on its card on the phone's Home: the touchpad moves a pointer on
  the tablet, clicks and scrolls, and the keyboard types there, any language and emoji included.
  The tablet asks the first time; **Let it control this device** in its Settings › Devices turns
  it off again. Thanks to [@kedar284](https://github.com/kedar284) ([#651](https://github.com/thtbee/nectarlink/issues/651)).
- **New settings say so.** After an update, the settings it added have a **New** pill, and so
  does their section in the PC's Settings, until you've looked at that section. A new install
  starts without them. Thanks to [@jed84](https://github.com/jed84) ([#656](https://github.com/thtbee/nectarlink/issues/656)).
- **Give your PC and phone a name of their own.** In Settings › General on the PC (**This PC's
  name**) and Settings › Devices on the phone (**This phone's name**). Paired devices show it at once;
  leave it empty to go back to the computer's or Android's name. Thanks to
  [@zubair1511983](https://github.com/zubair1511983) ([#648](https://github.com/thtbee/nectarlink/issues/648)).
- **Now playing shows the time you want.** The time on the right of Now playing on the PC's Home
  can show what's left, the whole length, or both: click it to switch, or pick in Settings ›
  General. Thanks to [@jed84](https://github.com/jed84) ([#658](https://github.com/thtbee/nectarlink/issues/658)).
- **One mouse and keyboard for two PCs.** Set another paired PC **Beside this PC** on the left
  or right, and the mouse goes over that edge of the screen onto it, with the keyboard typing
  there, no phone needed. The other PC asks first. From Windows, to Windows or Linux. Thanks to
  [@ManuZhu0728](https://github.com/ManuZhu0728) ([#432](https://github.com/thtbee/nectarlink/discussions/432)).
- **Now playing on the phone.** One card on the phone's Home with every player on your PCs and
  on the phone, a page each to swipe through, with play and pause, previous and next, the
  position, and the volume and output of where it plays. Thanks to
  [@doodoobug-dot](https://github.com/doodoobug-dot) ([#536](https://github.com/thtbee/nectarlink/issues/536)).
- **Stay connected over Bluetooth when there's no network (Windows, Linux, Android).**
  Pair the phone and the PC in the system's Bluetooth settings once, then turn on
  **Bluetooth when there's no network** on both (Settings › Connection on the PC,
  Settings on the phone). With no Wi-Fi, data or cable between them, they keep
  notifications, texts, calls (without their sound), clipboard text and quick settings
  going over Bluetooth, and switch back to Wi-Fi when it's there. Thanks to
  [@unaibenidorm](https://github.com/unaibenidorm) ([#313](https://github.com/thtbee/nectarlink/issues/313))
  and [@lukee12zakk](https://github.com/lukee12zakk) ([#182](https://github.com/thtbee/nectarlink/issues/182)).
- **Read and answer replies to your reports in the app.** When someone answers a problem report
  you sent, the notification shows what they wrote, and you can reply right in it. **Your
  reports**, in Settings under Report a problem on the PC and on the phone's Report a problem
  screen, shows each report with its answers and a field to reply, without a GitHub account.
  Crash reports from the crash card are followed too, and a closed report stays in the list.
- **The mirroring toolbar can sit beside the picture.** **Settings › General › Toolbar beside
  the picture** gives it a strip of the window above or below the phone's picture, so it never
  covers the app's own buttons ([#549](https://github.com/thtbee/nectarlink/issues/549)).
- **A one-color tray icon on Linux.** **Settings › General › Tray icon › One color** now works
  on Linux too. Desktops that recolor tray icons, like KDE Plasma, show it in the panel's
  colors ([#555](https://github.com/thtbee/nectarlink/issues/555)).
- **Updates get their own place in PC Settings.** **Settings › Updates** has the version, the
  check and when it last happened, automatic checks and installs, the channel and What's new.
  Connection & About keeps the version, logs and reporting a problem
  ([#558](https://github.com/thtbee/nectarlink/issues/558)).
- **Turn off the anonymous daily count.** **Send an anonymous daily count** in Settings, under
  Storage & Privacy on the PC and About on the phone, stops it. On a PC,
  `NECTARLINK_NO_DAILY_COUNT=1` turns it off for packagers
  ([#534](https://github.com/thtbee/nectarlink/issues/534)).
- **The PC's Wi-Fi and Bluetooth on the phone.** The PC's card on Home and the top of Deck
  show whether they're on, and a tap switches them. Turning Wi-Fi off asks first, since the
  phone may reach the PC over it. Windows and Linux. Thanks to
  [@M-P-A](https://github.com/M-P-A) ([#636](https://github.com/thtbee/nectarlink/issues/636)).
- **On / off tiles in Deck.** One tile with two actions, one to turn something on and one to
  turn it off, run in turn with each press. The tile on the phone stays lit while it's on, like
  the mute tiles, and the PC remembers it after a restart. Thanks to
  [@M-P-A](https://github.com/M-P-A) ([#640](https://github.com/thtbee/nectarlink/issues/640)).
- **Deck macros, and Restart and Shut down tiles.** A **Macro** tile runs up to 16 actions in a
  row, with a wait of up to 10 seconds after each. **Restart PC** and **Shut down PC** tiles
  ask on the phone first. A macro with a shell command needs **Run Deck shell commands**, like a
  command tile ([#490](https://github.com/thtbee/nectarlink/issues/490)).
- **Sharing the PC's screen to the phone is lighter on Windows.** The screen is captured,
  converted and encoded on the graphics card, without a copy through the CPU for each frame.
  PCs that can't do that keep the old way. Thanks to [@aihehye](https://github.com/aihehye)
  ([#593](https://github.com/thtbee/nectarlink/pull/593)).
- **Mirroring on Windows uses much less CPU.** The phone's picture now stays on the graphics
  card from decoding to the screen, instead of being copied through the CPU for every frame.
  PCs that can't do that keep the old way. Thanks to [@aihehye](https://github.com/aihehye)
  ([#585](https://github.com/thtbee/nectarlink/pull/585)).
- **Unlock the PC from the phone.** With unlock with phone set up, **Unlock PC** on a Windows
  PC's card on the phone brings up the fingerprint prompt for its lock screen, so you don't
  have to pick NectarLink on the PC first. It works only for a phone the PC unlocks with
  already, on the same network, and only while the PC is locked. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#565](https://github.com/thtbee/nectarlink/pull/565)).
- **Select photos with a box.** In Photos on the PC, while you're selecting, drag from a photo
  to draw a box: everything it covers is added, and the grid scrolls when you reach its edge.
  Ctrl or Shift and a drag does it any time; dragging a selected photo still drags it out
  ([#587](https://github.com/thtbee/nectarlink/issues/587)).
- **Ukrainian.** NectarLink speaks Українська on the phone and the PC. Language (Settings ›
  Appearance on the phone, Settings › General on the PC) picks it, or the system language does. Thanks to
  [@Lik1ch](https://github.com/Lik1ch).
- **Zoom into a photo on the PC.** In the Photos viewer the mouse wheel zooms around the
  pointer, a double click goes between fitting the window and a close look, and a zoomed photo
  can be dragged around. It shows the full photo, so screenshots stay sharp up close
  ([#587](https://github.com/thtbee/nectarlink/issues/587)).
- **The phone's sound has its own volume on the PC.** **Phone sound on this PC** on the PC's
  Home has a slider for how loud it plays there, apart from the PC's own volume, and it applies
  to a mirrored phone's sound too. While the phone's sound plays on the PC, the phone's volume
  keys move that slider instead of turning the phone's speaker back up, and the phone's volume
  panel shows it ([#577](https://github.com/thtbee/nectarlink/issues/577)).
- **Restart the PC from the phone.** A Windows PC's card on the phone's Home has **Restart**
  next to **Shut down**, with the same confirmation; an app with unsaved work can still stop
  it. Thanks to [@w-rakeeb](https://github.com/w-rakeeb)
  ([#574](https://github.com/thtbee/nectarlink/issues/574)).
- **USB only (phone).** **Settings › Devices › USB only**, or a **USB only** Quick Settings
  tile, makes the phone reach PCs only through a USB cable: no dialing or accepting over
  Wi-Fi, mobile data or relays, no local discovery or LocalSend, and the multicast lock is
  released. Other connections close when you turn it on. It's silent while no PC is on the
  cable: the app stops and shows no notification. Plug the cable in and the PC (with USB
  debugging allowed) starts the app through adb, even when the PC app was started with the
  cable already in; the usual notification shows while connected. Asked for in
  [#481](https://github.com/thtbee/nectarlink/discussions/481). Thanks to
  [@farfromrefug](https://github.com/farfromrefug) ([#546](https://github.com/thtbee/nectarlink/pull/546)).
- **Touchpad gestures from the phone.** The phone's touchpad does what a laptop's does: pinch
  to zoom, three fingers up for Task View and down for the desktop, three fingers sideways
  through the app switcher, four fingers sideways between virtual desktops, and three- and
  four-finger taps for Search and notifications. On Windows and Linux.
- **Copy part of a notification.** A notification's **...** menu on the PC's Home shows its
  whole text, and any part of it can be selected and copied with Ctrl+C. Thanks to
  [@korry73](https://github.com/korry73) ([#526](https://github.com/thtbee/nectarlink/issues/526)).
- **Mark texts read on the phone.** **Settings › General › Mark texts read on the phone** (off
  by default) marks an SMS conversation read on the phone too when you read it on the PC, so
  the phone's unread count and notification match instead of keeping texts you've already read
  as unread. It presses the messaging app's own **Mark as read** button on the conversation's
  notification (so the app's own state and notification follow), and dismisses the notification
  when the app has no such button. With **Elevated** it also marks the texts read in the phone's
  messages directly: its helper allows itself the SMS write app-op for that one change and puts
  it back to the mode it had. Thanks to [@farfromrefug](https://github.com/farfromrefug)
  ([#553](https://github.com/thtbee/nectarlink/pull/553)).
- **The device you're on, and badges, on the PC's left rail.** The device the pages show sits
  at the bottom of the rail, with a dot for whether it's connected; click it to pick another
  (or pair a new one). **Messages** shows how many texts came in that you haven't read on the
  PC, and **Calls** how many calls were missed. A conversation's texts stop counting when you
  open it, and missed calls when you open **Calls**. When other devices have something waiting,
  the device button shows their total, and the list that opens shows each device's own
  number. Only what arrives after a phone first shows here counts, so a phone's older unread
  texts, which only its own messaging app can clear, don't leave a number up. Thanks to
  [@farfromrefug](https://github.com/farfromrefug) ([#552](https://github.com/thtbee/nectarlink/pull/552)).
- **Report a problem.** **Settings › Connection & About › Report a problem** on the PC (and at
  the bottom of the Connection Doctor) puts what you say went wrong together with what helps
  find the cause: versions, the Doctor's checks, how features have been doing, recent crashes
  and the recent log. You read the whole report first and can leave any part out. **Send**
  files it as an issue for you, no GitHub account needed; **Open on GitHub** fills in a new
  issue and copies the report to paste into it. The phone has its own **Report a problem** in
  Settings, and each app asks the other for its details; the other device's log is added only
  when you tap **Share** on it.
- **After a crash**, both apps show a small card offering to send a crash report (where in
  NectarLink it happened and the versions, never the log), once per crash. When a newer version
  already fixes it, the card offers the update instead. **Don't ask about crashes** turns it
  off. You also get a notification when someone replies to a report you sent, or closes it.
- **An anonymous daily count.** Once a day each app sends its version, platform and device
  model, and how features did, with no ID ([what's sent](docs/guide/privacy-and-security.md)). Crashes are recorded on
  both apps now, and the phone keeps its own small log.
- **What's new, in the app.** The sparkle button at the top of the PC app, and on the
  phone's Home, lists what changed since you last looked, in short points by category,
  with a link to the full release notes. A dot on it means something new arrived with an
  update. Thanks to
  [@zzz1465720680](https://github.com/zzz1465720680)
  ([#461](https://github.com/thtbee/nectarlink/issues/461)).
- **Community, with the Discord server.** Settings › Community (formerly Support
  NectarLink) has the [Discord server](https://discord.gg/AA2Dp8PYqf), news on X, the code on
  GitHub and the support page, side by side. Now and then after an update, a small ask to come
  chat on Discord appears too: at most once a month, three times in all, and never again after
  you join.
- **Set up any AI agent with one prompt.** Settings › AI agents no longer lists particular
  apps: name the agent, choose what it may do, then copy the prompt it shows into your agent,
  which adds NectarLink to its own settings and checks that it works. The card says
  **Connected** once it has. **Set up by hand** has the MCP settings, the commands and the HTTP
  request. NectarLink no longer edits other apps' settings files itself.
- **WhatsApp calls from the PC.** On the PC's Calls page, pick **WhatsApp** (or WhatsApp
  Business) under **Call with**, and the call to that contact starts in WhatsApp on the
  phone. If Android won't let NectarLink open WhatsApp right then, a notification on the
  phone starts it with a tap. Thanks to [@kedar284](https://github.com/kedar284)
  ([#434](https://github.com/thtbee/nectarlink/issues/434)).
- **Pick which of the PC's screens the phone shows.** With more than one screen on the PC,
  the PC screen view on the phone has a **Screen** button that moves to the next one, and
  taps land there. Windows only for now. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Dark picture for apps with no dark mode.** The moon button on a mirroring or app
  window swaps light and dark in the picture, so a white map turns dark gray with its
  colors kept. It's remembered for each app.
- **Choose the frame rate for mirroring.** Settings › Camera & Audio › Frame rate: 30, 60,
  90 or 120 for the phone's screen and app windows. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Keep pop-ups paused after a restart.** Settings › Notifications › Keep pop-ups paused
  after a restart makes **Pause pop-ups** last until you resume them. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Each day of Photos starts a row of its own, and the scrollbar shows the date.** Hold
  or hover the scrollbar on the PC's Photos page to see the day you're at, and drag it
  there. Thanks to [@Dipen425](https://github.com/Dipen425)
  ([#388](https://github.com/thtbee/nectarlink/issues/388)) and
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Install an app on the phone from the PC.** The download button on the PC's Apps page
  sends an app file (.apk) to the phone, whose notification then offers **Install**, which
  opens Android's installer. Thanks to [@w-rakeeb](https://github.com/w-rakeeb)
  ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Make Desk clock yours.** **Customize** on Desk clock picks the background (black,
  graphite or your accent color), the clock's font (mono, sans or serif) and what it shows:
  seconds, the date, the PC's media and volume, and the battery of the phone and the PC.
  Thanks to [@xo4x4ww](https://github.com/xo4x4ww)
  ([#380](https://github.com/thtbee/nectarlink/discussions/380)).
- **Look for PCs only where they are, and stop after a while.** In the phone's Settings,
  under Connect over mobile data: **Only on networks where a PC was found** keeps the
  phone from looking for your PCs on other Wi-Fi, and **Stop looking after** 15 minutes, 1
  hour or 4 hours with no PC reachable stops it until the network changes or you open
  NectarLink. A PC can still reach the phone either way. Thanks to
  [@ThePharaohArt](https://github.com/ThePharaohArt)
  ([#397](https://github.com/thtbee/nectarlink/issues/397)).
- **Unlock the PC with your phone's fingerprint (Windows, Android 11+).** On the lock
  screen, pick NectarLink under Sign-in options, press the arrow and confirm with your
  fingerprint on the phone; allow NectarLink to display over other apps and the fingerprint prompt opens
  straight away instead of a notification to tap. Off by default on both sides;
  turning it on asks for an administrator once and for your Windows password once. The
  password is kept encrypted on the PC with a key only the phone holds, behind its
  fingerprint, and sealed so only the lock screen can read it, not apps running as you. Only for unlocking, not the first sign-in after a restart
  ([#36](https://github.com/thtbee/nectarlink/pull/36)).
- **Unlock a Linux session with your phone's fingerprint.** An opt-in PAM service for a
  running graphical session: the lock screen asks the phone, you confirm with your
  fingerprint, and the password still works as before. Set up and revoked by the
  administrator ([docs](https://github.com/thtbee/nectarlink/blob/main/docs/guide/linux-phone-unlock.md)).
  Thanks to [@UmarAlMukhtar](https://github.com/UmarAlMukhtar)
  ([#494](https://github.com/thtbee/nectarlink/pull/494)).
- **AI agents can use NectarLink.** Claude Code, Claude Desktop, Codex, Cursor, Gemini CLI,
  VS Code and other AI assistants on the PC can read notifications and texts, send files and
  messages, control media, look at and copy photos, work with the phone's files, change its
  quick settings, open phone apps on the PC, press Deck tiles, wake or power off PCs, and
  follow what happens as it happens, through MCP or the `nectarlink-agent` command. Off by
  default: turn it on in **Settings › AI agents**, add each agent with Read only or Everyday
  permissions, and click **Add to** to put it in that app's settings (or copy the setup line
  it shows). **Permissions** changes what each agent may do, area by area. Sending a text,
  replying, calling, deleting or shutting down always waits for your OK in NectarLink, and
  every agent's activity is listed there
  ([docs](https://github.com/thtbee/nectarlink/blob/main/docs/guide/ai-agents.md)).
- **Notifications and messages in the local API.** With **Notifications and messages** on
  under **Settings › Local automation API** (off by default), `GET /v1/notifications` and
  `GET /v1/messages` give tools the phone's notifications and conversations with their unread
  counts, so a Start menu panel or a Stream Deck button can show them. Apps hidden in
  NectarLink stay hidden. Thanks to [@windcom-acd](https://github.com/windcom-acd)
  ([#441](https://github.com/thtbee/nectarlink/issues/441)).
- **Share and accept the clipboard separately, per device.** In the phone's Settings, under
  each paired device: **Share my clipboard with it** decides whether what you copy goes
  there by itself (with Elevated on), and **Accept its clipboard** whether what it copies
  lands on this phone. Both are on, so nothing changes unless you turn one off. Thanks to
  [@FlootingDream](https://github.com/FlootingDream)
  ([#453](https://github.com/thtbee/nectarlink/issues/453)).
- **Ask before sharing your screen, per PC.** In the phone's Settings, under each PC:
  with it on, that PC's requests to see your screen always wait for your tap, even when
  Elevated could share it at once. Off by default, so nothing changes unless you turn it on.
- **Choose whether Elevated turns Wireless debugging on by itself.** It does by default, on
  any Wi-Fi, so Elevated keeps working; turn it off in Elevated's settings to switch it on
  yourself in Developer options.
- **A switch for USB debugging on the PC.** Settings › Connection & About › Use USB
  debugging (on by default) can turn off everything NectarLink does over a USB cable, so a
  phone plugged in for something else is never asked to allow USB debugging. Wi-Fi works as
  usual; it applies when NectarLink restarts. Thanks to
  [@unaibenidorm](https://github.com/unaibenidorm)
  ([#439](https://github.com/thtbee/nectarlink/issues/439)).
- **Choose how long notification history stays.** Settings › Storage & Privacy › Keep
  notification history for: 1 day (as before), 3 days, 1 week or 30 days. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Scroll with the air mouse.** Drag two fingers on the air pad to scroll, up and down
  or sideways, the same as on the touchpad. The pointer holds still while you scroll.
  Thanks to [@w-rakeeb](https://github.com/w-rakeeb)
  ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **See all of a copied text in the Timeline.** A clipboard entry with more than its
  first line gets a **Show all** arrow that opens the whole text right in the entry,
  where you can select and copy just the part you need. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb)
  ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Your phone as a game controller.** Tap the controller button in Control PC and the
  phone turns into a gamepad: two sticks, a d-pad, A B X Y, bumpers, triggers, Back, Home
  and Start. The PC gets an Xbox controller that games see like a real one, one per phone.
  On Windows it needs the free ViGEmBus driver on the PC (the Gamepad tells you if it's
  missing); on Linux it works as is. Not on macOS yet. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb)
  ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Two PCs work together.** Pair a PC with another PC, Windows, Mac or Linux, the
  same way as a phone: open the pairing screen on both and pick the other one. Home
  then shows that PC with Send files, Send clipboard and Ring PC, and what you copy on
  one goes to the other while Sync clipboard is on. No phone needed. Thanks to
  [@ManuZhu0728](https://github.com/ManuZhu0728)
  ([#432](https://github.com/thtbee/nectarlink/discussions/432)).
- **Calls from WhatsApp and other apps show on the PC.** When a WhatsApp, Signal,
  Messenger or Telegram call rings on the phone, the PC shows it like a phone call, with
  the caller and the app's name, in the banner, Nectar Island and the call card. Answer,
  Decline and End press the app's own buttons on the phone, and the PC's volume buttons
  set the call's volume; the sound stays on the phone. Needs notification access. Thanks
  to [@s-shahriar](https://github.com/s-shahriar)
  ([#434](https://github.com/thtbee/nectarlink/issues/434)).
- **The Mac preview does a lot more, and comes as a DMG.** Apple silicon Macs (macOS 14
  or newer) get one with Nightlies and releases. On a Mac, NectarLink now mirrors the phone's screen,
  records it to MP4, plays the phone's sound and microphone, sends the Mac's sound to the
  phone (with Screen Recording permission), shows the phone's storage in Finder, sets the
  Mac's volume instantly, and lives in the menu bar when its window is closed (the Dock
  icon brings it back). It isn't signed with
  an Apple Developer ID yet, so macOS asks you to allow it once (see Getting started),
  and it doesn't update itself yet. Thanks to [@svkrvavilla](https://github.com/svkrvavilla)
  ([#410](https://github.com/thtbee/nectarlink/pull/410), [#411](https://github.com/thtbee/nectarlink/pull/411),
  [#412](https://github.com/thtbee/nectarlink/pull/412), [#413](https://github.com/thtbee/nectarlink/pull/413),
  [#414](https://github.com/thtbee/nectarlink/pull/414), [#415](https://github.com/thtbee/nectarlink/pull/415),
  [#416](https://github.com/thtbee/nectarlink/pull/416), [#417](https://github.com/thtbee/nectarlink/pull/417),
  [#419](https://github.com/thtbee/nectarlink/pull/419), [#426](https://github.com/thtbee/nectarlink/pull/426)).
- **Talk through your PC.** During a call, **On PC** on the call card or in Nectar Island moves the
  call's sound to a Windows PC: you hear the other person from its speakers or headphones
  and talk into its microphone, over NectarLink's own connection, without Bluetooth. The
  phone shows a notification while the sound is on the PC, with **Back to phone** to move
  it back. A call answered or dialed from the PC moves there on its own. Settings › Camera &
  Audio › Microphone for calls picks the PC's microphone. Needs Elevated on Android 13 or
  newer, on a phone whose maker lets apps reach the call's sound (a Pixel 7 does; the PC
  only shows On PC where the phone can). Thanks to [@rus152](https://github.com/rus152)
  ([#418](https://github.com/thtbee/nectarlink/pull/418)); asked for in
  [#133](https://github.com/thtbee/nectarlink/issues/133).
- **New home-screen widgets on the phone.** Four of them, in the app's Bloom style, light and
  dark: **PC at a glance** (connected or not, battery or when it was last seen, Ring, and
  Wake when it's off), **Now playing** (what plays on the PC, with its controls), **Quick
  actions** (send the clipboard or files, ring the PC) and **From your PC** (the last clip,
  to copy again, and the last file, to open). The power button opens a menu with Lock,
  Sleep, Hibernate and Shut down, named the way the PC's OS names them (Suspend and Power
  off on Linux), and only what that PC allows. With more than one PC, each widget can show
  a different one. The old PC widget becomes PC at a glance by itself.
- **Assist only while a PC uses the phone.** With Elevated set up, a switch in the phone's
  Settings › Permissions › Control from your PC keeps the accessibility service off until a PC shows the
  phone, opens an app window or types on it, and turns it off again shortly after. For
  apps that crash or refuse to open while an accessibility service is on, like InstaPay.
  Thanks to [@Devhoss](https://github.com/Devhoss) ([#400](https://github.com/thtbee/nectarlink/issues/400)).
- **Hide NectarLink from recent apps.** A switch in the phone's Settings › Background keeps
  it off the recent apps screen, so clearing that screen can't close it by accident on
  phones where that stops apps. Thanks to [@FlootingDream](https://github.com/FlootingDream)
  ([#409](https://github.com/thtbee/nectarlink/issues/409)).
- **Install the phone app from the PC, over USB.** Settings › Install on a phone checks
  what's needed and walks you through the rest: it gets Android's USB tools from Google
  when the PC has none (checked against Google's checksum), helps turn on USB debugging,
  notices a missing USB driver on Windows, then installs and opens the newest phone app.
  When the phone refuses, it says why and what to do. For phones where Play Protect or
  the maker blocks installing from a browser. The adb it downloads also connects phones
  over a cable from then on.
- **Select a whole day of photos.** On the PC's Photos page, click the day at the top of
  the grid to select everything from that day; click it again to clear it. Thanks to
  [@Dipen425](https://github.com/Dipen425) ([#388](https://github.com/thtbee/nectarlink/issues/388)).
- **Scrollbars.** Long pages on the PC (Home, Settings, Timeline, Messages, Photos, Calls,
  Apps, Deck) and sheets show a thin scrollbar while they scroll or when the pointer is near
  their edge; drag it, or click to jump. Thanks to [@Dipen425](https://github.com/Dipen425)
  ([#387](https://github.com/thtbee/nectarlink/issues/387)).
- **Flex mode for app windows.** A button in an app window's toolbar turns off fitting
  the app to the window, for apps that don't take odd shapes well: the app restarts at
  the phone's resolution and the window keeps that shape when resized. It's remembered
  for each app.
- **Zoom an app window.** Ctrl + mouse wheel makes the app in a window draw bigger or
  smaller, from 50% to 200%, remembered for each app. The window is blurred with a
  spinner until the phone has drawn the app at its new size. Both thanks to
  [@rus152](https://github.com/rus152) ([#369](https://github.com/thtbee/nectarlink/pull/369)).
- **App windows fit any shape, and say when they're catching up.** With Elevated, an app
  window's display now takes the window's exact size, so there are no bars beside the
  app. While you hold an edge on Windows, the window shows the app blurred under its
  icon, and the app is laid out once, when you let go. Thanks to
  [@rus152](https://github.com/rus152) ([#368](https://github.com/thtbee/nectarlink/pull/368)).
- **The full PC media controls on the phone's Home.** What the PC plays shows its artwork,
  Previous as well as Play and Next, and a bar to see and move where it is, like the PC's
  own Home. Thanks to [@doodoobug-dot](https://github.com/doodoobug-dot) ([#399](https://github.com/thtbee/nectarlink/issues/399)).
- **A phone app on the PC's desktop.** Right-click an app on the Apps page (or press the
  Menu key) for Add to desktop on Windows, or Add to app menu on Linux: a shortcut with
  the app's own icon that opens it straight in an app window. Thanks to
  [@unaibenidorm](https://github.com/unaibenidorm) ([#370](https://github.com/thtbee/nectarlink/issues/370)).
- **Uninstall a phone app from the PC.** With Elevated, the same menu has Uninstall, which
  asks once more before it removes the app from the phone. NectarLink can't uninstall
  itself this way. Thanks to [@twistkan](https://github.com/twistkan)
  ([#349](https://github.com/thtbee/nectarlink/discussions/349)).
- **Shut a PC down from the phone.** Shut down sits next to Lock and Sleep on the phone's
  Home, for Windows and Linux PCs, and asks before it does anything. It's the normal
  shutdown: an app with unsaved work can still stop it. Thanks to
  [@unamed666](https://github.com/unamed666) for the idea ([#372](https://github.com/thtbee/nectarlink/discussions/372)).
- **Play the phone's sound on the PC from the phone.** Play phone sound on the PC, on the
  phone's Home, asks the PC to start it, the same as turning it on from the PC. Thanks to
  [@doodoobug-dot](https://github.com/doodoobug-dot) ([#398](https://github.com/thtbee/nectarlink/issues/398)).
- **The torch can turn on with the webcam.** Settings › Camera & Audio on the PC: Torch on
  with the back camera turns the phone's torch on each time the webcam starts with the back
  camera. Thanks to [@jerrywoo96](https://github.com/jerrywoo96) ([#353](https://github.com/thtbee/nectarlink/issues/353)).
- **Hibernate a PC from the phone.** Next to Lock PC and Sleep on Home, for a PC that can
  hibernate: on Windows, while hibernation is turned on. Thanks to
  [@atilazz](https://github.com/atilazz) for the idea ([#355](https://github.com/thtbee/nectarlink/issues/355)).
- **Your PC's mouse on the phone, over the screen edge.** With the phone beside the monitor, set
  **Beside this PC** to Left or Right in Settings › Devices, and the mouse goes past that edge
  onto the phone, with a pointer of its own: click, scroll and type there, then move back, or
  press Ctrl+Alt+Esc. Windows, with Elevated on the phone
  ([#107](https://github.com/thtbee/nectarlink/issues/107)).
- **Support NectarLink.** If you'd like to help keep it going, Settings has a Support
  section on the phone and the PC, and the "Enjoying NectarLink?" prompt has a Support
  button next to Star on GitHub. It only opens the page when you tap it.
- **Choose and check relay servers.** Settings on the PC and the phone lists the relay
  servers used away from home, shows which one each device goes through, checks how fast
  they answer, and takes your own. Thanks to [@krakotay](https://github.com/krakotay)
  ([#324](https://github.com/thtbee/nectarlink/pull/324)).
- **Right-click can be a long press.** In mirroring and app windows, an option in Settings ›
  General makes right-click hold a finger down, for menus and selecting; Esc stays Back.
  Shift+click is a long press either way ([#341](https://github.com/thtbee/nectarlink/issues/341)).
- **A mirror says when the phone ignores your taps.** Xiaomi, Redmi and POCO phones refuse
  taps from the PC with Elevated until USB debugging (Security settings) is on. The phone
  now notices and the PC's window says what to turn on, instead of taps doing nothing.
- **Pick the network interface relays use, on Linux.** Settings › Connection & About can
  send relay connections through one interface, like the one past a VPN that's always on.
  Thanks to [@krakotay](https://github.com/krakotay)
  ([#331](https://github.com/thtbee/nectarlink/pull/331)).
- **Keep a mirror window on top.** A pin button in the window's controls keeps it above other windows. Thanks to [@Nebhay](https://github.com/Nebhay) ([#327](https://github.com/thtbee/nectarlink/pull/327)).
- **Spanish.** NectarLink is available in Spanish on Android and Windows.
  Thanks to José María Garcia Dominguez, and to
  [@KevinL50-sysadmin](https://github.com/KevinL50-sysadmin) for review passes on it
  ([#348](https://github.com/thtbee/nectarlink/pull/348), [#402](https://github.com/thtbee/nectarlink/pull/402),
  [#438](https://github.com/thtbee/nectarlink/pull/438), [#480](https://github.com/thtbee/nectarlink/pull/480),
  [#547](https://github.com/thtbee/nectarlink/pull/547)).

- **Vietnamese.** NectarLink speaks Tiếng Việt now: on the Android phone and
  the Windows PC, every screen is translated. Language (Settings › Appearance on the
  phone, Settings › General on the PC) picks it, or the system language does. Thanks to
  [@wuwenhou](https://github.com/wuwenhou).
- **Russian.** NectarLink speaks Русский on the phone and the PC. Language (Settings ›
  Appearance on the phone, Settings › General on the PC) picks it, or the system language does. Thanks to
  [@krakotay](https://github.com/krakotay) for reviewing it
  ([#343](https://github.com/thtbee/nectarlink/issues/343)).
- **More ways to sync a folder.** A synced folder can now go one way from the PC to the
  phone. Phone to PC can also remove from the PC what you delete on the phone: only
  files it synced and that haven't changed on the PC, and they go to the Recycle Bin. Both
  ways has the same option, so a deletion on either side goes across
  ([#243](https://github.com/thtbee/nectarlink/issues/243)).
- **Sync on Wi-Fi only.** A switch in Settings › Files & Sync on the PC holds folder sync
  and photo backup back while the phone is away, so they don't use its mobile data
  ([#244](https://github.com/thtbee/nectarlink/issues/244)).
- **Share photos and videos from the PC.** On Windows, the Photos page has a Share
  button that opens the system's share sheet
  ([#174](https://github.com/thtbee/nectarlink/issues/174)).
- **Pick how Elevated starts.** When Shizuku is installed or the phone is rooted, Elevated's
  settings on the phone let you choose Wireless debugging, Shizuku or root
  ([#110](https://github.com/thtbee/nectarlink/issues/110)).
- **Don't receive while this PC is locked.** A stricter switch in Settings ›
  Notifications on the PC: the phone sends nothing until you unlock, then Home catches
  up with what's still on the phone
  ([#219](https://github.com/thtbee/nectarlink/issues/219)).
- **Sync copied passwords.** An opt-in switch on the phone and the PC sends what password
  managers mark private. It's marked private on the other side too, leaves that clipboard
  after a minute, and never goes into history, the Timeline or the offline queue
  ([#272](https://github.com/thtbee/nectarlink/issues/272)).
- **Fluent theme.** The PC app can look like Windows 11's own apps: Mica, Segoe UI Variable,
  WinUI's toggles, buttons and navigation, and your Windows accent color. Pick it in
  Settings › Appearance ([#295](https://github.com/thtbee/nectarlink/issues/295)).
- **Switch the PC's audio output from the phone.** Tap the device under Volume on the
  phone, then pick another output, like speakers or headphones. Windows and Linux.
- **A touchpad in full screen.** Control PC's Screen tab, in full screen, has a **Touchpad**
  button: the whole screen then moves the PC's pointer like a laptop's touchpad (tap to click,
  two fingers to scroll or right-click) instead of pointing straight at things
  ([#306](https://github.com/thtbee/nectarlink/issues/306)).
- **Full-screen controls beside the picture.** In landscape, the controls sit in the black
  space on both sides of the PC's screen instead of over it, always shown, with Esc, Copy,
  Paste, Switch window and Undo added.
- **Mirror quality.** Balanced as before, Sharper for the phone's full resolution, or
  Smoother for weak Wi-Fi, in Settings › Camera & Audio on the PC. It's for the phone's
  screen and app windows (those stay at 1920 at most), and over the relay it stays
  Balanced ([#275](https://github.com/thtbee/nectarlink/issues/275)). App windows also
  stay closer to live when the PC falls behind
  ([#284](https://github.com/thtbee/nectarlink/pull/284)). Thanks to
  [@krakotay](https://github.com/krakotay).
- **Back, Home and Recent apps buttons** in the mirror window's toolbar
  ([#192](https://github.com/thtbee/nectarlink/issues/192)).
- **Brazilian Portuguese.** The Android and PC apps speak Brazilian Portuguese
  (Português (Brasil)): pick it in Settings, or they follow the system's language.
  Thanks to [@AlphaOrionisWKG](https://github.com/AlphaOrionisWKG), and to
  [@AlphaOrionisBetelgeuse](https://github.com/AlphaOrionisBetelgeuse) for a polish pass
  ([#375](https://github.com/thtbee/nectarlink/pull/375)).
- **Enter sends messages.** An option under Settings › General: in mirroring and app
  windows, Enter presses the app's Send and Ctrl+Enter makes a new line. It needs
  Assist on the phone, and both devices on this version. Thanks to
  [@s-shahriar](https://github.com/s-shahriar) for finding the last piece
  ([#220](https://github.com/thtbee/nectarlink/issues/220), [#346](https://github.com/thtbee/nectarlink/pull/346)).
- **A bigger touchpad in landscape.** Control PC's Touchpad takes the phone's screen
  when it's sideways, with its buttons underneath. **More** brings the rest back.
- **Keep the phone's screen off.** The toolbar's screen-off button now stays set for
  that phone, and Settings › Devices has the same switch, so mirroring and app
  windows start with the screen off, barely lighting it up
  ([#217](https://github.com/thtbee/nectarlink/issues/217)).
- **App windows run while the phone sleeps.** With Elevated on Android 14 or newer,
  an app window gets a display of its own that keeps running when the phone's screen
  turns off or locks, including on Android 17, and opening one no longer wakes the
  phone. While a window is open, the phone has an app streaming link for the shell,
  removed with the last window; where it can't make one, windows wake the screen as
  before. Their toolbar no longer shows Keep phone awake, Turn phone screen off and
  Lock phone, and they no longer ask the phone for the first two. Thanks to
  [@rus152](https://github.com/rus152) ([#365](https://github.com/thtbee/nectarlink/pull/365)).
- **Pause phone notifications.** From the tray, Home or the command palette, for when
  you're presenting or sharing your screen. They still collect on Home
  ([#218](https://github.com/thtbee/nectarlink/issues/218)).
- **Quiet while this PC is locked.** A setting under Notifications: no pop-ups while
  it's locked, on Windows and Linux
  ([#219](https://github.com/thtbee/nectarlink/issues/219)).
- **App size.** Settings › Appearance makes the PC app bigger, up to 150%, after a
  restart ([#221](https://github.com/thtbee/nectarlink/issues/221)).
- **The macOS app seals the device key in the login Keychain.** `identity.key`
  and the clipboard vault are now encrypted with a key held in the Keychain
  instead of sitting on disk in the clear, so a copy of the app's data folder
  can't act as this Mac. An existing key file is re-sealed in place on first
  launch and keeps its `DeviceId`, so no re-pairing is needed. Clipboard history
  and unsent queue items written by an earlier macOS build can't be opened with
  the new key, so they start empty once. Thanks to
  [@aydinoglucagan-sudo](https://github.com/aydinoglucagan-sudo).
- **Pinch to zoom on the PC's screen.** In Control PC › Screen, two fingers zoom in up
  to 5x and move around while zoomed, and a tap still lands where you see it. Two fingers
  moving together still scroll, and a double tap shows the whole screen again
  ([#351](https://github.com/thtbee/nectarlink/issues/351)).
- **Typing on the PC's screen goes over as you type.** Each letter reaches the PC right
  away, like a keyboard, instead of after Send; the button presses Enter
  ([#351](https://github.com/thtbee/nectarlink/issues/351)).
- **Polish.** The Android and Windows apps can now run in Polish, offered by their
  language pickers. Thanks to [@erykosky](https://github.com/erykosky)
  ([#253](https://github.com/thtbee/nectarlink/pull/253)).
- **Japanese.** NectarLink speaks 日本語 on the phone and the PC. Language (Settings ›
  Appearance on the phone, Settings › General on the PC) picks it, or the system language does.
  Thanks to [@sho9029](https://github.com/sho9029)
  ([#437](https://github.com/thtbee/nectarlink/pull/437)).

- **Pick the SIM a text goes out on.** On a dual-SIM phone, the box you type in shows
  which SIM sends it (SIM 1, SIM 2); click it to switch. A conversation starts on the SIM its
  last message used, a new one on the phone's default, and a SIM you pick stays for that
  conversation. Needs the phone's Phone permission. Thanks to [@jed84](https://github.com/jed84) ([#474](https://github.com/thtbee/nectarlink/issues/474)).
- **Pin apps to the top of Apps.** Right-click or long-press an app and choose **Pin to
  the top**; pinned apps get their own row above Recent, per phone. Thanks to [@jed84](https://github.com/jed84) ([#477](https://github.com/thtbee/nectarlink/issues/477)).
- **Turn the phone's screen from the PC.** A button on the phone's screen window turns the
  phone to landscape or back to portrait; it goes back to its own rotation when mirroring
  stops. Needs Elevated. Thanks to [@jed84](https://github.com/jed84) ([#473](https://github.com/thtbee/nectarlink/issues/473)).
- **Choose when the battery counts as low.** **Settings › General › Low battery at**
  picks 5, 10, 15, 20 or 30%; the alert comes once, and again only after charging or
  climbing 10 points. Thanks to [@jed84](https://github.com/jed84) ([#478](https://github.com/thtbee/nectarlink/issues/478)).
- **A one-color tray icon (Windows).** **Settings › General › Tray icon** shows the
  NectarLink mark in white or black to match the taskbar, and switches when the taskbar
  does. Thanks to [@jed84](https://github.com/jed84) ([#479](https://github.com/thtbee/nectarlink/issues/479)).
- **Choose where the Island sits, and show this PC's players on it.** **Island
  position** puts it at the left, center or right of the top of the screen. **This PC's
  players too** adds music and videos playing on the PC (Spotify, a browser, any player in
  the system's media controls); with more than one playing, a button on the Island switches
  between them. Thanks to [@jed84](https://github.com/jed84) ([#472](https://github.com/thtbee/nectarlink/issues/472)).
- **A light Island, and which app a notification is from.** **Island look** keeps it dark
  or follows the app's light or dark look, and a new notification shows the app's icon,
  who it's from and the app's name instead of a bell and a count.
  Thanks to [@2218084076](https://github.com/2218084076) ([#325](https://github.com/thtbee/nectarlink/issues/325)).
- **A sound buffer for phone sound on the PC, and PC sound on the phone.** Low (what it
  always was), Medium or High, in **Settings › Camera & Audio** on the PC and in Settings on
  the phone: a bigger buffer rides out a busy Wi-Fi, with a little more delay.
  Thanks to [@NeilTheFisher](https://github.com/NeilTheFisher) ([#321](https://github.com/thtbee/nectarlink/issues/321)).
- **More keys in full screen on a phone held upright.** Esc, Copy, Paste, Switch window and
  Undo join the row of buttons that shows on a tap, like the side buttons in landscape.
  Thanks to [@cxigit](https://github.com/cxigit) ([#306](https://github.com/thtbee/nectarlink/issues/306)).
- **Pin a phone app to the taskbar.** Pinning an app window's taskbar button now pins
  that app: the icon opens it in a window, starting NectarLink first if needed. Thanks to
  [@ososuosos](https://github.com/ososuosos)
  ([#465](https://github.com/thtbee/nectarlink/discussions/465)).
- **Hide the toolbar on mirroring and app windows.** Its new last button, or Ctrl+Shift+T,
  hides it in every window so it no longer covers the app's top corner; Ctrl+Shift+T or
  the new switch in Settings brings it back. Thanks to
  [@JohnDoeXA](https://github.com/JohnDoeXA)
  ([#475](https://github.com/thtbee/nectarlink/discussions/475)).
- **Phone number links open in NectarLink.** Click a phone number on the PC (a `tel:` link
  in a browser, an email or another app) and pick NectarLink in the list Windows shows: the
  Calls page opens with the number filled in, ready for you to press **Call**. NectarLink
  doesn't make itself the default. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb)
  ([#509](https://github.com/thtbee/nectarlink/issues/509)).
- **Copy screenshots to the PC.** Turn on **Copy screenshots to PC** in the phone's
  clipboard settings and each new screenshot goes to the clipboard of the connected PCs you
  share your clipboard with, ready to paste. It's off by default and needs full photo
  access. On Windows, images from the phone now show in Win+V history but stay out of cloud
  clipboard sync. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb)
  ([#508](https://github.com/thtbee/nectarlink/pull/508)).
- **Your own color for the Home card.** On the phone, **Settings › Appearance › Home card
  color** sets the color of the PC's card on Home, in any theme: one of the presets or any
  hex color, with the text kept readable on it. **Follow theme** goes back to the theme's
  own. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb)
  ([#501](https://github.com/thtbee/nectarlink/pull/501)).
- **Nectarlink Services for rooted phones and custom ROMs (experimental).** A separate
  community add-on, [nectarlink-services](https://github.com/isaiahscape/nectarlink-services),
  installed as a system app through a
  [KernelSU or Magisk module](https://github.com/isaiahscape/nectarlink-module), gives the
  phone app what Elevated does without Wireless debugging, Shizuku or a root prompt each
  time. NectarLink uses it only after checking its signature, version and what it offers;
  phones without it are unchanged. It's built and looked after by
  [@isaiahscape](https://github.com/isaiahscape), not by NectarLink
  ([#517](https://github.com/thtbee/nectarlink/pull/517)).
- **Move the mirroring toolbar.** Drag it by its grip to any corner of a mirroring or app
  window, so it's off the app's own buttons (a menu in the top corner, say). It stays in
  that corner in every window. Ctrl+Shift+T still hides it. Thanks to
  [@s-shahriar](https://github.com/s-shahriar) ([#340](https://github.com/thtbee/nectarlink/issues/340)) and g3ck0 on Discord.
- **Bring the phone's sound back to the phone.** While the phone's sound plays on a PC,
  that PC's card on the phone says **Play the sound on the phone again** instead of starting
  it once more. Thanks to [@doodoobug-dot](https://github.com/doodoobug-dot)
  ([#521](https://github.com/thtbee/nectarlink/issues/521)).
- **Copy a notification's text.** On the PC's Home, a notification's **...** menu has
  **Copy text** and **Copy title and text**. Thanks to [@korry73](https://github.com/korry73)
  ([#526](https://github.com/thtbee/nectarlink/issues/526)).
- **Adaptive mirror quality.** Settings › Camera & Audio › Mirror quality has **Adaptive**:
  it starts at Balanced, lowers the picture while this PC can't keep up (which is what
  froze mirroring on slower PCs), and goes back up once it has kept up for a while. Thanks
  to [@unaibenidorm](https://github.com/unaibenidorm) and [@marceleno1717](https://github.com/marceleno1717) ([#523](https://github.com/thtbee/nectarlink/issues/523),
  [#464](https://github.com/thtbee/nectarlink/issues/464)).
- **Search in Settings**, on the PC and the phone: type part of a setting's name and pick
  it to go straight there. Thanks to [@unaibenidorm](https://github.com/unaibenidorm) ([#522](https://github.com/thtbee/nectarlink/issues/522)).
- **A paused song stays on the Island** with Play for five minutes, so you can start it
  again from there. Thanks to [@Konakonai](https://github.com/Konakonai) ([#516](https://github.com/thtbee/nectarlink/issues/516)).
- **Folder sync**: pick several phone folders at once, keep each folder's name inside the
  PC folder you choose, edit a synced folder after adding it, and see a notification with
  progress while files go across. Thanks to [@znbaboy](https://github.com/znbaboy) ([#491](https://github.com/thtbee/nectarlink/issues/491)).

### Fixed

- **A feature Windows 10 can't run says it needs Windows 11** instead of only *Unavailable*, such
  as Phone as webcam, and one that needs a newer Android says which version.
- **The phone's volume buttons work for the phone while it plays something itself,** with
  **Volume buttons control your PC** on and the PC playing too. They went to the PC, so the phone's
  own volume couldn't be changed. Thanks to [@Deliond](https://github.com/Deliond)
  ([#643](https://github.com/thtbee/nectarlink/issues/643)).
- **Copy and paste on the phone works with Elevated on** for phones where it pasted nothing in
  every app (a Xiaomi with HyperOS). Elevated only watches the clipboard while some device takes
  the phone's copies by itself, so with **Share my clipboard with it** off for every device, it
  doesn't touch the clipboard. Thanks to [@golip37](https://github.com/golip37)
  ([#655](https://github.com/thtbee/nectarlink/issues/655)).
- **A crash that could close the app on Deck** (likeliest as the PC connected or the phone was
  plugged in) is fixed, along with the same mistake on Desk clock and Control PC. Thanks to
  everyone who sent the report
  ([#550](https://github.com/thtbee/nectarlink/issues/550), [#589](https://github.com/thtbee/nectarlink/issues/589),
  [#671](https://github.com/thtbee/nectarlink/issues/671)).
- **Clicking Play on a phone video in Photos more than once downloads it once.** The button shows
  that it's fetching and waits. Thanks to [@SolomonNumb1](https://github.com/SolomonNumb1)
  ([#666](https://github.com/thtbee/nectarlink/pull/666)).
- **The mouse no longer gets lost on the phone with Beside this PC.** It stays on the PC while a
  game or other app is in full screen or holds the cursor, so pushing against the edge in a game
  doesn't take the mouse and keyboard away. Going back, the PC checks with the phone as the mouse
  heads for the PC, so it comes back as soon as the phone's pointer reaches that edge rather than
  some way past it, and if the phone's pointer doesn't move, the mouse comes back instead of staying
  lost there.
- **Sending to a LocalSend device no longer says it's offline.** It wasn't a paired device, so the
  PC and the phone said "offline, will send when it's back" though the files were going out.
- **A phone's music no longer comes back to the phones from a Windows PC.** The PC shows a phone's
  media in Windows' media controls, and could then send it on to phones as if it played on the PC,
  showing on the phone that plays it and on any other. Thanks to
  [@FlootingDream](https://github.com/FlootingDream) ([#653](https://github.com/thtbee/nectarlink/issues/653)).
- **The phone's hotspot switch works from the PC with Elevated** on current Android. It tried
  shell commands that Android doesn't have, so nothing happened; it now asks Android's tethering
  service, as Settings does. Thanks to [@johnzhang0707](https://github.com/johnzhang0707)
  ([#615](https://github.com/thtbee/nectarlink/issues/615)).
- **The phone's check for banking apps keeps its slow pace once the screen turns off**, instead
  of looking once more straight away after a recent unlock. Thanks to
  [@aihehye](https://github.com/aihehye) ([#592](https://github.com/thtbee/nectarlink/pull/592)).
- **The PC's Doctor no longer warns about a VPN for Windows' own tunnels** (Teredo,
  IP-HTTPS, 6to4, ISATAP) or a PPPoE internet connection, and when it does warn, it names the
  adapter it means. Thanks to [@Tuna321](https://github.com/Tuna321)
  ([#662](https://github.com/thtbee/nectarlink/issues/662)).
- **The Open and Show in folder buttons on the PC's notifications are in your language** for
  received files, recordings and screenshots, not always in English. Thanks to
  [@yiriyangguang](https://github.com/yiriyangguang) ([#659](https://github.com/thtbee/nectarlink/issues/659)).
- **What's new on the phone no longer closes the app** when two lines of the notes read the
  same ([#594](https://github.com/thtbee/nectarlink/issues/594)).
- **A failure in one of the phone's answers to the PC no longer takes the connection down.**
  It's caught and kept for problem reports with where it happened
  ([#597](https://github.com/thtbee/nectarlink/issues/597)).
- **The PC's volume no longer jumps to full when an app opens on some phones** (HyperOS) while
  the phone's volume buttons control the PC. A sudden jump up that isn't from the buttons or the
  volume panel is left alone; turning it down always works
  ([#643](https://github.com/thtbee/nectarlink/issues/643)).
- **App windows aren't offered on phones that can't run apps on another display** (some Android
  Go phones), instead of opening the app on the phone's own screen
  ([#635](https://github.com/thtbee/nectarlink/issues/635)).
- **A laptop's battery now shows on the phone**: on the PC's card, in the PC widget and on the
  Desk clock. The phone could show it, but the PC never sent it. Thanks to
  [@M-P-A](https://github.com/M-P-A) ([#636](https://github.com/thtbee/nectarlink/issues/636)).
- **Moving a call to the PC no longer stops at "No microphone"** when Windows' default
  communications microphone is off or missing: it uses the default microphone instead. When
  Windows' privacy settings keep NectarLink off the microphone, it now says so and where to
  turn it on ([#637](https://github.com/thtbee/nectarlink/issues/637)).
- **A WhatsApp call from the PC starts on the phone by itself.** It used to need a tap on a
  notification whenever NectarLink wasn't on the phone's screen. Now it starts straight away
  when Android allows it: with **Display over other apps** allowed for NectarLink, or with the
  PC set up as the phone's companion device. On a locked phone, or if Android still blocks it,
  the notification to tap comes as before.
- **The phone app no longer crashes on start with battery saver on.** On some phones (Xiaomi
  with battery saver, say) Android stopped the background service before it could show its
  notification, and closed the app for it. Thanks to [@aihehye](https://github.com/aihehye)
  ([#628](https://github.com/thtbee/nectarlink/issues/628), [#638](https://github.com/thtbee/nectarlink/pull/638)).
- **The phone on PC Home has a round camera.** Phones that report their camera hole as a box
  from the top edge showed a square tab there. It's now drawn round, and the signal and Wi-Fi
  icons line up with it ([#618](https://github.com/thtbee/nectarlink/issues/618)).
- **The Linux install script stops at a missing option value.** `--version` or `--package`
  with nothing after it no longer takes the next option as its value, or quietly falls back
  to the defaults. Thanks to [@dani3lsamir](https://github.com/dani3lsamir)
  ([#617](https://github.com/thtbee/nectarlink/pull/617)).
- **A phone number clicked on Linux opens the dialer.** It was sent to the phone as a link
  instead, unlike on Windows ([#611](https://github.com/thtbee/nectarlink/issues/611)).
- **The PC app stays usable in a small tiled window.** Tiling window managers such as Hyprland
  and Sway can make the window smaller than it's meant to go, and the top bar, Home's cards
  and Settings' categories overlapped or ran off the edge. Below that size the app now scrolls,
  and Settings' categories wrap. Thanks to [@Lik1ch](https://github.com/Lik1ch)
  ([#607](https://github.com/thtbee/nectarlink/pull/607)).
- **The Linux app no longer crashes a few seconds after it starts.** When the window was busy
  loading, a clipboard check could give up waiting and then still run later, writing over
  memory it no longer owned. Thanks to [@aorizondo](https://github.com/aorizondo) for finding it
  and the fix ([#612](https://github.com/thtbee/nectarlink/issues/612)).
- **Photos taken sideways show upright.** Photos that store their rotation as a flag, as many
  phones do, showed on their side in Photos on the PC. Thanks to
  [@svenvdmeer](https://github.com/svenvdmeer)
  ([#559](https://github.com/thtbee/nectarlink/issues/559)).
- **A PC touchpad scrolls the phone smoothly.** With the mouse on the phone beside the PC, a
  touchpad's small scroll steps were dropped until one was a whole wheel notch, so scrolling
  felt stuck and jumpy. They now add up
  ([#107](https://github.com/thtbee/nectarlink/issues/107)).
- **The Linux app starts on Wayland with PipeWire capture installed.** Checking for the screen
  capture portal stopped the app's background service at start, so it stayed on the loading
  screen. Thanks to [@krakotay](https://github.com/krakotay)
  ([#598](https://github.com/thtbee/nectarlink/issues/598), [#595](https://github.com/thtbee/nectarlink/pull/595)).
- **Photos loads more than the first page.** Dragging the scrollbar to the bottom stopped short
  of it, and a first page that didn't fill the window never asked for more, so some galleries
  showed only their first 80 photos. Thanks to [@aihehye](https://github.com/aihehye)
  ([#533](https://github.com/thtbee/nectarlink/issues/533), [#588](https://github.com/thtbee/nectarlink/pull/588)).
- **App windows take clicks with the phone locked on more phones.** On some phones (a POCO
  F5 Pro on HyperOS, say) the window's display went to sleep with the phone, so typing still
  worked but clicks didn't. The phone is now woken for it as on phones that don't run windows
  on a display of their own, with its screen kept dark
  ([#528](https://github.com/thtbee/nectarlink/issues/528)).
- **Setting up unlock with phone says what's wrong.** With it off on the phone, the PC said
  "Not confirmed on the phone" although the phone never asked; it now says to turn it on there.
  A phone without a fingerprint that Android counts as strong now gets its own message instead
  of being told to turn it on, and a phone off the PC's network gets that one
  ([#578](https://github.com/thtbee/nectarlink/issues/578),
  [#579](https://github.com/thtbee/nectarlink/issues/579)).
- **The virtual camera rests on Linux.** With the phone's camera off, NectarLink Webcam kept
  redrawing its placeholder, using about a third of a CPU core; it now draws it once. Thanks
  to [@vladikchi](https://github.com/vladikchi)
  ([#566](https://github.com/thtbee/nectarlink/pull/566)).
- **HEIC photos show in the Photos viewer.** The PC can't read HEIC itself, so the viewer
  stayed empty; it now shows the photo's preview and says that Open in Photos shows it in full
  ([#559](https://github.com/thtbee/nectarlink/issues/559)).
- **Unlock with phone asks the next phone.** With several phones set up, one that's
  connected but away from the PC's network said "Not confirmed on the phone" and stopped
  there; the PC now goes on to the next phone that's on the network
  ([#535](https://github.com/thtbee/nectarlink/issues/535)).
- **The PC's mouse goes over to phones on Android 13 and older.** The phone measured its
  pointer from a status line only newer Android versions have, so the mouse came over for a
  moment and went straight back ([#107](https://github.com/thtbee/nectarlink/issues/107)).
- **Folder sync is quicker with many files.** Comparing a big folder no longer slows down
  with the square of its file count. Thanks to [@aihehye](https://github.com/aihehye)
  ([#537](https://github.com/thtbee/nectarlink/pull/537)).
- **Less work in the tray.** Storage sizes for Settings are only counted while Settings is
  open, not on every new clipboard item, text or photo. Thanks to
  [@aihehye](https://github.com/aihehye) ([#538](https://github.com/thtbee/nectarlink/pull/538)).
- **Photos load faster.** The phone makes thumbnails several at a time and only reads one page
  of its gallery per request, and the PC asks for more than one batch at once, visible photos
  before album covers. Going back to Photos no longer reloads everything. Thanks to
  [@aihehye](https://github.com/aihehye) ([#583](https://github.com/thtbee/nectarlink/pull/583)).
- **The Windows app connects sooner after it starts.** Checking whether the graphics card
  decodes H.265 no longer holds up the start; it's offered to phones once the check is done.
  Thanks to [@aihehye](https://github.com/aihehye) ([#539](https://github.com/thtbee/nectarlink/pull/539)).
- **Sleep from the phone works on more laptops.** Laptops that sleep the Modern Standby way
  ignored it; the screen now goes off as with the power button, and Windows takes them into
  standby ([#542](https://github.com/thtbee/nectarlink/issues/542)).
- **One NectarLink in the Start menu.** The helper that puts NectarLink in Windows' Share
  menu no longer adds a second entry ([#544](https://github.com/thtbee/nectarlink/issues/544)).
- **Showing the PC's screen no longer closes the app on Wayland.** Without GStreamer's
  PipeWire plugin, a Wayland session fell back to X11 capture, which some compositors
  answer with an error that ended the app. The phone now says sharing failed instead
  ([#548](https://github.com/thtbee/nectarlink/issues/548)).
- **The wireless debugging code shows during Do Not Disturb.** Elevated setup's code
  notification can come through Do Not Disturb, and while it's on the app says to turn it
  off until pairing is done ([#496](https://github.com/thtbee/nectarlink/issues/496)).
- **The PC widget's Wake button fits a small widget** instead of being cut off at the bottom.
- **Linux file pickers are the desktop's own.** Send files and the other file dialogs use the
  desktop portal when no Qt theme is set, so COSMIC, GNOME and others show their usual picker
  with sorting by date and size. Thanks to [@krakotay](https://github.com/krakotay)
  ([#545](https://github.com/thtbee/nectarlink/pull/545)).
- **Showing the PC's screen on Wayland is steadier.** Stopping it no longer panics during
  cleanup, and when the OpenGL capture stalls or comes out black it tries the plain PipeWire
  path again on a fresh connection. Thanks to [@krakotay](https://github.com/krakotay)
  ([#540](https://github.com/thtbee/nectarlink/pull/540)).
- **A text's notification opens the conversation.** Clicking a text message's notification on
  the PC brings up NectarLink on that phone's Messages page, with the conversation open at the
  message, ringed for a moment. This works wherever a notification reports its click: Windows
  and Linux now, and macOS once its notifications do. Thanks to
  [@farfromrefug](https://github.com/farfromrefug) ([#551](https://github.com/thtbee/nectarlink/pull/551)).
- **Conversations open at the latest message**, every time: on the first sync, when you open
  one again, and after you send a message, which no longer jumps somewhere else.
- **Scrolling Messages follows the system's scroll speed on macOS**, for a mouse wheel as for a
  trackpad. Qt dropped the speed and acceleration set in System Settings for a wheel, so
  scrolling felt slow beside other apps.
- **Copying a message or a phone number** from Messages or Calls says **Copied to clipboard.**

- **A phone app's shortcut can be taken away again.** Once an app has a shortcut, its menu
  on the PC's Apps page says **Remove from app menu** (Linux, where app menus have no way to
  delete an entry) or **Remove from desktop** (Windows). Thanks to
  [@unaibenidorm](https://github.com/unaibenidorm)
  ([#370](https://github.com/thtbee/nectarlink/issues/370)).
- **Chinese and Japanese text on Windows** is drawn with Windows' own text renderer, which
  is crisper at small sizes. Thanks to [@zzz1465720680](https://github.com/zzz1465720680) ([#460](https://github.com/thtbee/nectarlink/issues/460)).
- **Turning the phone's screen off while mirroring froze the picture** when the mirror
  ran through screen capture instead of Elevated. The screen now goes dark only when
  Elevated shows the mirror; otherwise the phone just stays awake. Thanks to
  [@yiriyangguang](https://github.com/yiriyangguang) ([#520](https://github.com/thtbee/nectarlink/issues/520)).
- **Pair nearby** no longer keeps listing a device you've just paired, and when the other
  device isn't on its pairing screen it says to open it, instead of "Couldn't reach the PC".
  A PC's page shows its transfers too. Thanks to [@krakotay](https://github.com/krakotay)
  ([#514](https://github.com/thtbee/nectarlink/issues/514), [#515](https://github.com/thtbee/nectarlink/issues/515)).
- **Niri** (Linux): typing from the phone uses the keyboard layout that's active right now;
  the Nectar Island hides when it has nothing to show, rather than catching clicks; and the
  PC's screen on the phone falls back to OpenGL capture when the GPU's frames don't come
  through (NVIDIA). Thanks to [@vladikchi](https://github.com/vladikchi)
  ([D511](https://github.com/thtbee/nectarlink/discussions/511)).
- **Mirroring and the webcam on NixOS.** The Nix package now keeps the video decoder and
  the other libraries NectarLink loads at run time, and builds against the FFmpeg version
  it supports, so a phone's screen and camera show again instead of "This PC can't decode
  the phone camera's video". Thanks to [@Konakonai](https://github.com/Konakonai)
  ([#512](https://github.com/thtbee/nectarlink/pull/512),
  [#519](https://github.com/thtbee/nectarlink/issues/519)).
- **Mirroring with Elevated on phones that can't share their sound from it.** On some
  phones (Android 17 on MagicOS) capturing the phone's sound failed in a way that stopped
  Elevated altogether, so mirroring asked for screen-capture consent and root was asked for
  again. Now the screen still mirrors without a prompt, just without the phone's sound.
  Thanks to [@yiriyangguang](https://github.com/yiriyangguang)
  ([#518](https://github.com/thtbee/nectarlink/issues/518)).
- **Less battery on the phone while it's connected.** A quiet connection no longer sends
  pings of its own every 15 seconds on top of the connection's keepalives; it only checks
  when nothing has arrived for a while, and a PC that's gone still shows as offline as
  quickly as before. A small change in the round-trip time also no longer redoes the
  work of a new connection, and the banking pause sleeps when no apps are chosen. Thanks
  to [@Roxxust](https://github.com/Roxxust) ([#145](https://github.com/thtbee/nectarlink/issues/145)).
- **Photos on the PC no longer slows the whole app down with thousands of photos.** Each
  thumbnail that arrived made the PC rebuild the entire gallery, so with a large library
  everything lagged while the grid filled in. Thanks to
  [@w-rakeeb](https://github.com/w-rakeeb) ([#442](https://github.com/thtbee/nectarlink/issues/442)).
- **Less CPU on a PC whose graphics driver isn't working.** Windows then draws the app
  in software, where the moving honeycomb costs real CPU; it now keeps still there, and
  the log names the graphics adapter in use. Thanks to
  [@donsenilo1968](https://github.com/donsenilo1968)
  ([#456](https://github.com/thtbee/nectarlink/discussions/456)).
- **NectarLink starts on Windows installs without `cldapi.dll`.** Some trimmed Windows
  builds remove the Cloud Files API, and the app stopped with "cldapi.dll was not found".
  It now starts there, with the phone folders in File Explorer off. Thanks to
  [@Akbardaging](https://github.com/Akbardaging) ([#499](https://github.com/thtbee/nectarlink/issues/499)).
- **The PC's log keeps your details out and stays small.** It no longer writes your Windows
  user name inside folder paths, network addresses (only their kind, like "home network"),
  or values it doesn't know are safe. It also keeps to about 2 MB while NectarLink runs,
  where it used to grow until the next start. Logs from earlier versions are deleted once.
- **App windows take input while the phone is locked on Xiaomi phones.** HyperOS doesn't
  let NectarLink make the virtual device app windows normally run on, and on the plain
  display it falls back to, Android drops input once the phone sleeps. Now the phone stays
  awake for the window with its own screen dark and still locked, and goes back to sleep
  when the last window closes; its power button lights the screen as usual.
  Thanks to [@MedAICodes](https://github.com/MedAICodes) ([#483](https://github.com/thtbee/nectarlink/issues/483)).
- **The phone stays quiet while a call's sound is on the PC.** Android copies the call to
  the PC rather than moving it, so the phone kept playing it too; now its call volume goes
  to the lowest while the PC has the call, and comes back after. Thanks to [@znbaboy](https://github.com/znbaboy) ([#133](https://github.com/thtbee/nectarlink/issues/133)).
- **The Island shows the first notification after NectarLink starts.** It only noticed
  notifications from the second one on.
- **Messages no longer lists empty conversations.** Conversations with no texts left in them
  (cleared ones, or chats that were only ever RCS) showed on the PC as empty threads. Thanks to
  [@jed84](https://github.com/jed84) ([#167](https://github.com/thtbee/nectarlink/issues/167)).
- **NectarLink starts even when the network isn't up yet at sign-in.** Starting with
  Windows while the network was still coming up could stop it with "Service 'mdns'
  error" until it was started again. It now starts without finding phones on the network
  and turns that on as soon as it can. Thanks to [@AdamB2023](https://github.com/AdamB2023)
  ([#466](https://github.com/thtbee/nectarlink/issues/466)).
- **The share sheet says "Send with NectarLink".** It said "Send to PC", but it sends to
  whichever paired device is connected, another phone included. Each device still shows
  by name next to it. Thanks to [@FlootingDream](https://github.com/FlootingDream)
  ([#458](https://github.com/thtbee/nectarlink/issues/458)).
- **Shizuku forks work for Elevated.** Elevated offered Shizuku only when Shizuku's own
  app was installed. It now also offers it when a fork under another name is running
  (#452).
- **A paired phone's music stays on that phone.** When two phones were paired, music
  playing on one showed up in the other's media controls. Only a PC's players show there
  now (#454).
- **Stop reconnecting stops at once.** Right after turning it on, a device could still dial
  once, and the other side showed it connected for a moment. The other side also no
  longer flashes it as connected each time it tries to reach a device that has stopped.
- **Linux packages always carry every language.** The .deb, .rpm, tarball and AppImage
  now build the translations for each language offered in Settings themselves, so a
  package can't go out missing one or carrying an old one. Thanks to
  [@hvenanc](https://github.com/hvenanc)
  ([#333](https://github.com/thtbee/nectarlink/pull/333)).
- **The phone's screen sleeps when nothing is happening.** After viewing a PC's screen,
  the phone never slept again while NectarLink was open. The touchpad, Deck and Desk now
  keep it on for 10 minutes after the last touch (Deck and Desk stay on while charging),
  the PC's screen while it shows, and the webcam while it streams.
- **Uninstalling a phone app from the PC works.** It always failed: Elevated's helper
  didn't run the uninstall it was asked for.
- **Pictures and files attach to texts on Linux and macOS.** Picking one turned its path
  into a relative one, so nothing was attached.
- **Polish follows the system language on the PC.** With the PC set to Polish and the app
  on "Follow system", it showed English.
- **Widgets for two PCs act on the right one.** Ring, Wake and the media buttons went to
  whichever PC's widget was drawn last.
- **Links from the PC open with the phone's notifications off.** With NectarLink open they
  open at once; otherwise they come as a notification and never open over what you're doing.
- **Linux updates start the new version.** After a .deb or .rpm update the app closed and
  didn't come back. Without curl, downloading an update hung.
- **Settings shows only update options that work.** Linux no longer offers installing
  updates by itself (it asks for your password each time), and the Mac preview no longer
  says it's up to date without checking for a Mac build.
- **A video from the PC lets the phone's screen sleep once paused or finished.**
- **A drag on the touchpad never leaves the PC's mouse button held**, even if the screen
  is left midway.
- **NectarLink's data on Linux and macOS is readable by you only.** The data folder and
  database took the system default, which on many Linux systems lets other accounts on
  the same computer read them.
- **Elevated makes sure it's talking to its own helper** before it sends it anything.
- **The phone is more careful about what may pair with it over a USB cable.** Plugging
  into a PC works as before.
- **LocalSend receiving holds up better on a busy or unfriendly network.** One device
  can no longer crowd out the others or fill the list of LocalSend devices.
- **Cancelling a LocalSend receive ends it at once,** also when you accepted it and the
  sender hasn't started: it could stay "receiving". A cancel at the last
  moment is no longer reported as received, a file that fails to save to another drive
  leaves no broken copy, and a write error at the very end now fails the file.
- **What you type in Messages on the PC stays when you go to another page.** A half
  written text came back empty after a look at Calls or Settings; it now waits in its
  conversation until you send it or clear it.
- **A first text to someone new that fails to send comes back to the box,** with the
  number and the picture, instead of being lost.
- **A chat reply from the PC that fails says "Not sent".** It said "Sending…" for good,
  then vanished.
- **The PC stays on the phone you picked when its list reorders.** When a phone paired
  earlier came back online and moved up the list, a page open for another phone could
  act on the one that took its place, such as taking a photo with the wrong phone.
- **Renaming a phone on the PC shows the new name everywhere at once.** Some labels kept
  the old one until something else changed.
- **Saving a file to the phone from the PC twice at once can't mix the two.** The second
  is turned away instead of writing into the same unfinished file.
- **Cancelling pairing always stops it,** even at the last moment, and a pairing code
  pairs one device.
- **Transfers, rings and pairing catch up when the app falls behind.** In a very busy
  moment the app can miss some of what happens; a transfer could then stay "receiving", a
  ring stay on screen or pairing wait forever. The app now gets what it missed.
- **Switching the phone's update channel during a check no longer offers the old
  channel's build,** or installs it by itself.
- **The phone as a webcam recovers when Android takes its video encoder away.** It
  starts it again, or ends the stream, instead of leaving the PC a frozen picture.
- **A voice recording keeps going after the phone's sound system restarts,** instead of
  recording silence to the end, and the phone's microphone on a PC ends rather than
  hang when the microphone stops answering.
- **A PC task's "running" notification goes away when you unpair that PC,** or when it
  has been away for half an hour.
- **A file saved from the PC over one on the phone's SD card is never left half
  written** when saving fails; the old one stays as it was.
- **Elevated uses less battery while it can't reach Wireless debugging.** It tries less
  often as time goes on, and right away when Wi-Fi or Wireless debugging changes.
- **Windows updates that ask for administrator rights check the download more safely.**
- **Windows shows the update channel that really applies.** Where updates install for
  every account on the PC, they all follow one channel, the last one picked; Settings
  now shows that one and says so, instead of each account setting it back at start.
- **Pairing from the PC's list of nearby phones works.** Picking a phone there said it
  couldn't reach the phone, because the phone only took pairing it started itself. While
  its pairing screen is open, the phone now accepts it from either side.
- **A PC's gamepad works in phone games.** Playing on a mirrored phone with a controller
  on the PC, the phone read its buttons in the wrong order (B left the game, others did
  nothing) and had forward and back the wrong way round. Buttons, sticks, triggers and the
  D-pad now arrive as the phone expects. Thanks to
  [@unamed666](https://github.com/unamed666)
  ([#433](https://github.com/thtbee/nectarlink/discussions/433)).
- **The Mac app talks like a Mac app.** The walkthrough and Settings no longer mention
  Windows notifications, Start with Windows, Windows DPAPI or Windows Firewall on a Mac:
  they say Start at login, the login Keychain and the menu bar, the network check says
  what it really checks, and Send to and hotspot joining show as not on macOS yet. Next
  and Done in the walkthrough follow the app's language. Thanks to
  [@ManuZhu0728](https://github.com/ManuZhu0728)
  ([#431](https://github.com/thtbee/nectarlink/issues/431)).
- **The phone stops trying to reach PCs when it can't.** With no Wi-Fi, Ethernet or VPN,
  and Connect over mobile data off, it no longer dials every two minutes in the background:
  it waits until a network comes back, which saves battery away from home, and its
  notification says it's waiting for Wi-Fi. A USB cable to the PC still connects. Thanks to [@ThePharaohArt](https://github.com/ThePharaohArt)
  ([#397](https://github.com/thtbee/nectarlink/issues/397)).
- **Sharing the PC's screen on Wayland recovers when the desktop doesn't answer.**
  Starting it while the desktop was locked (on COSMIC, say) left the phone on
  Connecting, and every later try got stuck behind the first. Each try now waits up to
  20 seconds for the screen sharing permission and then lets go, and the phone says
  sharing failed and offers to reconnect. Thanks to [@krakotay](https://github.com/krakotay)
  ([#403](https://github.com/thtbee/nectarlink/pull/403)).
- **Recording a mirror saves on Windows when the phone streams H.265.** It stopped with
  "required headers were not provided", since the recording was written as H.264. On
  Linux, an H.265 mirror says it can't be recorded yet instead of failing at the end.
  Thanks to [@znbaboy](https://github.com/znbaboy) ([#393](https://github.com/thtbee/nectarlink/issues/393)).
- **The phone's seek bar for what the PC plays stays put.** It disappeared whenever a
  player on the PC dropped its timeline for a moment, around play and pause, and while
  the phone was locked or asleep and the PC's player stopped reporting it for a while
  during playback. Thanks to
  [@znbaboy](https://github.com/znbaboy) ([#392](https://github.com/thtbee/nectarlink/issues/392)).
- **Chinese, Japanese and Korean text uses Windows' own UI fonts.** Chinese showed in a
  serif font like SimSun that was hard to read; it now uses Microsoft YaHei UI (or the
  matching font for Traditional Chinese, Japanese and Korean). Thanks to
  [@ManuZhu0728](https://github.com/ManuZhu0728) ([#390](https://github.com/thtbee/nectarlink/issues/390)).
- **The phone sounds and vibrates for notifications while you use the PC.** It used to
  silence them whenever you were active on a paired PC. That's now a choice, off by
  default: Quiet while you use the PC, in the phone's Settings › Notifications, and it
  only applies while the PC shows the phone's notifications. Thanks to
  [@sbstn87](https://github.com/sbstn87) ([#384](https://github.com/thtbee/nectarlink/issues/384))
  and [@krilok](https://github.com/krilok) ([#407](https://github.com/thtbee/nectarlink/issues/407)).
- **NectarLink starts quietly in KDE Plasma.** Plasma opened its shortcut settings at
  every start, since NectarLink asked for its global shortcuts each time (twice); it now
  asks only when Plasma doesn't have them yet, or when you change one. And Plasma's
  session restore no longer reopens NectarLink's window at login: Start at login starts
  it, in the tray. Thanks to [@sbstn87](https://github.com/sbstn87) ([#383](https://github.com/thtbee/nectarlink/issues/383)).
- **Elevated starts through root.** Magisk and KernelSU gave the shell user none of its
  groups, so the helper couldn't open its connection. NectarLink now asks for them.
  Thanks to [@SA-SUJON](https://github.com/SA-SUJON) ([#366](https://github.com/thtbee/nectarlink/issues/366)).
- **Resizing a window is smooth on Windows.** Dragging the edge of an app window, the
  mirror or the main window often froze for a second at a time. Thanks to
  [@rus152](https://github.com/rus152) ([#367](https://github.com/thtbee/nectarlink/pull/367)).
- **App windows open with Elevated on ROMs that treat their display as screen sharing.**
  Some Android 13 ROMs refused the display an app window draws into, because the shell
  user only has the screen capture permission from Android 14, and fell back to the
  screen sharing prompt. NectarLink now makes a private display there, which needs no
  such permission. Thanks to [@Rajdave69](https://github.com/Rajdave69) ([#136](https://github.com/thtbee/nectarlink/issues/136)).
- **Setup works on Windows that can't show the admin prompt.** Some Windows installs have
  lost "Run as administrator" for programs. Setup then said it needed your permission as if
  you'd said no; it now shows Windows' real reason, and started from an administrator
  prompt it installs without asking again ([#364](https://github.com/thtbee/nectarlink/discussions/364)).
- **USB connects after a restart without starting adb yourself.** When no adb server is
  running, NectarLink starts one, using the adb from Android Studio's SDK folder or your
  `PATH`, and tries again every minute until one answers. The Connection Doctor says whether
  adb is running, with a Start adb button, or missing. Thanks to
  [@Asofwar](https://github.com/Asofwar) ([#360](https://github.com/thtbee/nectarlink/pull/360), [#322](https://github.com/thtbee/nectarlink/issues/322)).
- **Russian reads more naturally on the phone.** Pairing, silent mode, file transfer and
  clipboard history wording, and the screen sharing hints name the real Stop button.
  Thanks to [@localzet](https://github.com/localzet) ([#361](https://github.com/thtbee/nectarlink/pull/361)).
- **A cut-off video stream counts as an error, not a clean end.** Phone screen, webcam
  and microphone streams also reject a packet length that doesn't fit before using it.
  Thanks to [@localzet](https://github.com/localzet) ([#362](https://github.com/thtbee/nectarlink/pull/362)).
- **The Linux updater's tests cover ARM64 packages**, a first step towards Linux ARM64 builds
  ([#356](https://github.com/thtbee/nectarlink/issues/356)). Thanks to
  [@localzet](https://github.com/localzet) ([#363](https://github.com/thtbee/nectarlink/pull/363)).
- **Relays through a chosen network interface on Linux try every address**: one that
  can't be set up (IPv6 on that interface, say) no longer stops the IPv4 one being tried.
  Thanks to [@localzet](https://github.com/localzet) ([#396](https://github.com/thtbee/nectarlink/pull/396)).
- **Deleting photos from the PC counts what the phone deleted correctly**, and a phone
  can't claim more than were asked for. Thanks to [@localzet](https://github.com/localzet) ([#395](https://github.com/thtbee/nectarlink/pull/395)).
- **Handoff spots videos and maps in links with a port**, like `youtube.com:443`, and
  keeps a Vimeo time right, even in a link that already had one. Thanks to
  [@localzet](https://github.com/localzet) ([#391](https://github.com/thtbee/nectarlink/pull/391),
  [#405](https://github.com/thtbee/nectarlink/pull/405)).
- **The Theme picker in Settings › Appearance no longer sits over its label.**
- **Waking a PC through another device says when no packet went out**, instead of
  reporting success. Thanks to [@localzet](https://github.com/localzet) ([#385](https://github.com/thtbee/nectarlink/pull/385)).
- **The Linux install tests fail when they should**: uninstalling, and a damaged package
  being refused for its checksum. Thanks to [@localzet](https://github.com/localzet) ([#386](https://github.com/thtbee/nectarlink/pull/386)).
- **Pairing a phone again after unpairing is covered by tests**, also when one side
  forgot the other while it was away. Thanks to [@localzet](https://github.com/localzet) ([#406](https://github.com/thtbee/nectarlink/pull/406)).
- **The phone's screen window keeps the phone's shape, and its size.** Resizing it no
  longer leaves black bars: a moment after, it fits the phone's shape again. It opens
  at the size you left it, also when the phone turns. Thanks to
  [@MikeWilliams-UK](https://github.com/MikeWilliams-UK) ([#358](https://github.com/thtbee/nectarlink/issues/358)).
- **The flashlight works from the PC while the phone is a webcam.** The phone's camera is busy then,
  so the flashlight button now switches the webcam's own torch, and shows it, instead of failing.
  The button also follows the webcam starting and stopping, so it never shows a torch that's off
  as on. Thanks to [@jerrywoo96](https://github.com/jerrywoo96) ([#353](https://github.com/thtbee/nectarlink/issues/353)).
- **Full-screen controls beside the PC's screen.** They're one size and shape on both
  sides, labels shrink to fit instead of breaking mid-word with a large font, the
  selected one is easy to read, and so is what you type in the bar below.
- **Large fonts squeezed a PC's status on Home.** "Offline · seen …" was squeezed into a
  letter-wide column beside Wake; the buttons now move to a line of their own instead.

- **Find device stops everywhere.** The button on the device that started the alarm resets when it's stopped on the other one, and a phone shows a full-screen Stop. PC playback also reaches every connected phone, in order. Thanks to [@krakotay](https://github.com/krakotay) ([#326](https://github.com/thtbee/nectarlink/pull/326)).
- **Phone storage in File Explorer kept showing files deleted on the phone.** They're now removed when the phone's folder is read again. Files you added on the PC that haven't reached the phone stay ([#328](https://github.com/thtbee/nectarlink/issues/328)).
- **WireGuard tunnels no longer fill the Linux log with mDNS errors.** Interfaces that
  can't multicast are skipped. Thanks to [@c4g7-dev](https://github.com/c4g7-dev)
  ([#337](https://github.com/thtbee/nectarlink/pull/337)).
- **On Linux, starting at login opened the window.** The app is started minimized there, but
  the window never saw that. Thanks to [@c4g7-dev](https://github.com/c4g7-dev) for finding
  why ([#336](https://github.com/thtbee/nectarlink/issues/336)).
- **A mirrored phone could scroll by itself from a Linux PC.** The app took any joystick
  device for a game controller, a laptop's motion sensor too, and sent its tilt to the
  phone as a held stick. Only real controllers count now
  ([#332](https://github.com/thtbee/nectarlink/issues/332)).
- **The mirror's toolbar no longer stays over the app.** It stayed up the whole time the
  phone's screen was off or a recording ran, covering the app's top bar. It now shows only
  when you point at it, and a small badge shows recording and the screen being off
  ([#340](https://github.com/thtbee/nectarlink/issues/340)).
- **"Call with" didn't show for phones with two SIMs.** On newer Android the phone
  couldn't list its SIMs, so the PC never offered the choice
  ([#334](https://github.com/thtbee/nectarlink/issues/334)).
- **Phone storage catches up when the phone connects.** Every folder already open in File
  Explorer is read again, so files deleted on the phone while it was away, or before the
  app started, go from the PC too
  ([#328](https://github.com/thtbee/nectarlink/issues/328)).
- **Deleting a downloaded file in phone storage failed.** Sending it to the Recycle Bin said
  "The cloud operation was unsuccessful" and still deleted it on the phone. It now goes to
  the Recycle Bin and leaves the phone, and a file is only deleted on the phone once Windows
  has really deleted it ([#328](https://github.com/thtbee/nectarlink/issues/328)).
- **Phone as speaker could play the PC's sound twice on the phone.** It now leaves NectarLink's own sound out only while NectarLink is playing a phone's sound ([#281](https://github.com/thtbee/nectarlink/issues/281)).
- **The PC app's album art cache no longer grows without limit.** It kept every cover of everything played on the PC (166 MB after a month on one PC), and now keeps the newest 90.
- **The Elevated card on the phone is titled Elevated.** That's the name the guide, the setup and every hint use. It was titled "Real touch and keys", so people looking for Elevated couldn't find it ([#136](https://github.com/thtbee/nectarlink/issues/136)).
- **"Quiet while this PC is locked" now works on macOS.** The switch was there,
  but the app never found out whether the screen was locked, so pop-ups kept
  coming while the Mac was locked. Thanks to
  [@aydinoglucagan-sudo](https://github.com/aydinoglucagan-sudo).
- Elevated pairs on Android 17 with the right code. Another app listening on the phone
  could make it say the code didn't work
  ([#94](https://github.com/thtbee/nectarlink/issues/94)).
- Starting Elevated on Android 17 no longer overloads the phone. Looking for wireless
  debugging connected to every local port, which could make other apps fail.
- Phone sound through Elevated is no longer almost silent on the PC. It has a volume of
  its own that starts low, so NectarLink now turns it up
  ([#94](https://github.com/thtbee/nectarlink/issues/94)).
- A second title bar no longer covers the top of the window on Windows 11 23H2
  ([#227](https://github.com/thtbee/nectarlink/issues/227),
  [#209](https://github.com/thtbee/nectarlink/issues/209)).
- **Pairing could fail over Tailscale on PCs with Hyper-V, WSL, Docker or VMware,** since their
  adapters could push the Tailscale address out of the pairing code.
- **Phone sound on this PC turned itself off after a second** on phones that refuse
  Elevated's sound capture. The phone now asks you instead, as it does without
  Elevated ([#228](https://github.com/thtbee/nectarlink/issues/228)).
- **Pressing Enter or Backspace from the PC threw some apps back to their first
  screen,** Snapchat's camera for one. With Elevated these keys went through a
  virtual USB keyboard, and adding one can make Android restart an app. They're
  now sent like any other typed key, and the phone's own keyboard is no longer
  hidden after you press one.
- **On Wayland the dock showed a gear and "nectarlink-desktop"** instead of
  NectarLink's icon and name.
- **Switches under Photo backup could miss a click** while photos were copying.
  Each copied file rebuilt the row.
- **The phone's webcam preview cut off a portrait picture,** though the PC got all
  of it. The preview now shows what's being sent.
- **The Shelf and Nectar Island couldn't be moved on Wayland,** where apps can't
  place their own windows. Drag the Shelf by its header, or the Island by the pill.
- **Text from the phone could come out as one letter repeated in some Windows
  apps,** such as "ttttt" for "great" in Notepad, most often with voice typing
  ([#52](https://github.com/thtbee/nectarlink/issues/52)).

## 0.1.5 (2026-11-05)

The PC app on Linux and on Windows on ARM, Phone as microphone, Play on
phone, your phone's sound on the PC, a Nightly channel, and the fixes for
what people reported since 0.1.4.
- The phone's list of the PC's audio outputs scrolls, so a PC with many of them shows them all.
- Voice typing comes through right in Notepad on Windows 11. Words could still turn into
  repeats of one letter, because the PC typed faster than Notepad could read
  ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- Turning the phone's screen off keeps it off on Android 16. Setting the brightness to 0
  right after could wake it again, dim but lit
  ([#217](https://github.com/thtbee/nectarlink/issues/217)).
- Settings shows which App size and Full charge target are selected
  ([#260](https://github.com/thtbee/nectarlink/issues/260)).
- The taskbar shows the NectarLink icon from the moment the app opens, not a generic one
  ([#258](https://github.com/thtbee/nectarlink/issues/258)).
- On Windows, NectarLink opens again by itself after an automatic update. It closed for the
  update and stayed closed until you opened it. This takes effect from the update after
  the one that has this fix.
- Show in folder opens the right folder when a file's path has a space in it, instead of
  Documents or the Desktop ([#264](https://github.com/thtbee/nectarlink/issues/264)).
- Deck tiles and page names show in the phone's language unless you renamed them
  ([#209](https://github.com/thtbee/nectarlink/issues/209)).
- The phone's clipboard setting says Elevated is on however it was started, and takes
  you to Elevated's settings ([#110](https://github.com/thtbee/nectarlink/issues/110)).
- The phone's box for typing on the PC grows to four lines, and you can select and edit
  anywhere in it ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- A phone connected to the PC's `adb` over Wi-Fi no longer shows as USB, or sends
  everything through that slower route ([#275](https://github.com/thtbee/nectarlink/issues/275)).
- A file transfer resumed after a pause could fail with "a file got shorter while sending",
  or read from the wrong place in the file, when the paused attempt's reader was still going.
- On Linux with Wayland, the PC remembers which screen you picked to share, so Control PC
  doesn't ask again each time NectarLink restarts.
- Nectar Island's progress bar moves while music plays, instead of only when the phone
  sends an update.
- On GNOME with Wayland, Nectar Island stays on top if you have the PiP on top extension.
- Quitting the PC app with a mirror open could crash it on the way out.
- On GNOME with Wayland, text copied on the phone could be replaced by what was on the
  PC's clipboard, once NectarLink got focus.
- On Wayland, Nectar Island went back to where the desktop put it after a song was paused and
  played again. It now keeps the spot you dragged it to for 10 minutes after it goes quiet.
- On a phone rooted with KernelSU or APatch, Elevated's settings didn't offer **Use root**:
  those show root only to apps they already allow. NectarLink now also looks for their
  manager app ([#291](https://github.com/thtbee/nectarlink/issues/291)).
- With Developer Mode on, NectarLink dropped out of the Windows Share sheet after updating
  itself, without a word. It now asks once for admin rights to register again, and logs
  why it isn't in the Share sheet otherwise
  ([#84](https://github.com/thtbee/nectarlink/issues/84)).
- On Linux, quitting the PC app (or logging out) with a phone's screen or an app window
  open could crash it on the way out.
- Voice typing into Notepad on Windows no longer mixes up letters when you speak fast
  ("or not" coming out as "tttttt"): letters your keyboard layout has are typed with
  its keys ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- With the phone's keyboard open in Remote, a strip of touchpad stays above it when
  there's room, so you can move the pointer without closing the keyboard
  ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- Following the system's language, the PC app could open in German (or another language
  further down Windows' list) when English came first
  ([#280](https://github.com/thtbee/nectarlink/issues/280)).
- Phone as speaker together with the phone's sound on the PC (or a mirror with sound) no
  longer loops into loud distortion on Windows: the PC's sound sent to the phone leaves out
  NectarLink's own, and the phone
  doesn't send the PC's sound back with its own either ([#281](https://github.com/thtbee/nectarlink/issues/281)).
- The PC app does less in the background while a phone is connected. The phone's folder in
  File Explorer was set up again on every connection update, about every 15 seconds, and when
  that failed it was retried just as often. Now it's only when the phone connects or
  disconnects, and a failure waits a minute, then longer.
- On home networks numbered outside the usual private ranges, the PC reconnects to the phone
  on its own after a restart without **Connect over mobile data**. It found the phone over
  mDNS but didn't count it as on the same network
  ([#131](https://github.com/thtbee/nectarlink/issues/131)).
- Elevated starts on phones and networks where Wireless debugging can't be looked up: it's
  tried on the phone itself instead of waiting forever
  ([#147](https://github.com/thtbee/nectarlink/issues/147)).
- **Phone sound on this PC** keeps the phone awake and its Wi-Fi out of power saving, as
  mirroring already did. With only the sound going, neither was held
  ([#94](https://github.com/thtbee/nectarlink/issues/94)).
- **Enter sends messages** now sends in WhatsApp, Telegram and Messenger, whose message boxes
  ignore the keyboard's Send: it presses the app's Send button instead
  ([#220](https://github.com/thtbee/nectarlink/issues/220)).
- Elevated's pairing notification says to swipe it down for **Enter code**, which phones that keep
  it collapsed hide ([#305](https://github.com/thtbee/nectarlink/issues/305)).
- The Nectar Island keeps animating (the scrolling title and the equalizer) after the main window is
  closed to the tray ([#225](https://github.com/thtbee/nectarlink/pull/225)). Thanks to
  [@HaqmuinDox](https://github.com/HaqmuinDox).
- With two phones connected, **Phone sound on this PC** plays the phone selected on Home,
  not the first one. Thanks to [@krakotay](https://github.com/krakotay).
- Voice typing from the phone no longer loses or doubles letters in Notepad when it
  rewrites a phrase ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- **Type on phone** shows what you've typed instead of only the last key, which looked
  like it took one letter, and says to tap a text box on the phone first
  ([#116](https://github.com/thtbee/nectarlink/issues/116)).
- The PC's volume slider works on phones that ignore the normal request, like iQOO on
  FuntouchOS, when Elevated is on ([#266](https://github.com/thtbee/nectarlink/issues/266),
  [#192](https://github.com/thtbee/nectarlink/issues/192)).

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.5-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/Nectarlink-0.1.5-x64-setup.exe) | Windows | Windows 10 (2004+) or 11, 64-bit Intel or AMD (Phone as webcam needs Windows 11) |
| [Nectarlink-0.1.5-arm64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/Nectarlink-0.1.5-arm64-setup.exe) | Windows on ARM | Windows 11 on ARM (Snapdragon) |
| [nectarlink_0.1.5_amd64.deb](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/nectarlink_0.1.5_amd64.deb) | Ubuntu, Debian | Ubuntu 24.04 or newer, Debian 13, 64-bit Intel or AMD |
| [nectarlink-0.1.5-1.x86_64.rpm](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/nectarlink-0.1.5-1.x86_64.rpm) | Fedora, openSUSE | Fedora 40 or newer, 64-bit Intel or AMD |
| [Nectarlink-0.1.5-x86_64.AppImage](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/Nectarlink-0.1.5-x86_64.AppImage) | Other Linux | A distribution from 2024 or newer, 64-bit Intel or AMD |
| [Nectarlink-0.1.5-linux-x86_64.tar.gz](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/Nectarlink-0.1.5-linux-x86_64.tar.gz) | Other Linux | As the AppImage |
| [Nectarlink-0.1.5-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/Nectarlink-0.1.5-android.apk) | Android | Android 8.0 or newer |

All are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.5/SHA256SUMS.txt).
On Linux, `curl -fsSL https://github.com/thtbee/nectarlink/releases/latest/download/install.sh | sh`
picks the right one. Installed copies update themselves.

### New

- **Nix flake.** `nix run` starts the PC app on Linux, and `nix develop` gives a
  development shell. Thanks to [@m-uvex](https://github.com/m-uvex)
  ([#173](https://github.com/thtbee/nectarlink/pull/173)).
- **Delete photos from the PC.** Select them on the Photos page and they go to
  the phone's trash, where you can still restore them. It needs All files
  access on the phone
  ([#174](https://github.com/thtbee/nectarlink/issues/174)).
- **SD cards in File Explorer.** With All files access, an SD card or USB
  drive in the phone shows as a folder of its own (such as **SD card**)
  beside the internal storage's folders. Before Android 11, Android lets
  apps only read SD cards
  ([D#149](https://github.com/thtbee/nectarlink/discussions/149)).
- **Turkish.** The Android and PC apps speak Turkish (Türkçe): pick it in
  Settings, or they follow the system's language. Thanks to
  [@sedatdagg](https://github.com/sedatdagg).
- **Your phone's sound on the PC, without mirroring.** **Phone sound on this
  PC** on Home plays what the phone plays through the PC's speakers or
  headphones, with no mirror window. With Elevated it starts right away;
  otherwise the phone asks once, as for mirroring. The phone goes quiet
  meanwhile and its media volume comes back after
  ([#94](https://github.com/thtbee/nectarlink/issues/94)).
- **Sharing your PC's screen with the phone uses H.265** when both sides
  can, and H.264 otherwise. Thanks to [@AS8148](https://github.com/AS8148)
  ([#66](https://github.com/thtbee/nectarlink/pull/66)).
- **Hardware video for screen sharing on Linux.** With FFmpeg 6 to 9
  installed, sharing the PC's screen and showing a shared screen use the
  GPU (VAAPI, NVIDIA or Intel QSV) and fall back to software H.264 when it
  can't. Thanks to [@AS8148](https://github.com/AS8148)
  ([#67](https://github.com/thtbee/nectarlink/pull/67)).
- **Elevated over USB.** Plug the phone into the PC with USB debugging
  allowed and Elevated starts by itself, without setting up Wireless
  debugging. It runs while the cable is in, and needs Android's `adb` running on the PC
  (Google's platform-tools, then `adb start-server`). The PC only starts NectarLink's
  own helper, on the phone that asked
  ([#70](https://github.com/thtbee/nectarlink/issues/70)).
- **Mirroring wakes the phone's screen.** With Elevated, starting mirroring
  from the PC turns the phone's screen on if it's off, so the phone can stay
  in a pocket; a locked phone shows its lock screen in the window
  ([#82](https://github.com/thtbee/nectarlink/issues/82)).
- **Watch the PC and move its pointer at once.** Control PC's Screen tab
  on the phone has a touchpad under the PC's screen (it hides in full
  screen). Thanks to [@waytoo-average](https://github.com/waytoo-average)
  ([#104](https://github.com/thtbee/nectarlink/pull/104)).
- **Call or text a copied number from the command palette.** Copy a phone
  number anywhere, open the palette with its shortcut, and Call and Text
  for that number are at the top
  ([#83](https://github.com/thtbee/nectarlink/issues/83)).
- **Connect over Tailscale.** With Tailscale on the phone and the PC, they
  reach each other directly through it from different networks, without
  relays. Pair once while Tailscale is up on both. On
  Windows, mark the Tailscale network Private so the firewall lets it in
  (`docs/technical/tailscale.md` has the command). Thanks to
  [@waytoo-average](https://github.com/waytoo-average)
  ([#105](https://github.com/thtbee/nectarlink/pull/105)).
- **A native installer for Windows on ARM** (Snapdragon PCs):
  `Nectarlink-<version>-arm64-setup.exe`, next to the x64 one. Thanks to
  [@HosseinDianati](https://github.com/HosseinDianati)
  ([#14](https://github.com/thtbee/nectarlink/pull/14)).
- **Elevated through Shizuku.** If you use Shizuku, **Use Shizuku** in
  Elevated's settings starts it through Shizuku instead of NectarLink's own
  Wireless debugging pairing, so it keeps working away from Wi-Fi while
  Shizuku runs. Shizuku asks once whether NectarLink may use it. The helper
  still runs as the shell user, never as root
  ([#110](https://github.com/thtbee/nectarlink/issues/110)).
- **Pick the SIM for a call from the PC.** On a phone with two SIMs, the
  Calls page has **Call with** under the dial pad. It starts on the phone's
  default SIM for calls and remembers your pick for each phone. Before,
  the phone's own SIM prompt came up and the call didn't go out ([#112](https://github.com/thtbee/nectarlink/issues/112)).
- **Battery alerts for phones that stop charging at 80%.** Settings has a
  **Full charge target**: at **80% (Protect battery)**, the PC tells you the
  phone is charged when it reaches 80% on the charger. Thanks to
  [@Armone0](https://github.com/Armone0)
  ([#98](https://github.com/thtbee/nectarlink/pull/98)).
- **Nicknames for your phones on the PC.** **Set nickname** on Home names a
  paired phone for this PC only; leave it blank to use the phone's own name.
  Thanks to [@Armone0](https://github.com/Armone0)
  ([#98](https://github.com/thtbee/nectarlink/pull/98)).
- **The PC app on Linux**, for Ubuntu 24.04 or newer, Debian 13, Fedora 40
  or newer and other distributions from 2024 on, as a `.deb`, an `.rpm`,
  an AppImage or a tarball. It does what the Windows app does: notifications with
  replies, messages and calls, the clipboard, files, your phone's storage
  in the file manager, photos, media controls, mirroring and phone apps in
  windows, your PC's screen on the phone, the phone as webcam and
  microphone, and the phone as touchpad and keyboard. It works on GNOME
  and KDE, on Wayland and X11, and updates itself. A few features need a
  package or a permission first; the Connection Doctor says which. GNOME on
  Wayland doesn't give background apps the clipboard, so there it syncs only
  while NectarLink's window has focus. Thanks
  to Jochen Kirstätter for the port
  ([#34](https://github.com/thtbee/nectarlink/pull/34),
  [#38](https://github.com/thtbee/nectarlink/pull/38),
  [#41](https://github.com/thtbee/nectarlink/pull/41),
  [#42](https://github.com/thtbee/nectarlink/pull/42),
  [#43](https://github.com/thtbee/nectarlink/pull/43),
  [#44](https://github.com/thtbee/nectarlink/pull/44)).
- **One command installs NectarLink on Linux** with everything it needs,
  showing each step:
  `curl -fsSL https://github.com/thtbee/nectarlink/releases/latest/download/install.sh | sh`.
  It picks the package for your system, checks it against the release's
  sums, installs it with its dependencies, and sets up the phone as webcam,
  the phone as keyboard and mouse, and the firewall for your local network.
  `--uninstall` removes it all again.
- **The PC app on macOS, as a preview.** It builds and runs on macOS 14
  and later with pairing, files and folders, the photos browser,
  Continuity Camera, calls and SMS, the phone's toggles and volume,
  clipboard text both ways (items password managers mark private are
  never sent), photos copied on the phone, and notifications. Screen
  mirroring, recordings, PC audio, phone storage and the virtual camera
  aren't there yet, and phones show them as unavailable. Thanks to
  saivinay1805 ([#24](https://github.com/thtbee/nectarlink/pull/24)).
- **Play on phone.** A switch on Home, under Phone as speaker (and a button
  on the PC's media card), moves what plays on the PC to your phone. The
  switch waits, greyed out, until a video plays. Streaming services that protect their video
  (Netflix, Prime Video, JioHotstar, Disney+ and others) open in their
  app on the phone and the PC pauses. Anything else streams from the PC
  as it plays, picture and sound, so your browser's ad blocker still
  works on YouTube: the phone shows just the video, even with other
  windows over it on the PC, in Chrome, Edge, Firefox, Zen and other
  browsers. A video file playing in VLC, Media Player or another desktop
  player plays on the phone itself, read from the PC at its full quality,
  from where the PC was; the PC pauses. Tap the notification on the phone
  to watch.
  The PC mutes its speakers while you watch and turns them back on after
  (**Settings › Camera & Audio** to keep them on).
  Scripts can do the same through the local API
  (`POST /v1/play-on-phone`).
- **Phone as microphone.** In **Settings › Camera & Audio**, Start asks
  your phone for its microphone; tap Start there and the PC plays it into
  an output of your choice. With a virtual audio cable such as VB-CABLE
  installed, NectarLink plays into it by itself, and its other end is a
  microphone Discord, Zoom, OBS or Teams can pick
  ([#7](https://github.com/thtbee/nectarlink/issues/7)). A microphone of
  NectarLink's own, without the cable, needs an audio driver and comes
  later. A volume slider sets how loud it plays, and the phone evens out a
  quiet voice where it can. The request also shows on the phone's Home,
  with Start and Not now, so it's seen with notifications off; Not now
  tells the PC, which stops waiting, and so does a request left unanswered
  for two minutes. It can also be started from the phone: **Use phone as
microphone** sits under Use phone as speaker on Home. While it's on, Home
has a Stop button. Choosing another
  output on the PC moves the sound there right away, and **Use this phone
  as a microphone** in the phone's Settings, for each PC, turns asking off
  for that PC. Thanks to Jochen Kirstätter for finding the gaps.
- **Photo backup.** In **Settings › Files & Sync**, turn on Photo backup
  for a phone and new photos from every album are copied into a folder on
  the PC (Pictures\NectarLink\<phone> unless you choose another), filed by
  year and month, with videos if you want them. It only needs the phone's
  photos permission, copies one item at a time so a lost connection costs
  at most one file, and never deletes anything, on the PC or the phone.
  Backed-up items don't fill the Timeline.
- **Instant Hotspot joins the hotspot too.** In **Settings › Devices**,
  choose the phone's hotspot from the Wi-Fi networks this PC has saved.
  When Instant Hotspot on Home turns the phone's hotspot on, the PC joins
  it by itself. Windows keeps the password; NectarLink never sees it.
- **Turn off the background connection.** In the phone's **Settings ›
  Background & battery**, or with **Turn off** on its notification. NectarLink
  then connects only while the app is open, and the notification goes
  away. Mirroring, file transfers, the webcam and the mic still finish
  before it disconnects. Thanks to Abinanthan
  ([#22](https://github.com/thtbee/nectarlink/pull/22)).
- **Pair a phone with a tablet (or another phone).** In NectarLink's
  pairing screen, **Pair a phone or tablet instead** shows a code to scan
  from the other device. Once paired, each shows the other on Home with its
  battery, Ring, Send files and Send clipboard, and a phone's notifications
  show on the tablet; dismiss one there and it goes on the phone too.
- **The Windows app follows the system language**, and speaks Simplified
  Chinese, by [@zzz1465720680](https://github.com/zzz1465720680)
  ([#16](https://github.com/thtbee/nectarlink/pull/16),
  [#17](https://github.com/thtbee/nectarlink/pull/17)), or the language
  you pick in **Settings › General**. Its messages, errors and the
  Features guide are translated too, with the timeline's wording from
  [@zzz1465720680](https://github.com/zzz1465720680)
  ([#18](https://github.com/thtbee/nectarlink/pull/18)).
- **Disconnect from a device without unpairing it.** **Disconnect** on
  Home (**Stop reconnecting** while it isn't connected) keeps a phone or PC
  disconnected, even after a restart, until you tap **Reconnect**
  ([#48](https://github.com/thtbee/nectarlink/issues/48)).
- **The phone's volume buttons control your PC** while it plays music or
  video, in 4% steps, as with KDE Connect. When the phone plays something
  itself, or the PC is paused, they work on the phone as usual. Turn it off
  in the phone's **Settings › Control your PC**.
- **App windows bring the phone's sound to the PC.** It starts with the
  first app window and stops with the last, and the phone goes quiet
  meanwhile. The speaker button in any app window mutes it
  ([#121](https://github.com/thtbee/nectarlink/issues/121)).
- **Start the camera and microphone without asking.** For a PC you trust,
  turn it on under that PC in the phone's **Settings › PCs**: its webcam
  and microphone requests start right away, even with the screen off,
  with no tap. Needs Elevated; it's off by default
  ([#132](https://github.com/thtbee/nectarlink/issues/132)).
- **A Nightly channel.** Pick **Stable** or **Nightly** under Updates in
  Settings, on the PC and on the phone. Nightly gets a build of the latest
  code every couple of hours when it changes, each named for a flower
  ("Golden Lavender"), and releases too; Stable gets releases. Back on
  Stable, a nightly stays until the next release is newer.
- **Updates arrive sooner and cost less.** The apps read a small file per
  channel instead of GitHub's release list, and an unchanged one is a few
  hundred bytes. The PC checks every 12 hours on Stable and every 2 hours on
  Nightly (twice as long on battery), and tells your phone when there's a new
  build, so the phone hardly checks on its own.

### Changed

- **Less battery in the background.** The phone app wakes the radio far
  less often: connection checks are spaced out, looking for nearby devices
  slows down outside pairing, the phone stops listening for nearby devices
  while its screen is off, and it no longer asks the router to open ports
  on a home network. It also retries a missing PC less often, and right away
  when you unlock the phone. In our tests it sent and received about 85%
  fewer packets with the screen off. With the screen off, a PC that has just
  started can take up to two minutes to show the phone as connected.
- **Windows 10 looks right.** The window no longer shows Windows 10's own
  title bar over NectarLink's, Snap and resizing work, and the caption
  buttons show. Phone as webcam still needs Windows 11. Thanks to
  [@waytoo-average](https://github.com/waytoo-average)
  ([#103](https://github.com/thtbee/nectarlink/pull/103)).
- **Send to your phone from more Linux file managers.** Right-click files
  or folders in Nemo (Cinnamon) or Thunar (XFCE) as well as GNOME Files,
  Dolphin and Caja, and folders now work in all of them
  ([#49](https://github.com/thtbee/nectarlink/issues/49)).
- **Smaller Linux installs.** The app no longer needs the ffmpeg program:
  recordings add the phone's audio themselves. ffmpeg, the webcam driver
  and NetworkManager are only suggested now; the one-command install still
  adds the webcam driver. Thanks to
  [@jochenkirstaetter](https://github.com/jochenkirstaetter)
  ([#156](https://github.com/thtbee/nectarlink/pull/156)).
- **Setup on Windows asks about the desktop shortcut and the folder.**
  Untick **Add a desktop shortcut** to skip it. Updates leave it as you
  had it, so one you deleted no longer comes back. **Change** next to
  **Installs in** puts NectarLink on another drive or folder instead of
  Program Files, and updates go there too. Setup locks the folder like
  Program Files. If a folder above it can be changed without
  administrator rights, updates there ask for your permission each time
  ([#102](https://github.com/thtbee/nectarlink/issues/102)).
- **Files received on Windows are marked as from another device,** the way
  a browser marks a download, so Windows checks a program with SmartScreen
  before it runs and Office opens documents in Protected View.
- **Pause for banking apps uses less battery.** While the screen is off,
  the phone checks the app in front every 15 seconds instead of several
  times a second, and checks again as soon as the screen turns on. By
  [@alexfoxtm](https://github.com/alexfoxtm)
  ([#61](https://github.com/thtbee/nectarlink/pull/61)).
- **The phone app starts about twice as fast right after an update.**
  Android compiles an app installed outside the Play Store only during
  idle maintenance, often days later. With Elevated set up, NectarLink
  asks for that compile a couple of minutes after each update.
- **Mirroring and control from the PC use less CPU.** Each mirrored frame
  takes about 14% less work to draw, and permission checks for every
  touch and message are read from memory instead of the database.
- **4K for Phone as webcam shows on the PC only when the phone can stream
  it.** The PC offered 4K for every phone and camera; picked for one that
  can't, the stream stayed at 1080p and the choice moved back. The phone
  now says what the camera in use can do, and the PC's choices follow it,
  also when you switch cameras.
- **The phone's apps show at once.** The list of apps to open in windows
  was fetched with every icon each time it opened. The PC now keeps the
  last list, also for the next start, and refreshes it in the background;
  the phone keeps the icons it already drew.
- **The PC's window opens where you left it.** It comes back on the same
  monitor, at the same size and place, and maximized if it was. If that
  monitor is gone or smaller, it fits the window on screen. Thanks to
  Jochen Kirstätter ([#26](https://github.com/thtbee/nectarlink/issues/26)).

### Fixed

- **A nightly install stays on Nightly after taking a release.** One that never had its channel picked would quietly move to Stable. The channel is now remembered from the first run.
- **The Screen tab kept streaming the PC's screen after you left the app.** It carried on using data and the PC's CPU with the phone on something else or locked. It now stops when you leave and picks up when you come back.
- **Files on a slow connection, such as over a relay, kept dropping the connection and could fail with "Transfer failed after 5 attempts".** The sender only waited 30 seconds for the other device to confirm a file, however much of it was still on its way, and NectarLink's regular check on the connection closed connections that were working but full. Both now wait while data is still moving, and a transfer that's making progress no longer runs out of attempts.
- **The Screen tab lagged further and further behind on a slow connection, such as over a relay.** The PC sent more video than the link could carry and it queued on the way, so what you did showed up seconds later, and eventually the stream stopped. The phone now reports what has arrived, and the PC holds video back and lowers the bitrate to what the link carries. In a test at 700 kbit/s, frames that reached 16 seconds old now stay around half a second old. Mirroring your phone to the PC gets the same limit on delay. Both devices need this version.
- **A file resumed over a shaky connection could fail its integrity check.** When a connection dropped and came straight back, the old and new attempts could both write to the same partial file. They now take turns, and a resumed file is checked as it is on disk before it's saved.
- **Phone as webcam on phones whose encoder has no low latency mode.** The camera opened and closed again, because the encoder refused to start. NectarLink now starts it without that mode. Thanks to [@i5alidx](https://github.com/i5alidx) for finding the cause ([#202](https://github.com/thtbee/nectarlink/issues/202)).
- **The Connection Doctor could say NectarLink isn't allowed through Windows Firewall** when it was. It couldn't read any firewall rule, so it never saw the one allowing NectarLink, and its **Allow** button seemed to fail too. Thanks to [@Codlight](https://github.com/Codlight) for sticking with this ([#160](https://github.com/thtbee/nectarlink/discussions/160)).
- **Clips waiting to send are protected.** Text, images and links waiting for a device to come online were readable in the app's database, and stayed after you cleared your history. They're now encrypted like clipboard history (on Windows and Android), removed when they expire, and removed when you clear clipboard history or everything.
- **Connect over mobile data takes effect right away, both ways.** It used to need a restart: turned on, it did nothing until then, and turned off, it kept publishing your device's addresses until then. Your device's name is also no longer published. After turning it off, one idle connection to the relay, carrying nothing, stays until the next restart.
- **Delete works on any number of photos.** More than 24 at once used to hide the button.
- **Scrolling up in a conversation on the PC.** With messages of different lengths, the wheel pulled you back to the bottom.
- **Pause for banking apps could leave NectarLink's accessibility service on.**
  Android can store the service's name in a short form, and Pause didn't
  recognise it, so it turned debugging off but not the service, and banking
  apps that check for it still refused to open
  ([#147](https://github.com/thtbee/nectarlink/issues/147)).
- **A USB connection that stopped answering recovers by itself.** Before, the
  phone stayed offline until you replugged the cable. Thanks to
  [@TheGeekyM](https://github.com/TheGeekyM)
  ([#191](https://github.com/thtbee/nectarlink/pull/191)).
- **Phone storage on Linux says why it can't list files,** such as Permission
  denied when storage access isn't granted on the phone, instead of an
  input/output error. Thanks to [@TheGeekyM](https://github.com/TheGeekyM), who
  also wrote up how the mount works
  ([#185](https://github.com/thtbee/nectarlink/pull/185),
  [#186](https://github.com/thtbee/nectarlink/pull/186)).
- **Messages on the PC.** A conversation opens at the latest message and stays
  there as pictures load and after you send, one you read lately opens at once,
  and one you've read here stops showing as unread. The phone keeps its own
  count unless NectarLink is its default SMS app
  ([#176](https://github.com/thtbee/nectarlink/issues/176)).
- **Mirroring: lock the phone, and keep its sound on it.** A **Lock phone**
  button in the mirror window, and taps from the PC now reach the phone as
  finger touches, so a launcher's double tap can see them
  ([#153](https://github.com/thtbee/nectarlink/issues/153)). **Play the sound
  on the phone** gives the sound back to the phone, not just mutes it here
  ([#178](https://github.com/thtbee/nectarlink/issues/178)).
- **Unpairing a phone clears what the PC kept from it.** Its messages, call
  log, contacts, photo thumbnails and opened photos, and its transfers in the
  list. Files you saved stay, and so do the Timeline and clipboard history.
- **Nectar Island said "Transfer complete" when a transfer failed or was
  cancelled.** A failed one now says so, and a cancelled one says nothing. A
  paused one keeps showing its name and progress.
- **Add-on, Off and Connect over mobile data labels did nothing when clicked.**
  They open the right page of Settings.
- **The setup walkthrough's Connection Doctor step** showed no results, and its
  Messages switch didn't turn Messages on.
- **Mirror windows came back on the wrong monitor** when it sits left of or
  above the main one.
- **The Instant Hotspot button stayed on** when the phone couldn't turn its
  hotspot on.
- **A custom install folder** that's a link to somewhere else is refused, and
  sharing to NectarLink from other Windows apps keeps working after an update.

- **Pairing said "Couldn't reach the PC" on PCs with Hyper-V, WSL or a
  VPN.** Their virtual network adapters could fill the pairing code and
  leave out the PC's Wi-Fi or Ethernet address, and the PC listened for
  nearby phones only on one of those adapters. The code now lists the
  address the PC uses to reach your network first, and the PC looks for
  phones on all of its networks. Thanks to
  [@samsarajr](https://github.com/samsarajr) for the packet capture that
  found it ([#148](https://github.com/thtbee/nectarlink/issues/148)).
- **Your paired phone no longer shows twice on Home.** NectarLink on the
  phone also answers LocalSend, so it was listed under LocalSend nearby as
  well, with a second Send files. Paired devices are left out of that list,
  on the PC and on the phone.
- **App windows stayed empty when the phone's screen was off.** Android
  started their display asleep too, so the app never drew. Opening one now
  wakes the phone's screen first, as mirroring does
  ([#152](https://github.com/thtbee/nectarlink/issues/152)).
- **A transfer paused and quickly resumed on the receiving device could stay
  paused on the sending one** for good. It now carries on.
- **App windows failed with "invalid mirroring options"** after Keep
  phone awake or Turn phone screen off was turned on in any mirror window,
  until the PC app restarted. Both settings now reach app windows once
  they open ([#124](https://github.com/thtbee/nectarlink/issues/124)).
- **Turn phone screen off while mirroring left the screen on,** only
  dimmed, on recent Android. It now turns the panel off, on Android 15 and
  later through the system's own call for it, while the mirrored screen or
  app windows keep running
  ([#122](https://github.com/thtbee/nectarlink/issues/122)).
- **An app window could freeze on its last picture** when the PC fell
  behind decoding: it now shows a new picture at least five times a second
  even then, and the log says when nothing could be shown
  ([#125](https://github.com/thtbee/nectarlink/issues/125)).
- **A home network numbered outside the private ranges** (such as
  192.1.1.0/24) only connected with Connect over mobile data on. A
  device's address counts as this network when it announces itself there,
  and Connection Doctor names the address instead of blaming AP isolation
  ([#131](https://github.com/thtbee/nectarlink/issues/131)).
- **The Apps page showed "android:11"** on phones older than Android 11.
  It now says app windows need Android 11 or newer
  ([#134](https://github.com/thtbee/nectarlink/issues/134)).
- **A landscape-only app or game in an app window came out sideways,**
  and taps landed in the wrong place. The window now turns to the app's
  shape, so the picture is upright and taps land where you click
  ([#126](https://github.com/thtbee/nectarlink/issues/126)).
- **Clear all on the PC cleared ongoing notifications too** (a timer, a
  battery monitor), which the phone keeps, so they vanished from the PC
  until the app next updated them. They now stay, without a dismiss
  button, as on the phone.
- **The build docs registered the virtual camera with `regsvr32`,** which
  can't work. They now use the app's `--register-vcam`, and say why a
  debug build's camera fails to start. Thanks to
  [@Parth412100](https://github.com/Parth412100)
  ([#117](https://github.com/thtbee/nectarlink/pull/117)).
- **Add folder in Synced folders only ever added DCIM to Pictures.** It now
  asks which folder on the phone (with shortcuts for Camera, Screenshots,
  Download and others), where it goes on the PC, and whether to sync one
  way or both ways. Sync now also says when the phone hasn't let the PC
  into its files, instead of "Folders are up to date"
  ([#111](https://github.com/thtbee/nectarlink/issues/111)).
- **Screen readers couldn't open a step of the first-run walkthrough** from
  its step dots on the PC. Pressing one now goes to that step.
- **Typing on the phone from the PC with a non-US keyboard.** Characters
  typed with AltGr (@ and € on a French or German keyboard, say) were
  dropped in the mirror window and in Type on phone. Ctrl+C, Ctrl+X and
  Ctrl+A now copy, cut and select all on the phone, too; what you copy
  there reaches the PC with the clipboard
  ([#107](https://github.com/thtbee/nectarlink/issues/107)).
- **A big phone folder showed only part of its files** in File Explorer (a
  camera folder of thousands of photos, say). Folders now come in pages, so
  every file shows, and listing one no longer holds up the connection.
- **A transfer too big for the receiving drive started anyway** and failed
  partway. The receiver now checks the space first and the sender says
  there isn't enough.
- **The phone's keyboard didn't open after typing from the PC.** With
  Elevated, the PC's keys reach the phone through a virtual keyboard that
  stayed connected, so Android hid the on-screen keyboard in every app. It
  now goes away 10 seconds after the last key from the PC
  ([#93](https://github.com/thtbee/nectarlink/issues/93)).
- **Renaming the phone reached the PC only after the app restarted.** It
  now shows on the PC right away ([#87](https://github.com/thtbee/nectarlink/issues/87)).
- **The phone's folder in File Explorer said the network was unavailable**
  when the phone had turned it away. It now says access is denied, and a
  notification on the PC says what to turn on ([#88](https://github.com/thtbee/nectarlink/issues/88)).
- **The phone rang twice during a call** while Phone as speaker was on:
  the PC's ringtone was played on the phone as well.
- **NectarLink never appeared in the Windows Share sheet,** even with
  Developer Mode on, because its package didn't register. It does now,
  and if Windows refuses it, `sparse-package.log` in the install folder
  says why ([#84](https://github.com/thtbee/nectarlink/issues/84)).
- **An update could keep the old Phone as webcam driver.** While Windows'
  camera service has it loaded, the file can't be replaced, and setup
  quietly kept the old one, even after a restart. Setup now moves it
  aside, so the new one takes over by the next restart at the latest.
  Uninstalling removes it at the next restart instead of leaving it
  behind.
- **The Calls page showed "PC call audio connected"** on Windows, sometimes
  with a headset's name. Windows doesn't let apps carry a phone call's
  audio, so the panel is gone there; call audio stays on the phone
  ([#86](https://github.com/thtbee/nectarlink/issues/86)).
- **A Phone Status widget was listed for the Widgets board** but never
  showed anything. It's removed until a working one is built
  ([#85](https://github.com/thtbee/nectarlink/issues/85)).
- **Find my phone could ring too quietly to hear** when the alarm volume
  was low. It now rings at full alarm volume and puts your volume back
  afterwards, even if the app is closed mid-ring. Do Not Disturb still
  decides whether it sounds. By [@alexfoxtm](https://github.com/alexfoxtm)
  ([#56](https://github.com/thtbee/nectarlink/issues/56),
  [#58](https://github.com/thtbee/nectarlink/pull/58)).
- **Call controls on the PC went to the wrong call** when a second call
  came in during one: hanging up or holding from the PC acted on the
  waiting call. They now stay on the call in progress. By
  [@alexfoxtm](https://github.com/alexfoxtm)
  ([#59](https://github.com/thtbee/nectarlink/pull/59)).
- **A resumed transfer showed as paused** on the sending side until the
  other device answered.
- **The phone forgot how to wake the PC** when the PC's network dropped,
  as it does going to sleep: the PC told phones it had no wake addresses
  and they deleted the ones they had. With no network adapter up, the PC
  now leaves them alone.
- **A file saved from the PC's shared folders could be cut short** and
  still land in Downloads as if it were whole, if the file changed or the
  PC stopped answering partway. The phone now saves it only once every
  byte has arrived.
- **Wake when PC is nearby** never found the PC: the Windows app didn't
  start its Bluetooth beacon, and the phone looked for the wrong bytes in
  it. Both are fixed, and the Linux app broadcasts the beacon too
  ([#47](https://github.com/thtbee/nectarlink/issues/47)).
- **Typing on the PC from the phone lost letters** when voice typing
  rewrote what it had typed: the PC turned away the burst of backspaces
  as too fast. The PC now takes typing at full speed, and the phone
  resends anything an older PC turns away
  ([#52](https://github.com/thtbee/nectarlink/issues/52)).
- **The Windows app could stay closed after updating itself** if a newer version
  had come out since it last checked, leaving the phone offline.
- **Phone as webcam never showed up in other apps.** Windows couldn't
  start NectarLink's camera, though Settings said it was ready. It now
  appears as **NectarLink Webcam** in Zoom, Teams, OBS, the Camera app
  and others. Frames also stay on the graphics card on the way, and the
  phone sends H.265 when the PC can decode it, with H.264 otherwise.
  Thanks to [@AS8148](https://github.com/AS8148)
  ([#51](https://github.com/thtbee/nectarlink/pull/51)).
- **A file being sent from the PC showed up twice on Home**, once under
  Waiting to send with a bar that kept flickering, and again under
  Transfers. Each file now has one row with one smooth bar that shows
  the percent, sizes, speed and time left.
- With NectarLink in two profiles on one phone (a work profile, or a
  cloned app), only one of them could connect over USB, and the PC kept
  offering to pair the other. Each profile now connects over USB, and the
  phone's name says which one it is, such as "Pixel 9a (Work)"
  ([#25](https://github.com/thtbee/nectarlink/issues/25)).
- Plugging the phone in while it's connected over Wi-Fi moves the
  connection to USB sooner, and reconnecting after the phone restarts no
  longer tries a stale USB address first. PC audio sent to the phone
  drops a moment of sound under load rather than holding up everything
  else. Thanks to Jochen Kirstätter
  ([#23](https://github.com/thtbee/nectarlink/pull/23)).
- Copying text on the phone that also carried a link to a file sent the
  wrong thing to the PC, or treated it as an image. The text now comes
  through as text, and files that aren't images aren't sent as images.
  Thanks to Jochen Kirstätter
  ([#27](https://github.com/thtbee/nectarlink/issues/27)).
- The Timeline showed thumbnails only for photos saved from the phone,
  and some of those came out empty. Images you send or receive now get
  one too, files without one get an icon for their type, and Copy works
  for them. Thanks to Jochen Kirstätter
  ([#28](https://github.com/thtbee/nectarlink/issues/28)).
- Turning on LocalSend closed the Windows app.
- Play on phone froze on the phone when the player's window was
  minimized, and showed the whole screen when it couldn't find the
  window. A minimized player now goes back behind your other windows and
  keeps playing on the phone; a window it can't find stops Play on phone
  instead of showing your screen.
- Browsing the PC's files from the phone showed nothing. It works with
  **Your PC in your pocket** on, and says what to do when it can't list a
  folder.
- Samsung's Camera, and other apps that check their screen's shape,
  crashed in an app window. App windows now always have a common screen
  shape.
- With several app windows open, the phone could drop its connection to
  the PC for a moment and every window had to start again. Messages that
  keep the connection alive now go ahead of the video.
- The "sharing your screen" bar had no background.
- The PC's log filled with network adapter warnings and kept only a few
  hours.
- From an audit by [@kunal4060](https://github.com/kunal4060)
  ([#72](https://github.com/thtbee/nectarlink/pull/72) to
  [#79](https://github.com/thtbee/nectarlink/pull/79)):
  - **Text typed in one conversation could be sent in another**: the
    message box kept it when you switched. Each conversation now keeps its
    own draft.
  - LocalSend uploads now have to be the size you accepted, and a peer's
    certificate has to match its fingerprint.
  - On Windows, a phone file named like `C:notes.txt` could be saved
    outside its folder by folder sync or Continuity Camera.
  - Phone as speaker now needs the phone's Media permission, and the PC's
    sound waits for the phone that asked instead of moving to another one.
  - A paired device can no longer take over another device's pairing by
    reporting its install ID.
  - Deleting a folder sync or automation rule could delete another rule
    too.
  - Two phones with the same name shared one folder in File Explorer;
    turning one off deleted the other's. A phone's name with letters like
    "İ" could crash the app there.
  - Renaming a file on the phone or PC from the other device no longer
    replaces a file that already has that name, and the trash keeps both
    files of the same name.
  - Silencing a call from the PC could leave the phone's ringer muted if
    the app was stopped mid-call. Keypad tones sent from the PC were too
    short to register.
  - Elevated could start again after the background connection was
    turned off.
  - Reinstalling from the Linux tarball could overwrite the app with its
    launcher script, and Linux updates now only use the same kind of
    package as the one installed.
  - Updates are only downloaded from this project's GitHub releases.
  - Settings and the local API token are readable only by you on Linux and
    macOS.
  - Smaller fixes: device names are cleaned and capped, SVGs are no longer
    opened on arrival, notification history and phone artwork are capped,
    chat history isn't cleared by a message stamped in the future, a busy
    database no longer cancels a queued send, the Shelf says when a send
    is only queued, and Find my phone on the PC shows ringing for the
    whole minute.

## 0.1.4 (2026-10-17)

Steadier connections, Pause for banking apps, Simplified Chinese on Android,
and the fixes for what people ran into in the first days after launch.

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.4-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.4/Nectarlink-0.1.4-x64-setup.exe) | Windows | Windows 11, 64-bit (Windows 10 works, without Phone as webcam) |
| [Nectarlink-0.1.4-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.4/Nectarlink-0.1.4-android.apk) | Android | Android 8.0 or newer |

Both are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.4/SHA256SUMS.txt).
Installed copies update themselves.

### New

- **Pause for banking apps.** Many banking apps won't open while Developer
  options, debugging or an accessibility service is on. Choose them in
  **Settings › Control from your PC**: while one is open, NectarLink turns
  those off, and turns back on exactly what was on when you leave, even after
  a crash or a restart. A shortcut can pause first for apps that check as
  they open.
- **Simplified Chinese** for the Android app, by
  [@zzz1465720680](https://github.com/zzz1465720680)
  ([#13](https://github.com/thtbee/nectarlink/pull/13)). It follows the
  phone's language.
- **Turn the phone's screen off from an app window**, as screen mirroring
  could: the phone stays awake with its screen dark, and the apps keep
  running on the PC. **Keep phone awake** works there too.
- **Root as a way to set up Elevated.** On a rooted phone (Magisk, KernelSU
  or APatch), **Use root** in Elevated setup starts it without pairing
  Wireless debugging, and it comes back by itself after a restart or away
  from Wi-Fi. It runs with the same permissions as through Wireless
  debugging.
- **Installing past Google Play Protect.** The install guide and
  troubleshooting explain what to do when Play Protect blocks the APK with
  no **Install anyway** button, as it now does in India and many other
  countries, by [@eren-998](https://github.com/eren-998)
  ([#11](https://github.com/thtbee/nectarlink/pull/11)). The README points
  there from the downloads.

### Changed

- **Root is only used when you choose it.** On a rooted phone, NectarLink
  asked for root every time it started and ran its Elevated helper as root.
  Now nothing asks for root unless you pick **Use root**, and the helper runs
  as the shell user either way.
- The apps say "PC" instead of "Windows PC".

### Fixed

- **Far fewer disconnects.** As the phone app opened, two of its startup
  calls could race and start the phone's connection twice, with one
  identity. The two took the PC from each other every few seconds: windows
  and mirroring dropped or couldn't start, and the PC showed the phone
  connecting over and over. It now starts once.
- **Windows 10.** NectarLink and its installer start on Windows 10. They
  failed right away with "Entry Point Not Found" because the Windows 11-only
  virtual camera was required at startup. On Windows 10, everything works
  except Phone as webcam, which needs Windows 11.
- **Phone sound never falls back to the microphone.** On a phone that blocks
  capturing what it plays, Elevated recorded the microphone instead and sent
  it to the PC as the phone's sound. It now sends no sound.
- **Mirroring with Elevated starts without asking on the phone.** Starting it
  failed on the phone (it opened the helper's stream on the main thread,
  which Android refuses), so it fell back to asking you to share the screen.
- **Screen mirroring and app windows use the GPU.** They decode on the PC's
  GPU when it has one, and in software when it doesn't or if the GPU fails
  mid-stream, and convert pictures on several cores. Large phone screens and
  several app windows stay smooth with less CPU.
- **App windows and mirroring stay up.**
  - With an app open in a window on the PC, the phone went to sleep after its
    screen timeout, which stopped the app and froze the window. While a
    window is open, the phone now stays awake with its screen dimmed, and
    sleeps as usual once the last window closes.
  - While it streams, the phone keeps its Wi-Fi and processor awake, so it
    stays on the network with its screen off.
  - If the connection to the PC dropped or was replaced just as you opened an
    app window or the phone's screen, the phone gave up at once with
    "Offline". It now waits up to 10 seconds for the connection to come back.
- **Phone as webcam in 4K is smooth.** The PC decoded and converted each 4K
  frame in about 47 ms, over the 33 ms a frame has at 30 fps, so frames were
  dropped. It now decodes on the GPU when there is one and converts on several
  cores, about 19 ms a frame
  ([#6](https://github.com/thtbee/nectarlink/issues/6)).
- **Webcam resolution.** The resolution buttons on the PC and the phone
  follow what's actually streaming, whichever side changed it; held upright,
  the phone's showed none selected. 4K is offered only when the camera in
  use can output it, so switching to a front camera without 4K drops to
  1080p.
- **Typing and pasting into the phone's screen and app windows.**
  - The emoji panel (**Win+.**) and input methods for Chinese and other
    languages work while you control the phone. Only plain key presses got
    through before.
  - **Win+V** pastes. Windows marks an item picked from clipboard history as
    one for clipboard watchers to skip, and NectarLink skipped it even when
    you asked to paste it.
  - Pasting is instant instead of typing out a letter at a time, and a pasted
    line break no longer presses Enter, which sent chat messages early.
- **Typing on the PC from the phone works like a normal keyboard.** In
  Control PC, Backspace deletes, the symbols page stays open for more than one
  symbol, autocorrect and suggestions work, and letters no longer come out
  jumbled.
- **No "copied" pop-up on the phone for copies made on the PC.** With
  Elevated on, text from the PC goes on the phone's clipboard without the
  copy toast or preview Android and One UI show for every copy. Without
  Elevated, Android doesn't let an app avoid it.
- **Calls show who is calling.** An answered call lost the contact name on
  the PC, and calls made on the phone showed "Unknown caller". The name now
  stays for the whole call, the number comes from Android's call service when
  its broadcast has none, and a number not in contacts shows as the number.
- **Old calls and notifications no longer come back** after the PC sleeps or
  restarts. A call whose end the phone never recorded rang again on every
  reconnect; it now counts as over after 90 seconds of ringing. Notifications
  you clear on the PC stay cleared when the phone resends its list (on
  reconnect or Refresh), even when the phone can't remove them, as with missed
  calls and alarms. A new post from the same app shows as usual.
- Android: the app no longer crashes when you turn off notification access or
  force stop it, or when Phone as webcam's preview comes back (the same fix
  covers the QR scanner and Continuity Camera).
- Android: changing the theme in Settings no longer jumps back to Home
  ([#8](https://github.com/thtbee/nectarlink/issues/8)).
- Android: on Hive, the category chips at the top of Settings and the
  Home/Settings bar are solid, so the honeycomb and the list no longer show
  through them.
- Windows: button tooltips, such as **Pause** on the media card, show next to
  their button again instead of in the window's top-left corner.
- Pausing a transfer on the receiving device just as its last bytes arrived
  left the sending device waiting, showing it as still sending. Both now show
  it paused.

### Thanks

To [@zzz1465720680](https://github.com/zzz1465720680) for the Simplified
Chinese translation and the reports in
[#6](https://github.com/thtbee/nectarlink/issues/6) and
[#8](https://github.com/thtbee/nectarlink/issues/8),
[@eren-998](https://github.com/eren-998) for the Play Protect guide,
[@jochenkirstaetter](https://github.com/jochenkirstaetter) for testing on a
Pixel and the Linux port in progress, and everyone who reported problems on
GitHub and X.

## 0.1.3 (2026-10-15)

Hive, a new theme for both apps, and updates to the Windows app that work on
slow or unsteady connections and show how far they are.

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.3-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.3/Nectarlink-0.1.3-x64-setup.exe) | Windows | Windows 11, 64-bit |
| [Nectarlink-0.1.3-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.3/Nectarlink-0.1.3-android.apk) | Android | Android 8.0 or newer |

Both are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.3/SHA256SUMS.txt).
Installed copies update themselves. If **Update** in 0.1.2 or earlier closes
NectarLink and nothing happens, install this version from the link above
once; updates after it work from the app.

### New

- **Hive**, a theme for the Windows and Android apps: honey on charcoal in
  dark mode and on warm wax in light mode, with a living honeycomb behind the
  whole app. The comb grows from the logo, breathes slowly, and sends a soft
  pulse of light outward every few seconds, the same on the PC and the phone.
  On the PC the phone on Home shows it too. Its colors are fixed, so the
  wallpaper, artwork and phone color options are hidden while it's on.
  - Choose it in **Settings › Appearance**, or from the **Try Hive** card on
    Home. Dismissed, the card returns only after an update or a reinstall.
  - **Animated comb** (on by default) turns the motion off. The comb also
    holds still with reduced motion or battery saver on, and while the app
    isn't in front.
  - Light on resources: one GPU shader draws the comb, updated about 30
    times a second and only while the window is in front. On Android 12 and
    older, a still comb breathes.
- The installer and uninstaller share Hive's comb.

### Fixed

- **Updating the Windows app.** Clicking **Update** closed NectarLink right
  away, and the download that followed in the background could time out on a
  slow connection to GitHub's download server. The update then failed with no
  sign of it, and NectarLink reopened only minutes later. Now NectarLink stays
  open while it downloads, shows the progress in **Settings › Connection &
  About**, resumes a download that was cut off, and closes only once the
  update is downloaded and checked. If an update doesn't install, NectarLink
  reopens and says so.
- Slow responses from GitHub no longer fail update checks or downloads: the
  Windows app waits longer and tries again.
- Album art no longer goes blank on the Windows app's Home media card after
  the track changes or the card switches between the phone and this PC. The
  same fix covers rounded images in notifications, calls, messages and the
  shelf.
- Elevated on Android reconnects by itself after Wi-Fi drops or changes. It
  used to stay on **Try again** until tapped, even with Wireless debugging
  back on. Now it starts again as soon as Wireless debugging is on, and keeps
  trying while the phone is on Wi-Fi.

## 0.1.2 (2026-10-15)

A smoother update on both devices, and the phone reconnects on its own after
an update or a restart.

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.2-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.2/Nectarlink-0.1.2-x64-setup.exe) | Windows | Windows 11, 64-bit |
| [Nectarlink-0.1.2-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.2/Nectarlink-0.1.2-android.apk) | Android | Android 8.0 or newer |

Both are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.2/SHA256SUMS.txt).
Installed copies update themselves.

### Fixed

- **The phone reconnects after an update or a restart.** Android closes an
  app while it updates, and nothing brought NectarLink's connection back until
  the app was opened again, so the PC showed the phone as gone. The connection
  now comes back on its own, after an update and after the phone restarts. A
  notification says the update is done; tapping it opens the app.
- **Check for updates in the phone's Settings shows what it found**: that
  NectarLink is up to date, or the update with its **Update** button and
  progress, or why it couldn't check. Before, the button greyed out and nothing
  appeared.
- **Allowing installs continues the update.** When Android first asks to let
  NectarLink install updates, coming back from that screen now carries on with
  the update instead of staying on **Allow**.
- **Phone downloads resume.** A dropped or changed connection no longer fails
  the update: the download waits for the network and continues where it
  stopped, also after the app restarts. Progress shows the size downloaded.
- **Updates keep going in the background.** Leaving the screen no longer stops
  a phone update that's downloading. Saying no to Android's install prompt
  leaves the update ready to install instead of reporting an error.
- **The PC app comes to the front after updating.** It reopened behind other
  windows; now it comes forward and a notification says it's updated. The same
  fix brings the window forward when NectarLink is opened while already
  running.

## 0.1.1 (2026-10-15)

Fewer steps when sharing from the phone, clearer setup on phones that hold
back permissions, and Graphite as the phone's default look.

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.1-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.1/Nectarlink-0.1.1-x64-setup.exe) | Windows | Windows 11, 64-bit |
| [Nectarlink-0.1.1-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.1/Nectarlink-0.1.1-android.apk) | Android | Android 8.0 or newer |

Both are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.1/SHA256SUMS.txt).
Installed copies update themselves.

### Changed

- **Sharing from the phone sends right away.** Choosing NectarLink in the
  Android share sheet starts the transfer immediately, with no Flick to send
  card or Undo countdown. With more than one PC connected, you pick one first.
  Progress shows in a notification with a **Cancel** button, followed by a
  **Sent to** confirmation that clears itself.
- **Files shared from the phone no longer open on the PC by themselves.** A
  shared photo or document is saved to `Downloads\Nectarlink` and announced in
  a notification. To have it open in its default app as it arrives, turn on
  **Settings › Files & Sync › Open files shared from the phone** (off by
  default).
- **Graphite is the phone app's default theme**, in light or dark to match the
  phone. Bloom and Glass remain in **Settings › Appearance**, and a theme you
  already picked stays as it is. The Windows app is unchanged.

### Fixed

- **Restricted settings during phone setup.** On Android 13 and newer, an app
  installed from a file can't get notification access or the accessibility
  service, and on Android 15 and newer the SMS permission, until the user
  allows restricted settings. When that happens, setup now explains it and
  offers **Open app info**: tap ⋮, then **Allow restricted settings**, and
  come back to grant.
- **Manage buttons in phone setup.** For a permission that's already granted,
  **Manage** now opens the screen where it can be changed: notification access,
  App info, All files access or Accessibility. Before, it did nothing.
- Release notes on GitHub keep their paragraphs instead of breaking at every
  line of the changelog.

## 0.1.0 "Pollen" (2026-10-14)

The first public release of NectarLink: the Windows app, the Android app and
the `nectarlink` command-line client. NectarLink connects an Android phone and
a Windows 11 PC directly over your network or a USB cable, end-to-end
encrypted, with no account and no cloud service.

### Downloads

| Download | For | Needs |
|---|---|---|
| [Nectarlink-0.1.0-x64-setup.exe](https://github.com/thtbee/nectarlink/releases/download/v0.1.0/Nectarlink-0.1.0-x64-setup.exe) | Windows | Windows 11, 64-bit |
| [Nectarlink-0.1.0-android.apk](https://github.com/thtbee/nectarlink/releases/download/v0.1.0/Nectarlink-0.1.0-android.apk) | Android | Android 8.0 or newer |

Both are listed with their SHA-256 hashes in [SHA256SUMS.txt](https://github.com/thtbee/nectarlink/releases/download/v0.1.0/SHA256SUMS.txt).

The Android app is signed with the project's release key. The Windows
installer isn't code-signed yet, so SmartScreen asks once: choose **More
info**, then **Run anyway**.

### Setup and pairing

- **Installer.** A setup window of its own, light or dark to match Windows.
  One click installs, with one administrator prompt. Setup adds the Windows
  Firewall rule for local connections, Start menu and desktop shortcuts,
  **Send to** in File Explorer and the update task, and starts NectarLink
  with Windows. **What this installs** on the
  first screen lists all of it. The last screen shows a QR code that opens the
  Android app's download page.
- **Uninstaller.** Removes everything setup added, along with the app's
  settings and pairings on the PC. Files you received and voice recordings
  stay where they were saved.
- **First-run walkthrough** in both apps: a privacy summary, installing the
  phone app, pairing, choosing what to share, and an optional PC check
  (notifications, Start with Windows, firewall). It resumes where you left off
  and can be reopened from Settings.
- **Features Guide** in both apps: every feature, whether it works with your
  paired phone right now, what would enable it, and a **Show me** button that
  goes to its control.
- **Pairing** by QR code (from the NectarLink app or any camera app), by
  picking a nearby device and comparing a 6-digit code, or automatically over
  a USB cable. Pairing links opened from other apps ask before pairing.
- **Power levels** on Android:
  - **Standard**: ordinary Android permissions.
  - **Assist**: adds the accessibility service, so the PC's mouse and
    keyboard can tap, swipe and type on the phone.
  - **Elevated**: a one-time Wireless debugging setup, no root. Adds real
    touch and key input, phone apps in their own windows, background clipboard
    sync, one-time codes on Android 15 and newer, Wi-Fi and Bluetooth
    switches, and in-call mute, hold and speaker.
- **Connection Doctor** in both apps: checks the firewall rule, the network
  profile, VPN routing, Wi-Fi client isolation and Android battery
  restrictions, and fixes what it can.

### Notifications

- The phone's notifications on the PC, as Windows popups and in a live feed
  on Home, grouped by app and conversation.
- Inline replies and notification actions from the PC.
- Dismissed on one device, dismissed on the other.
- Live Updates: progress bars, timers, deliveries and rides, kept current.
- One-time codes are detected and get a **Copy code** button. They are never
  kept in clipboard history.
- Pictures attached to notifications, such as photos in chat messages.
- Dismissed notification history, per-app mute rules and notification
  profiles.
- Optional: the PC's own notifications forwarded to the phone.

### Messages

- SMS and MMS conversations on the PC: read, reply, send pictures and start
  new conversations, with contact search.
- Chats from RCS, WhatsApp, Telegram, Signal and other messaging apps in the
  same inbox, answered through each app's own reply.
- Open conversations update as new texts arrive.

### Calls

- An incoming call banner on the PC with caller ID: answer, decline, or
  decline with a text message.
- Hang up from the PC. With Elevated: mute, hold, speakerphone and the keypad.
- Call any number or contact through the phone from the PC dialer.
- Recent calls and contacts.
- Call audio stays on the phone (see Known limitations).

### Clipboard

- Copy on one device and paste on the other: plain text, formatted text,
  images and copied files.
- On the phone: the **Send clipboard** Quick Settings tile, the share sheet
  and the home-screen widget. With Elevated, copies on the phone reach the PC
  in the background.
- Context chips on the PC for what was copied, such as **Open** for a link.
- Optional history of the last 50 items, encrypted on disk.
- Passwords from password managers are never sent, and anything Android marks
  as sensitive stays on the phone.

### Files and photos

- Files and folders in both directions at Wi-Fi speed. Transfers can be
  paused, resume after a dropped connection, and are verified with BLAKE3.
- **Send to** in File Explorer, drag and drop, and the share sheet on the
  phone.
- **Waiting to send**: files, clipboard items, links and alerts sent while
  the other device is offline are delivered in order when it reconnects.
- The phone's storage in File Explorer, and the PC's files from the phone.
- Folder sync between the phone and the PC.
- LocalSend compatibility, to send to and receive from LocalSend devices.
- Photos: the phone's camera roll, screenshots and albums on the PC, with
  full-resolution viewing, saving several at once, dragging into other apps,
  and alerts for new photos and screenshots.

### Screen mirroring and app windows

- The phone's screen in a window on the PC at up to 60 frames per second
  (H.264, H.265 or AV1, hardware decoded), with the phone's sound.
- Control it with the PC's mouse and keyboard (Assist or Elevated).
- Screenshots and MP4 recordings; keep the phone awake, or turn its screen off
  while mirroring.
- **Apps**: open any phone app in a window of its own on the PC, with its own
  taskbar icon, running beside whatever the phone shows (Elevated, Android 11
  and newer).
- Desktop mode (Samsung DeX and Android desktop mode) in a PC window.
- The PC's screen on the phone.

### Your PC from the phone

- Touchpad, air mouse, keyboard with a modifier bar, voice typing, and a
  presentation remote with a laser pointer.
- **Deck**: a grid of buttons for the PC (media, shortcuts, apps, commands and
  OBS Studio), with live volume and mute.
- Lock, sleep and wake the PC, including Wake-on-LAN.
- **Desk clock**: a stand display for the phone on a charger or desk stand,
  with the time, the PC's media and quick actions.

### Camera and sound

- **Phone as a webcam** through the NectarLink Webcam virtual camera, in
  Zoom, Teams, Meet, OBS and the Camera app, at up to 4K where the phone
  supports it, with lens and zoom controls.
- **Continuity Camera**: take a photo or scan a document with the phone, with
  corner cropping and a black-and-white mode, and have it on the PC's
  clipboard.
- **Phone as a speaker** for the PC's sound.
- **Voice recorder**: record on the phone with timestamped markers and save
  to the PC as M4A, MP3, WAV or FLAC.

### Across both devices

- **Handoff**: links, map locations, videos at the same timestamp, and
  documents open on the other device.
- Media controls in both directions.
- **Type on phone**: type into the phone with the PC's keyboard, without
  mirroring.
- Phone controls on PC Home: Do Not Disturb, ringer, volume, flashlight,
  brightness, Wi-Fi and Bluetooth.
- Battery levels and low-battery alerts, and **Find my device** to ring the
  phone.
- **Timeline**: a searchable history of everything that moved between your
  devices.

### On the PC

- Three themes, **Bloom**, **Glass** and **Graphite**, each in light and dark.
  The accent color can follow the desktop wallpaper, the album art of what's
  playing, or the paired phone's colors, or be one of five presets.
- **Command Palette** (`Ctrl+Alt+Space`) for every action, from any app.
- **Shelf** (`Ctrl+Alt+S`): a slide-out panel with the phone's latest photo,
  screenshot, clipboard item and recent files, plus Flick to send.
- **Nectar Island**: an optional pill at the top of the screen for calls,
  transfers, media, timers and alerts.
- The system tray and a taskbar Jump List.
- Automations: trigger-action rules, and `nectarlink notify-when` for an
  alert on the phone when a long task finishes.
- An opt-in local HTTP API on `127.0.0.1` with a bearer token, and a PowerToys
  Command Palette extension.
- Keyboard navigation and screen reader labels throughout.

### On the phone

- Material You colors from the wallpaper, in light and dark.
- A home-screen widget and a Quick Settings tile.
- A foreground service keeps the connection, with battery settings explained
  per manufacturer.

### Command-line client

`nectarlink` pairs (`pair`, `join`, `nearby`), lists and unpairs devices,
shows what works with a device (`caps`) and what it may do (`toggle`,
`toggles`), sends files and clipboard content, rings a device, controls
media, acts on notifications, and can stand in for a phone (`--as-phone`) for
testing. It builds on Windows and Linux.

### Privacy and security

- Local-first: devices connect directly over the local network or USB.
  Nothing passes through a NectarLink server, and there is no account.
- Every connection is QUIC with TLS 1.3, authenticated with Ed25519 device
  keys exchanged at pairing. Keys are protected by Windows DPAPI and the
  Android Keystore.
- Each paired device has its own feature switches. Remote control of the PC,
  shell commands and access to the phone's storage are off until you allow
  them.
- **Connect over mobile data** (off by default) keeps paired devices connected
  across networks through relays that carry only encrypted traffic.
- The only other internet traffic is the update check against GitHub
  Releases, which can be turned off.

### Updates

- Both apps check GitHub Releases once a day and verify every download
  against `SHA256SUMS.txt`. The Android app also refuses an APK not signed
  with its own key.
- **Install updates automatically** is on by default. On Windows, updates
  install through a system task once nobody has used the PC for 10 minutes and
  nothing is in progress, with no administrator prompt. Both settings can be
  turned off.

### Known limitations

- Call audio stays on the phone: Android doesn't let apps capture cellular
  call audio, and Windows reserves its Bluetooth hands-free API for its own
  apps.
- Without Elevated, Android doesn't let apps read the clipboard in the
  background. Use the Quick Settings tile, the share sheet or the widget.
- Chats from messaging apps show messages that arrive while connected. Older
  history and new chats need the app itself, for example in an app window.
- Apps that protect their screens, such as banking and streaming apps, show
  black when mirrored.
- The Windows Share sheet and the Widgets board need package identity from a
  signed installer. **Send to** in File Explorer and the Shelf work on every
  install.
- Wake-on-LAN from shutdown usually needs a wired connection.
- Windows adds its own copy button to some code notifications, so a popup can
  show two.

See [Known limitations](https://github.com/thtbee/nectarlink/blob/main/docs/guide/known-limitations.md) for the details.
