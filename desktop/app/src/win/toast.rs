// SPDX-License-Identifier: GPL-3.0-or-later
//! Phone notifications as Windows toasts, with the app's icon, its actions
//! and an inline reply field.
//!
//! Nectarlink isn't a packaged app, so it registers an AppUserModelID under
//! HKCU (display name and icon) and gives the process that ID. Toasts are
//! driven from one thread with COM initialized; clicks, replies and
//! dismissals come back through per-toast handlers while the app runs.
//! Toasts left over from an earlier run are cleared at startup, since their
//! handlers are gone.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, mpsc},
};

use windows::{
    Data::Xml::Dom::XmlDocument,
    Foundation::{IPropertyValue, TypedEventHandler},
    UI::Notifications::{
        NotificationData, NotificationSetting, ToastActivatedEventArgs, ToastDismissalReason,
        ToastDismissedEventArgs, ToastNotification, ToastNotificationManager, ToastNotifier,
    },
    Win32::{
        Foundation::ERROR_FILE_NOT_FOUND,
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            Registry::{
                HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
                RegCreateKeyExW, RegDeleteTreeW, RegSetValueExW,
            },
        },
        UI::Shell::SetCurrentProcessExplicitAppUserModelID,
    },
    core::{HSTRING, IInspectable, Interface},
};

/// The app's AppUserModelID: toasts are grouped and attributed by it.
pub const AUMID: &str = "Nectarlink.Desktop";

/// What the user did with a toast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    /// Clicked the toast itself.
    Opened { device: String, key: String },
    /// Clicked an action button.
    Action { device: String, key: String, action: String },
    /// Sent text from the reply field.
    Reply { device: String, key: String, action: String, text: String },
    /// Closed it (not timed out).
    Dismissed { device: String, key: String },
}

/// One toast to show.
#[derive(Clone)]
pub struct Toast {
    /// The phone's device ID (the toast group).
    pub device: String,
    /// The notification key (hashed into the toast tag).
    pub key: String,
    pub title: String,
    pub body: String,
    /// "WhatsApp · Pixel 9".
    pub attribution: String,
    pub icon: Option<PathBuf>,
    /// A picture shown under the text (a photo in a message).
    pub image: Option<PathBuf>,
    /// `(action id, label)`.
    pub actions: Vec<(String, String)>,
    /// `(action id, placeholder)` for an inline reply.
    pub reply: Option<(String, String)>,
    pub silent: bool,
    /// A progress bar, updated in place with [`update_progress`].
    pub progress: Option<Progress>,
    /// An incoming call: stays up and rings until answered or removed, with
    /// the icon (the caller's photo) in a circle.
    pub call: bool,
}

/// A toast's progress bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    /// Under the bar, on the left ("Sending…").
    pub status: String,
    /// 0 to 1, or `None` while it's not known yet.
    pub value: Option<f64>,
    /// Under the bar, on the right ("12 MB of 80 MB").
    pub label: String,
}

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast").field("device", &self.device).finish_non_exhaustive()
    }
}

enum Command {
    Show(Box<Toast>),
    Remove { device: String, key: String },
    RemoveDevice { device: String },
    Progress { device: String, key: String, progress: Progress },
}

type Handler = Box<dyn Fn(ToastEvent) + Send + Sync>;

struct Toaster {
    commands: mpsc::Sender<Command>,
}

static TOASTER: OnceLock<Mutex<Toaster>> = OnceLock::new();

/// Whether Windows shows this app's notifications. The user can turn them
/// off for all apps or for this one (Settings > System > Notifications).
pub fn enabled() -> bool {
    ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .and_then(|notifier| notifier.Setting())
        .map_or(true, |setting| setting == NotificationSetting::Enabled)
}

/// Gives the process the app's ID. Call before any window is created.
pub fn set_process_id() {
    // SAFETY: the ID is a valid null-terminated string for the call.
    if let Err(e) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) } {
        tracing::warn!(error = %e, "can't set the app's ID");
    }
}

/// Starts the toast thread, which registers the app for toasts with its
/// icon (written to `icon` as a PNG if missing); `on_event` gets what the
/// user does with toasts.
pub fn start(icon: PathBuf, on_event: impl Fn(ToastEvent) + Send + Sync + 'static) {
    let (commands, queue) = mpsc::channel();
    let handler: Handler = Box::new(on_event);
    let spawned = std::thread::Builder::new().name("toasts".into()).spawn(move || run(&icon, queue, handler));
    match spawned {
        Ok(_) => {
            let _ = TOASTER.set(Mutex::new(Toaster { commands }));
        }
        Err(e) => tracing::warn!(error = %e, "can't start notifications"),
    }
}

fn send(command: Command) {
    if let Some(toaster) = TOASTER.get() {
        let _ = toaster.lock().unwrap_or_else(|e| e.into_inner()).commands.send(command);
    }
}

pub fn show(toast: Toast) {
    send(Command::Show(Box::new(toast)));
}

/// Moves a shown toast's progress bar, without showing the toast again.
pub fn update_progress(device: &str, key: &str, progress: Progress) {
    send(Command::Progress { device: device.to_owned(), key: key.to_owned(), progress });
}

pub fn remove(device: &str, key: &str) {
    send(Command::Remove { device: device.to_owned(), key: key.to_owned() });
}

/// Removes every toast of a device (it stopped sharing, or was unpaired).
pub fn remove_device(device: &str) {
    send(Command::RemoveDevice { device: device.to_owned() });
}

fn registration_key() -> HSTRING {
    HSTRING::from(format!(r"Software\Classes\AppUserModelId\{AUMID}"))
}

/// Removes the registration (when uninstalling), and the app's toasts.
pub fn unregister() {
    if let Ok(history) = ToastNotificationManager::History() {
        let _ = history.ClearWithId(&HSTRING::from(AUMID));
    }
    // SAFETY: deletes a key under HKCU by a valid path.
    let result = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &registration_key()) };
    if result.is_err() && result != ERROR_FILE_NOT_FOUND {
        tracing::warn!(error = ?result, "can't remove the notification registration");
    }
}

/// `HKCU\Software\Classes\AppUserModelId\<AUMID>`: how Windows names the
/// sender of toasts from an unpackaged app.
fn register(icon: &Path) -> windows::core::Result<()> {
    let key_path = registration_key();
    let mut key = HKEY::default();
    // SAFETY: creating/opening a key under HKCU with a valid path; the handle
    // is closed below.
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &key_path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
        .ok()?;
    }
    let set = |name: &str, value: &str| {
        let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: the data is a null-terminated UTF-16 string of the given size.
        unsafe {
            RegSetValueExW(
                key,
                &HSTRING::from(name),
                None,
                REG_SZ,
                Some(std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2)),
            )
        }
        .ok()
    };
    let result = set("DisplayName", "Nectarlink").and_then(|()| set("IconUri", &icon.to_string_lossy()));
    // SAFETY: the key was opened above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    result
}

fn run(icon: &Path, queue: mpsc::Receiver<Command>, handler: Handler) {
    // SAFETY: initializes COM for this thread, which lives as long as the app.
    if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
        tracing::warn!(error = %e, "notifications are unavailable");
        return;
    }
    // WIC needs COM, which is why this happens here and not on the UI thread.
    if !icon.exists()
        && let Err(e) = super::icon::write_png(icon, 256)
    {
        tracing::warn!(error = %e, "can't write the app icon");
    }
    if let Err(e) = register(icon) {
        tracing::warn!(error = %e, "can't register for notifications");
    }
    let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID)) {
        Ok(notifier) => notifier,
        Err(e) => {
            tracing::warn!(error = %e, "notifications are unavailable");
            return;
        }
    };
    let history = ToastNotificationManager::History().ok();
    let aumid = HSTRING::from(AUMID);
    if let Some(history) = &history {
        let _ = history.ClearWithId(&aumid);
    }
    let handler: &'static Handler = Box::leak(Box::new(handler));
    while let Ok(command) = queue.recv() {
        let result = match command {
            Command::Show(toast) => show_now(&notifier, *toast, handler),
            Command::Remove { device, key } => match &history {
                Some(h) => h.RemoveGroupedTagWithId(
                    &HSTRING::from(tag(&key)),
                    &HSTRING::from(group(&device)),
                    &aumid,
                ),
                None => Ok(()),
            },
            Command::RemoveDevice { device } => match &history {
                Some(h) => h.RemoveGroupWithId(&HSTRING::from(group(&device)), &aumid),
                None => Ok(()),
            },
            Command::Progress { device, key, progress } => progress_data(&progress).and_then(|data| {
                notifier
                    .UpdateWithTagAndGroup(&data, &HSTRING::from(tag(&key)), &HSTRING::from(group(&device)))
                    .map(drop)
            }),
        };
        if let Err(e) = result {
            tracing::debug!(error = %e, "toast command failed");
        }
    }
}

fn show_now(notifier: &ToastNotifier, toast: Toast, handler: &'static Handler) -> windows::core::Result<()> {
    let xml = XmlDocument::new()?;
    xml.LoadXml(&HSTRING::from(toast_xml(&toast)))?;
    let notification = ToastNotification::CreateToastNotification(&xml)?;
    notification.SetTag(&HSTRING::from(tag(&toast.key)))?;
    notification.SetGroup(&HSTRING::from(group(&toast.device)))?;
    // Silent on the phone: straight to the notification center, no pop-up.
    notification.SetSuppressPopup(toast.silent)?;
    if let Some(progress) = &toast.progress {
        notification.SetData(&progress_data(progress)?)?;
    }

    let (device, key) = (toast.device.clone(), toast.key.clone());
    notification.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
        let Some(args) = args.as_ref().and_then(|a| a.cast::<ToastActivatedEventArgs>().ok()) else {
            return Ok(());
        };
        let arguments = args.Arguments()?.to_string();
        let event = match arguments.strip_prefix("a:") {
            None => ToastEvent::Opened { device: device.clone(), key: key.clone() },
            Some(action) => match reply_text(&args) {
                Some(text) => ToastEvent::Reply {
                    device: device.clone(),
                    key: key.clone(),
                    action: action.into(),
                    text,
                },
                None => {
                    ToastEvent::Action { device: device.clone(), key: key.clone(), action: action.into() }
                }
            },
        };
        handler(event);
        Ok(())
    }))?;
    let (device, key) = (toast.device.clone(), toast.key.clone());
    notification.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(
        move |_, args| {
            if let Some(args) = args.as_ref()
                && args.Reason()? == ToastDismissalReason::UserCanceled
            {
                handler(ToastEvent::Dismissed { device: device.clone(), key: key.clone() });
            }
            Ok(())
        },
    ))?;
    notifier.Show(&notification)
}

/// The values bound into a toast's progress bar.
fn progress_data(progress: &Progress) -> windows::core::Result<NotificationData> {
    let data = NotificationData::new()?;
    let values = data.Values()?;
    let value =
        progress.value.map_or_else(|| "indeterminate".to_owned(), |v| format!("{:.3}", v.clamp(0.0, 1.0)));
    values.Insert(&HSTRING::from(PROGRESS_VALUE), &HSTRING::from(value))?;
    values.Insert(&HSTRING::from(PROGRESS_STATUS), &HSTRING::from(&progress.status))?;
    values.Insert(&HSTRING::from(PROGRESS_LABEL), &HSTRING::from(&progress.label))?;
    Ok(data)
}

const PROGRESS_VALUE: &str = "progressValue";
const PROGRESS_STATUS: &str = "progressStatus";
const PROGRESS_LABEL: &str = "progressLabel";

/// The text typed in the reply field, if any.
fn reply_text(args: &ToastActivatedEventArgs) -> Option<String> {
    let value = args.UserInput().ok()?.Lookup(&HSTRING::from(REPLY_INPUT)).ok()?;
    let text = value.cast::<IPropertyValue>().ok()?.GetString().ok()?.to_string();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

const REPLY_INPUT: &str = "reply";

/// Toast tags are short; a stable hash of the key (FNV-1a) fits and stays
/// the same across runs.
fn tag(key: &str) -> String {
    let hash =
        key.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3));
    format!("{hash:016x}")
}

fn group(device: &str) -> String {
    device.chars().take(16).collect()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Characters XML 1.0 can't hold.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// On a call toast, the actions shown as Answer (green) and Decline (red).
pub const CALL_ANSWER: &str = "answer";
pub const CALL_DECLINE: &str = "decline";

/// The toast's XML (Windows toast schema).
fn toast_xml(toast: &Toast) -> String {
    let mut xml = String::from(if toast.call {
        r#"<toast launch="open" scenario="incomingCall" useButtonStyle="true"><visual><binding template="ToastGeneric">"#
    } else {
        r#"<toast launch="open"><visual><binding template="ToastGeneric">"#
    });
    xml.push_str(&format!(r#"<text hint-maxLines="1">{}</text>"#, escape(&toast.title)));
    if !toast.body.is_empty() {
        xml.push_str(&format!("<text>{}</text>", escape(&toast.body)));
    }
    xml.push_str(&format!(r#"<text placement="attribution">{}</text>"#, escape(&toast.attribution)));
    if let Some(image) = &toast.image {
        xml.push_str(&format!(r#"<image src="{}"/>"#, escape(&crate::icons::file_url(image))));
    }
    if toast.progress.is_some() {
        xml.push_str(&format!(
            r#"<progress value="{{{PROGRESS_VALUE}}}" status="{{{PROGRESS_STATUS}}}" valueStringOverride="{{{PROGRESS_LABEL}}}"/>"#
        ));
    }
    if let Some(icon) = &toast.icon {
        let crop = if toast.call { r#" hint-crop="circle""# } else { "" };
        xml.push_str(&format!(
            r#"<image placement="appLogoOverride"{crop} src="{}"/>"#,
            escape(&crate::icons::file_url(icon))
        ));
    }
    xml.push_str("</binding></visual>");
    if toast.reply.is_some() || !toast.actions.is_empty() {
        xml.push_str("<actions>");
        if let Some((action, placeholder)) = &toast.reply {
            xml.push_str(&format!(
                r#"<input id="{REPLY_INPUT}" type="text" placeHolderContent="{}"/>"#,
                escape(placeholder)
            ));
            xml.push_str(&format!(
                r#"<action content="Send" arguments="a:{}" hint-inputId="{REPLY_INPUT}"/>"#,
                escape(action)
            ));
        }
        // Windows shows at most five buttons, the send button included.
        let room = 5 - usize::from(toast.reply.is_some());
        for (id, label) in toast.actions.iter().take(room) {
            // On a call: Answer green, Decline red.
            let style = match (toast.call, id.as_str()) {
                (true, CALL_ANSWER) => r#" hint-buttonStyle="Success""#,
                (true, CALL_DECLINE) => r#" hint-buttonStyle="Critical""#,
                _ => "",
            };
            xml.push_str(&format!(
                r#"<action content="{}" arguments="a:{}"{style}/>"#,
                escape(label),
                escape(id)
            ));
        }
        xml.push_str("</actions>");
    }
    if toast.silent {
        xml.push_str(r#"<audio silent="true"/>"#);
    } else if toast.call {
        xml.push_str(r#"<audio src="ms-winsoundevent:Notification.Looping.Call" loop="true"/>"#);
    }
    xml.push_str("</toast>");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toast() -> Toast {
        Toast {
            device: "abcdefgh12345678xyz".into(),
            key: "0|com.chat|1|null|10123".into(),
            title: "Sam & <Alex>".into(),
            body: "Lunch \"today\"?\u{7}".into(),
            attribution: "Chat · Pixel".into(),
            icon: Some(PathBuf::from(r"C:\data\cache\icons\com.chat.png")),
            image: Some(PathBuf::from(r"C:\data\cache\images\1.jpg")),
            actions: (0..6).map(|i| (i.to_string(), format!("Action {i}"))).collect(),
            reply: Some(("r".into(), "Reply".into())),
            silent: true,
            progress: None,
            call: false,
        }
    }

    #[test]
    fn builds_valid_toast_xml() {
        let xml = toast_xml(&toast());
        assert!(xml.contains("Sam &amp; &lt;Alex&gt;"), "{xml}");
        assert!(xml.contains("Lunch &quot;today&quot;?</text>"), "control characters are dropped: {xml}");
        assert!(xml.contains(r#"src="file:///C:/data/cache/icons/com.chat.png""#));
        assert!(xml.contains(r#"<image src="file:///C:/data/cache/images/1.jpg"/>"#), "{xml}");
        assert!(xml.contains(r#"arguments="a:r" hint-inputId="reply""#));
        assert_eq!(xml.matches("<action ").count(), 5, "five buttons at most");
        assert!(xml.ends_with(r#"<audio silent="true"/></toast>"#));
        // Windows parses it.
        XmlDocument::new().unwrap().LoadXml(&HSTRING::from(xml)).expect("well-formed XML");
    }

    #[test]
    fn calls_ring_until_handled() {
        let actions = vec![(CALL_ANSWER.into(), "Answer".into()), (CALL_DECLINE.into(), "Decline".into())];
        let xml = toast_xml(&Toast { call: true, silent: false, reply: None, actions, ..toast() });
        assert!(
            xml.starts_with(r#"<toast launch="open" scenario="incomingCall" useButtonStyle="true">"#),
            "{xml}"
        );
        assert!(xml.contains(r#"arguments="a:answer" hint-buttonStyle="Success""#), "{xml}");
        assert!(xml.contains(r#"arguments="a:decline" hint-buttonStyle="Critical""#), "{xml}");
        assert!(xml.contains(r#"placement="appLogoOverride" hint-crop="circle""#), "{xml}");
        assert!(xml.contains(r#"Notification.Looping.Call" loop="true""#), "{xml}");
        XmlDocument::new().unwrap().LoadXml(&HSTRING::from(xml)).expect("well-formed XML");
    }

    #[test]
    fn progress_bars_are_bound_to_data() {
        let toast = Toast {
            progress: Some(Progress { status: "Sending…".into(), value: None, label: String::new() }),
            ..toast()
        };
        let xml = toast_xml(&toast);
        assert!(
            xml.contains(r#"<progress value="{progressValue}" status="{progressStatus}" valueStringOverride="{progressLabel}"/>"#),
            "{xml}"
        );
        XmlDocument::new().unwrap().LoadXml(&HSTRING::from(xml)).expect("well-formed XML");
        let data =
            progress_data(&Progress { status: "s".into(), value: Some(1.5), label: "l".into() }).unwrap();
        assert_eq!(data.Values().unwrap().Lookup(&HSTRING::from(PROGRESS_VALUE)).unwrap(), "1.000");
    }

    #[test]
    fn tags_are_short_and_stable() {
        assert_eq!(tag("a"), tag("a"));
        assert_ne!(tag("a"), tag("b"));
        assert_eq!(tag("0|com.chat|1|null|10123").len(), 16);
        assert_eq!(group("abcdefgh12345678xyz"), "abcdefgh12345678");
    }
}
