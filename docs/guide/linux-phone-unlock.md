# Unlock a Linux session with your Android fingerprint

Phone unlock is optional. It requires a paired phone running Android 11 or newer with a strong biometric enrolled, both apps running, and a direct local NectarLink connection. Use the regular NectarLink Android app with Linux phone unlock support. Your screen locker must be able to start a separate Linux-PAM authentication transaction as the locked user. The PAM module rejects lockers that call it as root.

## Install the PAM service

Build the verifier from the same checkout as the desktop app. Install the module and service template from that checkout together.

```sh
cargo build --locked --release -p nectarlink-pam-unlock
```

Configure the dedicated PAM service before installing it. Save the following as `nectarlink-phone-lock` for a Fedora setup where the password service uses `pam_faillock` and the `system-auth` account stack. Copy any pre-auth policy options from the password service, such as a custom tally directory. The pre-auth check must run before the phone request, including with Quickshell, which only calls the `auth` stack.

```text
#%PAM-1.0
auth requisite pam_faillock.so preauth silent
auth required libpam_nectarlink_phone_unlock.so
account requisite libpam_nectarlink_phone_unlock.so
account include system-auth
```

For other PAM configurations, start with the [service template](../../desktop/pam-unlock/nectarlink-phone-lock). Keep NectarLink's `auth` and `account` entries and match the password service's pre-auth lockout policy. The template uses `account required pam_unix.so` to check local account expiry for lockers that call `pam_acct_mgmt`. Replace that line with your distribution's account include when needed to match the password service's account policy.

Install the two artifacts and your configured service as root. Fedora x86-64 uses `/usr/lib64/security`. Debian on x86-64 uses `/usr/lib/x86_64-linux-gnu/security`. On immutable distributions with a read-only module directory, use the distribution's supported image or package layering mechanism to install the PAM module.

```sh
sudo install -m 0755 target/release/libpam_nectarlink_phone_unlock.so /usr/lib64/security/
sudo install -m 0755 target/release/nectarlink-phone-enrollment /usr/local/bin/
sudo install -d -m 0755 /etc/nectarlink /etc/nectarlink/phone-unlock
sudo install -m 0644 nectarlink-phone-lock /etc/pam.d/nectarlink-phone-lock
```

On SELinux systems, restore the distribution's default file contexts for the installed module and configuration.

The desktop listens in the session's `XDG_RUNTIME_DIR`. The PAM service defaults to `/run/user/<uid>`. If your session uses a different directory, add `runtime_dir=/absolute/path` to the module's `auth` line so both use the same directory. The directory must be owned by the account being unlocked and have mode `0700`. PAM requires each parent directory to be owned by root and writable only by root. Symbolic links are rejected throughout the path.

## Enroll a phone

1. Restart the desktop app after installing the PAM service. Pair the phone. Enable **Unlock this PC with the phone** in the desktop's device settings and **Unlock this PC with your fingerprint** on Android. Allow NectarLink notifications on Android so you can open an unlock prompt when the app is in the background.
2. Run `nectarlink-desktop --linux-unlock-devices` as the user whose session you want to unlock. Find the phone's device ID.
3. Run `nectarlink-desktop --linux-unlock-enroll PHONE_DEVICE_ID > enrollment.json`. Verify the displayed PC and account on the phone, then approve with your biometric.
4. Run `sudo nectarlink-phone-enrollment enroll < enrollment.json`. Check that the grouped key fingerprints match on the PC and Android. Enter the **fingerprint from the phone** at the terminal prompt. Spaces between groups are optional. Delete the candidate file after installation if you no longer need it.

The system stores one enrollment per Linux UID. To replace it, repeat enrollment with the new phone and confirm its key fingerprint. After renaming the PC or Linux account, repeat steps 3 and 4 to confirm the updated names and compare fingerprints again. The phone keeps its existing key if the key is still usable. To remove an enrollment, run `sudo nectarlink-phone-enrollment revoke "$(id -u)"` and turn off the unlock permission on both devices.

## Connect your screen locker

Start a dedicated PAM transaction for `nectarlink-phone-lock` when the user explicitly requests phone unlock. Keep the password action available during the request. Cancel the PAM transaction when the password path starts or the lock session ends. Apply a result only to the active request of the current lock session. Require `PAM_SUCCESS` from `pam_authenticate` and, if the locker calls it, `pam_acct_mgmt` before unlocking.

For Quickshell, copy [PhoneUnlock.qml](examples/PhoneUnlock.qml) into your locker configuration. Bind its `locked` property to the secure lock state, call `start()` from an accessible control and `cancel()` when the user cancels or starts typing a password, and connect `authenticated` to the locker's existing unlock handler.

### KDE Plasma

KDE's lock screen has no setting for another unlock method. It does start a second PAM transaction, on the `kde-fingerprint` service, as the locked user every time the screen locks, beside the password. NectarLink can answer in that transaction. The phone then asks for your fingerprint every time the PC locks while NectarLink runs, and the PC unlocks when you approve. You don't need to press anything on the PC.

Keep the `nectarlink-phone-lock` service installed as above, because NectarLink offers phone unlock only when that service exists. Then add NectarLink to `/etc/pam.d/kde-fingerprint` as root, after any pre-auth lockout line and before the fingerprint reader's line:

```text
auth sufficient libpam_nectarlink_phone_unlock.so allow_service=kde-fingerprint
```

`allow_service` lets the module run in that one other service. The module still accepts only a local, non-root account in a transaction started by that same account. If no phone is enrolled or NectarLink isn't running, the module reports `PAM_AUTHINFO_UNAVAIL` at once and the stack goes on to the fingerprint reader. While the phone's request is open, which can last up to 30 seconds, a fingerprint reader listed after it waits. Type your password at any time.

If your distribution has no `kde-fingerprint` service, create it with the NectarLink line followed by `auth required pam_deny.so`. To undo it, remove the NectarLink line, or the file if you created it.

To remove the integration, remove the locker action or the `kde-fingerprint` line, revoke the enrollment, and remove the dedicated PAM service, module and enrollment command installed above. Restart NectarLink so it refreshes its advertised capabilities.
