// SPDX-License-Identifier: GPL-3.0-or-later
//! What plays on a phone, in Windows' own media controls: the flyout by the
//! volume and Quick Settings, the lock screen, and the keyboard's media
//! keys (System Media Transport Controls).
//!
//! A desktop app gets these controls for one of its windows, so a hidden
//! window on its own thread owns them. Updates reach that thread through a
//! queue and a window message; button presses come back on Windows' threads.

use std::{
    path::PathBuf,
    sync::{
        Mutex, MutexGuard, OnceLock,
        atomic::{AtomicIsize, Ordering},
        mpsc,
    },
    time::SystemTime,
};

use nectarlink_core::{DeviceId, MediaAction};
use windows::{
    Foundation::{TimeSpan, TypedEventHandler},
    Media::{
        MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs,
        SystemMediaTransportControls, SystemMediaTransportControlsButton,
        SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
    },
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference},
    Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            Com::{COINIT_MULTITHREADED, CoInitializeEx},
            LibraryLoader::GetModuleHandleW,
            WinRT::ISystemMediaTransportControlsInterop,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, PostMessageW,
            RegisterClassExW, WINDOW_EX_STYLE, WM_APP, WNDCLASSEXW, WS_OVERLAPPED,
        },
    },
    core::{HSTRING, w},
};

use crate::state::PlayerView;

/// What the controls show: one player on one device.
#[derive(Debug, Clone)]
pub struct Shown {
    pub device: DeviceId,
    pub device_name: String,
    pub view: PlayerView,
}

type OnCommand = Box<dyn Fn(DeviceId, String, MediaAction, Option<u64>) + Send + Sync>;

static WINDOW: AtomicIsize = AtomicIsize::new(0);
static QUEUE: OnceLock<Mutex<mpsc::Sender<Option<Shown>>>> = OnceLock::new();
static ON_COMMAND: OnceLock<OnCommand> = OnceLock::new();
/// The player the controls stand for, for button presses.
static CURRENT: Mutex<Option<(DeviceId, String)>> = Mutex::new(None);

const WM_UPDATE: u32 = WM_APP + 1;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Starts the controls (hidden until something plays); `on_command` gets
/// what the user pressed, on a Windows thread.
pub fn start(on_command: impl Fn(DeviceId, String, MediaAction, Option<u64>) + Send + Sync + 'static) {
    if ON_COMMAND.set(Box::new(on_command)).is_err() {
        return;
    }
    let (sender, updates) = mpsc::channel();
    let _ = QUEUE.set(Mutex::new(sender));
    let spawned = std::thread::Builder::new().name("media-controls".into()).spawn(move || {
        if let Err(e) = run(updates) {
            tracing::warn!(error = %e, "Windows media controls are unavailable");
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "Windows media controls are unavailable");
    }
}

/// Shows a player in the controls, or hides them.
pub fn show(current: Option<Shown>) {
    let Some(queue) = QUEUE.get() else { return };
    if lock(queue).send(current).is_err() {
        return;
    }
    let window = WINDOW.load(Ordering::Acquire);
    if window != 0 {
        // SAFETY: posting to our own window; it drains the queue.
        unsafe {
            let _ = PostMessageW(Some(HWND(window as *mut _)), WM_UPDATE, WPARAM(0), LPARAM(0));
        }
    }
}

struct Controls {
    smtc: SystemMediaTransportControls,
    /// The artwork shown, so it's only loaded when it changes.
    art: Option<PathBuf>,
}

static CONTROLS: OnceLock<Mutex<Option<Controls>>> = OnceLock::new();
static UPDATES: OnceLock<Mutex<mpsc::Receiver<Option<Shown>>>> = OnceLock::new();

fn run(updates: mpsc::Receiver<Option<Shown>>) -> windows::core::Result<()> {
    // SAFETY: COM for this thread, which lives as long as the app.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;
    // SAFETY: class registration and a window that's never shown; the
    // window procedure only touches statics.
    let hwnd = unsafe {
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("Nectarlink.Media"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("Nectarlink.Media"),
            w!("Nectarlink media"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance.into()),
            None,
        )?
    };
    let interop =
        windows::core::factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    // SAFETY: the window is ours and alive for the app's lifetime.
    let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
    smtc.SetIsEnabled(false)?;
    smtc.ButtonPressed(&TypedEventHandler::<
        SystemMediaTransportControls,
        SystemMediaTransportControlsButtonPressedEventArgs,
    >::new(|_, args| {
        let Some(args) = args.as_ref() else { return Ok(()) };
        let action = match args.Button()? {
            SystemMediaTransportControlsButton::Play => MediaAction::Play,
            SystemMediaTransportControlsButton::Pause => MediaAction::Pause,
            SystemMediaTransportControlsButton::Next => MediaAction::Next,
            SystemMediaTransportControlsButton::Previous => MediaAction::Previous,
            _ => return Ok(()),
        };
        command(action, None);
        Ok(())
    }))?;
    smtc.PlaybackPositionChangeRequested(&TypedEventHandler::<
        SystemMediaTransportControls,
        PlaybackPositionChangeRequestedEventArgs,
    >::new(|_, args| {
        if let Some(args) = args.as_ref() {
            let ticks = args.RequestedPlaybackPosition()?.Duration.max(0);
            command(MediaAction::Seek, Some(u64::try_from(ticks / 10_000).unwrap_or(0)));
        }
        Ok(())
    }))?;
    let _ = CONTROLS.set(Mutex::new(Some(Controls { smtc, art: None })));
    let _ = UPDATES.set(Mutex::new(updates));
    WINDOW.store(hwnd.0 as isize, Ordering::Release);
    // Anything queued before the window existed.
    apply_updates();

    let mut msg = MSG::default();
    // SAFETY: a plain message loop for this thread's window.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}

fn command(action: MediaAction, position: Option<u64>) {
    let Some((device, player)) = lock(&CURRENT).clone() else { return };
    if let Some(on_command) = ON_COMMAND.get() {
        on_command(device, player, action, position);
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_UPDATE {
        apply_updates();
        return LRESULT(0);
    }
    // SAFETY: default handling for everything else.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Applies the newest queued update (older ones are already out of date).
fn apply_updates() {
    let Some(updates) = UPDATES.get() else { return };
    let Some(latest) = lock(updates).try_iter().last() else { return };
    let Some(controls) = CONTROLS.get() else { return };
    let mut controls = lock(controls);
    let Some(controls) = controls.as_mut() else { return };
    if let Err(e) = apply(controls, latest) {
        tracing::debug!(error = %e, "can't update the media controls");
    }
}

fn apply(controls: &mut Controls, shown: Option<Shown>) -> windows::core::Result<()> {
    let smtc = &controls.smtc;
    let Some(shown) = shown else {
        *lock(&CURRENT) = None;
        smtc.SetIsEnabled(false)?;
        smtc.DisplayUpdater()?.ClearAll()?;
        controls.art = None;
        return Ok(());
    };
    let player = &shown.view.player;
    *lock(&CURRENT) = Some((shown.device, player.id.clone()));
    let can = |action: &str| player.actions.iter().any(|a| a == action);

    smtc.SetIsEnabled(true)?;
    smtc.SetIsPlayEnabled(can("play"))?;
    smtc.SetIsPauseEnabled(can("pause"))?;
    smtc.SetIsNextEnabled(can("next"))?;
    smtc.SetIsPreviousEnabled(can("previous"))?;
    smtc.SetPlaybackStatus(if player.playing {
        MediaPlaybackStatus::Playing
    } else {
        MediaPlaybackStatus::Paused
    })?;

    let updater = smtc.DisplayUpdater()?;
    updater.SetType(MediaPlaybackType::Music)?;
    let music = updater.MusicProperties()?;
    music.SetTitle(&HSTRING::from(player.title.as_deref().unwrap_or(&player.app)))?;
    // The flyout shows two lines; the second says where it plays.
    let source = format!("{} on {}", player.app, shown.device_name);
    let artist = match &player.artist {
        Some(artist) => format!("{artist} · {source}"),
        None => source,
    };
    music.SetArtist(&HSTRING::from(artist))?;
    music.SetAlbumTitle(&HSTRING::from(player.album.as_deref().unwrap_or_default()))?;
    if controls.art != shown.view.art {
        match shown.view.art.as_deref().map(std::fs::read) {
            Some(Ok(bytes)) => updater.SetThumbnail(&thumbnail(&bytes)?)?,
            _ => updater.SetThumbnail(None)?,
        }
        controls.art = shown.view.art.clone();
    }
    updater.Update()?;

    if let Some(duration) = player.duration {
        let elapsed = if player.playing {
            SystemTime::now().duration_since(shown.view.at).map_or(0, |d| d.as_millis() as u64)
        } else {
            0
        };
        let position = player.position.unwrap_or(0).saturating_add(elapsed).min(duration);
        let span = |ms: u64| TimeSpan { Duration: i64::try_from(ms).unwrap_or(i64::MAX / 10_000) * 10_000 };
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        timeline.SetStartTime(span(0))?;
        timeline.SetEndTime(span(duration))?;
        timeline.SetMinSeekTime(span(0))?;
        timeline.SetMaxSeekTime(span(if can("seek") { duration } else { 0 }))?;
        timeline.SetPosition(span(position))?;
        smtc.UpdateTimelineProperties(&timeline)?;
    }
    Ok(())
}

/// Artwork as a stream reference, which the controls take for thumbnails.
fn thumbnail(bytes: &[u8]) -> windows::core::Result<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let writer = DataWriter::CreateDataWriter(&stream)?;
    writer.WriteBytes(bytes)?;
    writer.StoreAsync()?.join()?;
    writer.FlushAsync()?.join()?;
    writer.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}
