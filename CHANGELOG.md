# Changelog

All notable changes to NectarLink. Versions follow
[semantic versioning](https://semver.org); before 1.0, a minor version may
change things that need a step from you, and its notes say so.

## Unreleased

### New

- **Turkish.** The Android and PC apps speak Turkish (Türkçe): pick it in
  Settings, or they follow the system's language. Thanks to
  [@sedatdagg](https://github.com/sedatdagg).
- **Your phone's sound on the PC, without mirroring.** **Phone sound on this
  PC** on Home plays what the phone plays through the PC's speakers or
  headphones, with no mirror window. With Elevated it starts right away;
  otherwise the phone asks once, as for mirroring. The phone's own speaker
  keeps playing for now
  ([#94](https://github.com/thtbee/nectarlink/issues/94)).
- **Sharing your PC's screen with the phone uses H.265** when both sides
  can, and H.264 otherwise. Thanks to [@AS8148](https://github.com/AS8148)
  ([#66](https://github.com/thtbee/nectarlink/pull/66)).
- **Elevated over USB.** Plug the phone into the PC with USB debugging
  allowed and Elevated starts by itself, without setting up Wireless
  debugging. It runs while the cable is in. The PC only starts NectarLink's
  own helper, on the phone that asked
  ([#70](https://github.com/thtbee/nectarlink/issues/70)).
- **Mirroring wakes the phone's screen.** With Elevated, starting mirroring
  from the PC turns the phone's screen on if it's off, so the phone can stay
  in a pocket; a locked phone shows its lock screen in the window
  ([#82](https://github.com/thtbee/nectarlink/issues/82)).
- **Call or text a copied number from the command palette.** Copy a phone
  number anywhere, open the palette with its shortcut, and Call and Text
  for that number are at the top
  ([#83](https://github.com/thtbee/nectarlink/issues/83)).
- **The PC app on Linux**, for Ubuntu 24.04 or newer, Debian 13, Fedora 40
  or newer and other distributions from 2024 on, as a `.deb`, an `.rpm`,
  an AppImage or a tarball. It does what the Windows app does: notifications with
  replies, messages and calls, the clipboard, files, your phone's storage
  in the file manager, photos, media controls, mirroring and phone apps in
  windows, your PC's screen on the phone, the phone as webcam and
  microphone, and the phone as touchpad and keyboard. It works on GNOME
  and KDE, on Wayland and X11, and updates itself. A few features need a
  package or a permission first; the Connection Doctor says which. Thanks
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

### Changed

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
- **Wake when PC is nearby** never found the PC: the Windows app didn't
  start its Bluetooth beacon, and the phone looked for the wrong bytes in
  it. Both are fixed, and the Linux app broadcasts the beacon too
  ([#47](https://github.com/thtbee/nectarlink/issues/47)).
- **Typing on the PC from the phone lost letters** when voice typing
  rewrote what it had typed: the PC turned away the burst of backspaces
  as too fast. The PC now takes typing at full speed, and the phone
  resends anything an older PC turns away
  ([#52](https://github.com/thtbee/nectarlink/issues/52)).
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
