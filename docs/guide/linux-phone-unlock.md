# Unlock a Linux session with your Android fingerprint

Phone unlock is optional. It requires a paired phone running Android 11 or newer with a strong biometric enrolled, both apps running, and a direct local NectarLink connection. Your screen locker must be able to start a separate Linux-PAM authentication transaction from the locked user session. The screen locker keeps its own password action.

## Install the PAM service

Build the verifier from the same checkout as the desktop app.

```sh
cargo build --locked --release -p nectarlink-pam-unlock
```

Install the two artifacts and the service as root. If your password service uses `pam_faillock`, enable the pre-auth check in the phone service template first and use the same policy options as the password service. Replace the module directory if your distribution uses a different location. Fedora x86-64 uses `/usr/lib64/security`. Debian on x86-64 uses `/usr/lib/x86_64-linux-gnu/security`.

```sh
sudo install -m 0755 target/release/libpam_nectarlink_phone_unlock.so /usr/lib64/security/
sudo install -m 0755 target/release/nectarlink-phone-enrollment /usr/local/bin/
sudo install -d -m 0755 /etc/nectarlink /etc/nectarlink/phone-unlock
sudo install -m 0644 desktop/pam-unlock/nectarlink-phone-lock /etc/pam.d/
```

On SELinux systems, restore the distribution's default file contexts for the installed module and configuration. The dedicated service is for a local locker running as the account it unlocks.

The desktop listens in the session's `XDG_RUNTIME_DIR`. The PAM service defaults to `/run/user/<uid>`. If your session uses a different directory, add `runtime_dir=/absolute/path` to the module line in the dedicated PAM service so both use the same directory. The directory must be owned by the account being unlocked and have mode `0700`. PAM requires each parent directory to be owned by root and writable only by root. Symbolic links are rejected throughout the path.

## Enroll a phone

1. Restart the desktop app after installing the PAM service. Pair the phone. Enable **Unlock this PC with the phone** in the desktop's device settings and **Unlock this PC with your fingerprint** on Android. Allow NectarLink notifications on Android so you can open an unlock prompt when the app is in the background.
2. Run `nectarlink-desktop --linux-unlock-devices` as the user whose session you want to unlock. Find the phone's device ID.
3. Run `nectarlink-desktop --linux-unlock-enroll PHONE_DEVICE_ID > enrollment.json`. Verify the displayed PC and account on the phone, then approve with your biometric.
4. Run `sudo nectarlink-phone-enrollment enroll < enrollment.json`. Compare the key fingerprint on the PC with the fingerprint shown by the Android app, then enter the phone's fingerprint string at the terminal prompt. Delete the candidate file after installation if you no longer need it.

The system stores one enrollment per Linux UID. To replace it, repeat enrollment with the new phone and confirm its key fingerprint. To remove it, run `sudo nectarlink-phone-enrollment revoke "$(id -u)"` and turn off the unlock permission on both devices. After renaming the PC or Linux account, remove the old enrollment before setting it up again.

## Connect your screen locker

Start a dedicated PAM transaction for `nectarlink-phone-lock` when the user explicitly requests phone unlock. Keep the password action available during the request. Cancel the PAM transaction when the password path starts or the lock session ends. Apply a result only to the active request of the current lock session, and unlock only on `PAM_SUCCESS`. After cancellation, the phone may still show the old prompt. Dismiss it or let it expire, up to 30 seconds, before retrying phone unlock.

For Quickshell, copy [PhoneUnlock.qml](examples/PhoneUnlock.qml) into your locker configuration. Bind its `locked` property to the secure lock state, call `start()` from an accessible control and `cancel()` when the user cancels or starts typing a password, and connect `authenticated` to the locker's existing unlock handler.

To remove the integration, remove the locker action, revoke the enrollment, and remove the dedicated PAM service, module and enrollment command installed above. Restart NectarLink so it refreshes its advertised capabilities.
