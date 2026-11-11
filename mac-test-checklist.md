# Testing NectarLink on a Mac

For the M4 MacBook. About 30 to 40 minutes. You need the Mac, your Android
phone with NectarLink, and both on the same Wi-Fi.

## 1. Get the app

1. On the Mac, open the latest `mac-port` build:
   https://github.com/thtbee/nectarlink/actions?query=branch%3Amac-port
   Open the newest green run, scroll to **Artifacts**, and download
   **nectarlink-macos-arm64** (you need to be signed in to GitHub).
2. Double-click the downloaded zip, then the `NectarLink-...-macos-arm64.zip`
   inside it. Drag **NectarLink.app** into **Applications**.
3. The app isn't signed with an Apple developer ID yet, so macOS blocks it the
   first time. Open Terminal and run:
   ```
   xattr -dr com.apple.quarantine /Applications/NectarLink.app
   ```
   Then open it from Applications. (Or: try to open it once, then System
   Settings › Privacy & Security, scroll down, **Open Anyway**.)

## 2. Permissions

Allow each one when macOS asks, and write down any that never appear:

- [ ] **Local network** ("NectarLink would like to find devices on your local
      network"). Without it the phone can't be found.
- [ ] **Notifications**.
- [ ] Later, if asked: microphone, camera, automation (controlling media apps).

## 3. Pair

- [ ] On the phone: NectarLink › Home › the QR button (top right) › scan the
      code the Mac shows. Or pick the Mac in the nearby list and check that
      the 6-digit code matches on both.
- [ ] The Mac shows the phone as connected, with its battery.

## 4. What should work (mark each: works / doesn't / partly, with a note)

| # | Try this | Expect |
|---|---|---|
| 1 | Send a photo from the phone (Send files) | It arrives on the Mac and opens |
| 2 | Send a folder from the Mac to the phone | All files arrive |
| 3 | Copy text on the Mac | It appears on the phone's clipboard |
| 4 | Copy text on the phone, then Send clipboard | It appears on the Mac |
| 5 | Take a photo on the phone | It shows up on the Mac (photos copied on the phone) |
| 6 | Photos page on the Mac | The phone's albums and photos load; download one |
| 7 | Get a notification on the phone (a message) | It shows on the Mac as a banner and in the app |
| 8 | Messages page | Conversations load; send a text to yourself |
| 9 | Calls page | Recent calls load |
| 10 | Phone controls (Wi-Fi, Bluetooth, DND, volume) | Changing one on the Mac changes it on the phone |
| 11 | Continuity Camera (take a photo or scan with the phone from the Mac) | The picture lands on the Mac |
| 12 | Ring the phone (Find phone) | The phone rings; Stop works |
| 13 | Quit the app and open it again | It reconnects by itself |
| 14 | Switch the language in Settings › General › Language (German, Chinese) | The UI switches language |
| 15 | Turn Wi-Fi off on the phone for 30 s, then on | The Mac shows it offline, then back |

## 5. Not built for the Mac yet (they should say so, not fail)

Check that each shows as unavailable instead of doing nothing or crashing:

- [ ] Screen mirroring and phone apps in windows
- [ ] Recording the screen
- [ ] PC audio on the phone (phone as speaker) and phone as microphone
- [ ] Phone storage in Finder
- [ ] Phone as webcam (virtual camera)

## 6. Note anything that looks wrong

- Text cut off, overlapping, wrong colors in light or dark mode
  (System Settings › Appearance), the window too small or too big
- Anything slow, stuck, or crashing

## 7. Send back

- Your marks for every row above
- Screenshots of anything wrong (Shift-Cmd-4, then Space, click the window)
- The log: in Finder, Go › Go to Folder…, paste
  `~/Library/Application Support/Nectarlink/logs`, and send the newest file.
  It doesn't contain your messages or notifications.

## 8. Clean up (on your friend's Mac)

1. Quit NectarLink (menu bar icon › Quit).
2. Drag NectarLink from Applications to the Trash.
3. Remove its data: Go › Go to Folder…,
   `~/Library/Application Support/Nectarlink`, move that folder to the Trash.
4. On the phone: Settings › Devices, remove the Mac.
