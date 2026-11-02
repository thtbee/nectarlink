# Bluetooth HFP & PC Call Audio Architecture (`docs/research/hfp.md`)

## 1. Why Android Blocks Software Wi-Fi Call Audio Capture

On stock Android (non-rooted devices without OEM system-signature privileges), cellular and PSTN voice call audio (`AudioAttributes.USAGE_VOICE_COMMUNICATION` / `AudioSource.VOICE_CALL` / `AudioSource.VOICE_DOWNLINK` / `AudioSource.VOICE_UPLINK`) is hardware-routed by the baseband/modem DSP and the Android audio HAL:

1. **`CAPTURE_AUDIO_OUTPUT` & `CONTROL_INCALL_EXPERIENCE` are signature/privileged permissions**: Third-party apps, even when granted `READ_PHONE_STATE`, `ANSWER_PHONE_CALLS`, `CALL_PHONE`, or `InCallService` role, cannot read the downlink voice stream or inject uplink microphone PCM into an active cellular call over Wi-Fi/LAN.
2. **`AudioPlaybackCapture` (`MediaProjection`) excludes voice calls**: Android 10+'s `AudioPlaybackCaptureConfiguration` explicitly blocks capturing `USAGE_VOICE_COMMUNICATION` and `USAGE_VOICE_COMMUNICATION_SIGNALLING`.
3. **The shell user (`Elevated`) may record the call sources, but whether they carry the call depends on the phone**: `shell` holds `CAPTURE_AUDIO_OUTPUT` (the Elevated helper already records `REMOTE_SUBMIX` with it), which is the permission `AudioPolicyService` asks for `VOICE_CALL`, `VOICE_UPLINK` and `VOICE_DOWNLINK`. Checked on 2026-11-02 on an Android 17 emulator: a recorder on each of the three, created from `app_process` as `shell`, initializes and records, during a call and outside one. The emulator's audio HAL feeds its microphone input there either way, so it can't show whether a real phone's HAL delivers the call; that is decided per device, in the audio HAL and the modem's DSP. On a real phone (Samsung Galaxy A35, SM-A356E, Exynos 1380, Android 17, Jio VoLTE call to an automated line, 2026-11-02), `VOICE_DOWNLINK` and `VOICE_CALL` carried the other side's audio as `shell` (RMS 280 to 3500 against 8 to 18 with no call, not silenced), and `VOICE_UPLINK` returned only zeros.
4. **Feeding sound into a call is unconfirmed**: apps have no public API for it. Phones expose a telephony output (`AudioDeviceInfo.TYPE_TELEPHONY`), which system features use to play into a call. On the Galaxy A35 above, a `shell` `AudioTrack` with `USAGE_VOICE_COMMUNICATION` was routed to it during the call without an error, but whether the other side hears it is untested (the phone's call recordings carry only the other side, so they can't show it).

Consequently, hearing a cellular call on the PC works with `Elevated` on phones whose audio HAL delivers the call to those sources (confirmed on one Samsung), and talking into it from the PC's microphone without Bluetooth is unconfirmed.

---

## 2. Why Public Windows WinRT / Win32 APIs Cannot Route Two-Way HFP Call Audio for Unpackaged Apps

We evaluated three Windows Bluetooth APIs for using the PC's microphone and speakers as a Bluetooth Hands-Free Profile (HFP) Hands-Free (HF) unit during a cellular call:

1. **`Windows.Media.Audio.AudioPlaybackConnection` (Windows 10 2004+) is A2DP Media Sink Only**:
   - `AudioPlaybackConnection` activates the inbox Bluetooth **A2DP Sink** (`AvrcpTransport` / media playback) role so a paired phone can stream media audio *to* the PC.
   - It is **not** a bidirectional Bluetooth HFP Hands-Free unit or Audio Gateway API: it does not establish a Synchronous Connection-Oriented (SCO / eSCO) voice channel, does not route the PC's microphone back to the phone's cellular call uplink, and does not switch the phone's `InCallService` audio route to Bluetooth Hands-Free.
2. **`Windows.ApplicationModel.Calls.PhoneLineTransportDevice` (Broken / Locked Down on Windows 11 22H2+)**:
   - Microsoft originally introduced `PhoneLineTransportDevice` for companion apps to bind to `bthhfenum.sys` (`Microsoft Bluetooth Hands-Free Profile Handsfree role`).
   - Starting in Windows 11 22H2, `PhoneLineTransportDevice::RequestAccessAsync()` and `Connect()` are gated behind internal Windows inbox broker checks (`YourPhone.exe` / `PhoneExperienceHost` system identity and restricted `phoneCallTransport` / `phoneCallOrigin` capabilities) and fail with `E_ACCESSDENIED` for third-party unpackaged and self-signed applications (see [MyPhone#26](https://github.com/BestOwl/MyPhone/issues/26)).
3. **Kernel Profile Driver Requirement (`BTH_PROFILE_DRIVER_INTERFACE`)**:
   - Without access to the inbox `PhoneExperienceHost` broker, opening an HFP RFCOMM service-level connection (AT command channel) plus an eSCO bidirectional voice channel on Windows requires a WHQL-signed kernel-mode Bluetooth profile driver (`bthport.sys` / KMDF) paired with a custom audio miniport driver.

**Conclusion**: Routing two-way cellular call audio through the PC's microphone and speakers is **not possible with public Windows user-mode APIs**. NectarLink therefore supports incoming call alerts, caller ID, answer/decline/hangup, dialer, call log, and (with Elevated mode) mute/speakerphone/hold/DTMF keypad controls on the PC while call voice audio remains on the phone (or a Bluetooth headset paired directly to the phone).

---

## 3. Scope of `desktop/app/src/win/bt_hfp.rs`

- `desktop/app/src/win/bt_hfp.rs` reports no call audio, so the Calls page shows no "PC call audio" panel on Windows. Call audio stays on the phone.
- It used to report the Bluetooth link to a paired device, which read as call audio on the PC and could name a paired headset instead of the phone (#86).
- On Linux, `desktop/app/src/linux/bt_hfp.rs` routes call audio through BlueZ and PipeWire or PulseAudio, and the panel is shown there.
