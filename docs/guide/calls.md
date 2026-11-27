# Calls

The **Calls** page on your PC lets you see incoming phone calls with caller ID, answer or decline them from your desk, reject a call with a quick SMS reply, dial numbers from a keypad, browse your recent call history and contacts, and control active calls.

<p align="center">
  <img src="images/calls-page.png" alt="Calls page on Windows showing recent call history, contacts, and dialer keypad" width="760">
</p>

## What You Need

| Requirement | Details |
| :--- | :--- |
| **Android Permissions** | **Calls on your PC** (`READ_PHONE_STATE`, `READ_CALL_LOG`, `ANSWER_PHONE_CALLS`, `CALL_PHONE`) and **Contacts on your PC** (`READ_CONTACTS`) in **Settings › Permissions › Calls, contacts and messages**. |
| **Android Power Level** | `Standard` for incoming call alerts, caller ID, answer, decline, hang up, call log, dialer, and volume control. `Elevated` (on Android 12+) enables in-call **Mute**, **Speakerphone**, **Hold**, and **Keypad (DTMF)** controls from the PC, and (on Android 13+, Windows) talking through your PC. |
| **Per-Device Toggles** | **Calls** and **Contacts** (both enabled by default in **Settings › Devices** on the PC). |
| **Network** | Local network, USB cable, or **Connect over mobile data**. |

## Step-by-Step Use

### Handling Incoming Calls on Your PC

1. When your phone rings, NectarLink displays an incoming call banner on your PC (and expands `Nectar Island` if enabled) showing the caller's name, phone number, and contact photo.
2. If **Pause media during calls** is enabled in **Settings › Camera & Audio**, any music or video playing on your PC pauses automatically while the phone is ringing or in a call, and resumes when the call ends.
3. From the incoming call banner on your PC, you can:
   - Click **Answer** to pick up the call on your phone.
   - Click **Decline** to send the caller to voicemail.
   - Click **Silence** to mute the ringer without declining the call.
   - Click **Reply with text** (when SMS access is enabled) to decline the call and immediately send a preset or custom SMS reply to the caller.

### Controlling an Active Call

While a call is active on your phone, the call bar on your PC shows the live call timer and provides controls based on your phone's power level:

- **At `Standard` or `Assist`**: You can see the active call duration, adjust the call volume, and click **Hang up** to end the call.
- **At `Elevated` (Android 12 and newer)**: Additional in-call controls appear on the PC card, allowing you to toggle **Mute**, switch the phone to **Speaker** (speakerphone), place the call on **Hold**, or open the in-call **Keypad** to send DTMF tones for automated phone menus.

### Placing a Call from Your PC

1. Open **Calls** in the left navigation bar on your PC.
2. You can start a call in three ways:
   - **From the Keypad**: Type or paste a phone number into the dialer keypad (or press number keys on your keyboard) and click **Call**.
   - **With two SIMs**: Pick the SIM under **Call with** next to the keypad. The PC remembers it for that phone and uses it for every call it starts, so the phone doesn't stop to ask which SIM.
   - **From Contacts**: Search your synced phone contacts (favorites appear at the top) and click the call button next to any number.
   - **From Recent Calls**: Click any entry in your recent call history to call back or open a text conversation with that person.
3. Your phone places the cellular call immediately.

### Browsing Call History

The **Calls** page groups your phone's recent incoming, outgoing, and missed calls by day. Missed calls are highlighted in red and summarized on your PC's Home screen until viewed. The **Calls** item in the left rail shows a badge with the number of missed calls you haven't looked at, and it goes away when you open the page. A voicemail notification from your phone appears in the notification feed like any other notification.

### Talking Through Your PC (Windows)

With `Elevated` on Android 13 or newer, the call in progress can move to your PC: click **On PC** on the call card or in Nectar Island. You hear the other person from your PC's speakers or headphones and talk into its microphone, over NectarLink's own encrypted connection, without Bluetooth. Your phone shows a notification that the call's sound is on your PC; tap **Back to phone** there, or click **On PC** again on the PC, to move it back. A call you answer or dial from the PC moves there on its own. When the call ends, everything goes back on its own.

Use headphones if you can: on speakers, your PC's microphone also picks up the other person.

Calls use Windows' default communications microphone. To pick another one, go to **Settings › Camera & Audio › Microphone for calls**; a change applies to a call that's already on the PC too.

If Windows' default communications microphone is off or missing, NectarLink uses the default microphone instead. If NectarLink says Windows keeps it off the microphone, turn on **Settings › Privacy & security › Microphone › Let desktop apps access your microphone** in Windows.

Whether your phone allows this is up to its maker: the phone's audio system must let apps reach the call (Android calls this call audio interception). It works on Google Pixel phones. NectarLink checks this when Elevated starts; on a phone that doesn't allow it, the PC shows no **On PC** button. Details are in [`docs/research/hfp.md`](../research/hfp.md).

On Linux, call audio goes to the PC over Bluetooth instead, as for a headset.

## What the Options Do

- **Calls toggle** (`Settings › Devices` on PC, **On** by default): Enables caller ID, call log sync, incoming call alerts, and call control for that phone.
- **Contacts toggle** (`Settings › Devices` on PC, **On** by default): Syncs your phone's contacts so caller ID and the dialer show names and photos.
- **Also use the Mac's Contacts** (`Settings › Calls` on a Mac, **Off** by default): The phone's contacts are never copied to the PC: the phone looks up each caller and conversation itself, so a contact you just added on the phone shows at once, and the PC reloads its lists when the phone says its contacts changed. This setting adds the people only the Mac's Contacts app knows (macOS asks once): a caller the phone has no name for gets their name and picture, in the call notification, on Home and in `Nectar Island`, and the dialer and the new-message box list them with the phone's. A person both have is one contact, with the phone's name. Numbers match however they're written, so `+33 6 12 34 56 78` and `06 12 34 56 78` are one number. These contacts never leave the Mac.
- **Pause media during calls** (`Settings › Camera & Audio` on PC, **On** by default): Automatically pauses active Windows media playback when a call rings or connects on your phone, and resumes playback when the call finishes unless you manually changed playback in the meantime.
- **Island triggers › Calls** (`Settings › General` on PC, **On** by default when `Nectar Island` is enabled): Shows incoming and active phone calls inside the top-of-screen `Nectar Island` pill.

## Data and Privacy

- **Where data travels**: Call state changes, caller ID metadata, recent call log entries, and contact records travel directly between your phone and PC over your end-to-end encrypted QUIC connection.
- **Where data stays**: Recent call logs and contacts are read live from your phone and held in memory on the PC during your session. NectarLink never records call audio: when a call sounds on your PC, its sound only passes through while the call goes on.

## Limits

- **Talking through your PC**: Needs `Elevated` on Android 13 or newer, and a phone whose audio system lets apps reach the call (Pixel phones do; others may not). On Windows only, for now. There's no echo cancelling yet, so headphones work best.
- **In-Call Mute, Hold, Speaker, and DTMF Keypad**: Standard Android permissions allow third-party apps to answer and end calls (`TelecomManager`), but muting the microphone, toggling speakerphone, holding a call, and sending DTMF tones during a cellular call require `Elevated` on Android 12 or newer unless the app replaces your default phone dialer.
- **Dual-SIM Selection**: When you place a call from the PC on a dual-SIM phone, Android uses the default calling SIM configured in your phone's SIM settings (or prompts on the phone if your SIM preference is set to "Ask every time").
