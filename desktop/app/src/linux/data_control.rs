// SPDX-License-Identifier: GPL-3.0-or-later
//! The clipboard through Wayland's `ext-data-control` protocol, which lets an
//! app use it without focus: a normal Wayland client only sees the clipboard
//! while one of its windows has focus. Compositors that offer it include KDE
//! Plasma and those built on wlroots; GNOME doesn't.

use std::io::Read;
use std::os::fd::AsFd;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_device_v1::{
    self, EVT_DATA_OFFER_OPCODE, ExtDataControlDeviceV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_manager_v1::ExtDataControlManagerV1;
use wayland_protocols::ext::data_control::v1::client::ext_data_control_offer_v1::{
    self, ExtDataControlOfferV1,
};
use wayland_protocols::ext::data_control::v1::client::ext_data_control_source_v1::{
    self, ExtDataControlSourceV1,
};

/// How long another app gets to hand over the clipboard.
const READ_TIMEOUT: Duration = Duration::from_secs(3);
/// More than this isn't something to carry between devices.
const MAX_READ: usize = 64 << 20;

type Contents = Arc<Vec<(String, Vec<u8>)>>;

struct Offer {
    proxy: ExtDataControlOfferV1,
    mimes: Vec<String>,
}

#[derive(Default)]
struct Selection {
    /// Offers announced, until the selection event says which one counts.
    pending: Vec<Offer>,
    current: Option<Offer>,
    /// What we offer while ours is the clipboard.
    source: Option<(ExtDataControlSourceV1, Contents)>,
}

struct Shared {
    conn: Connection,
    queue: QueueHandle<Handler>,
    manager: ExtDataControlManagerV1,
    device: ExtDataControlDeviceV1,
    selection: Mutex<Selection>,
}

impl Shared {
    fn selection(&self) -> MutexGuard<'_, Selection> {
        self.selection.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct Handler {
    shared: Arc<Mutex<Option<Arc<Shared>>>>,
    on_change: fn(),
    /// Past the first selection, which is what was already on the clipboard.
    ready: bool,
}

impl Handler {
    fn with_selection<T>(&self, f: impl FnOnce(&mut Selection) -> T) -> Option<T> {
        let shared = self.shared.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
        let mut selection = shared.selection();
        Some(f(&mut selection))
    }
}

static ACTIVE: OnceLock<Arc<Shared>> = OnceLock::new();

/// Connects, if this compositor offers the protocol. `on_change` runs on the
/// connection's own thread whenever the clipboard changes.
pub fn start(on_change: fn()) -> bool {
    let Ok(conn) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, mut queue)): Result<(GlobalList, _), _> = registry_queue_init::<Handler>(&conn) else {
        return false;
    };
    let qh = queue.handle();
    let (Ok(manager), Ok(seat)) = (
        globals.bind::<ExtDataControlManagerV1, _, _>(&qh, 1..=1, ()),
        globals.bind::<WlSeat, _, _>(&qh, 1..=1, ()),
    ) else {
        return false;
    };
    let device = manager.get_data_device(&seat, &qh, ());
    let shared =
        Arc::new(Shared { conn: conn.clone(), queue: qh, manager, device, selection: Mutex::default() });
    let slot = Arc::new(Mutex::new(Some(Arc::clone(&shared))));
    let mut handler = Handler { shared: slot, on_change, ready: false };
    // The first selection is what's already there: know it before anyone asks.
    if queue.roundtrip(&mut handler).is_err() || ACTIVE.set(shared).is_err() {
        return false;
    }
    handler.ready = true;
    let spawned = std::thread::Builder::new().name("clipboard-data-control".into()).spawn(move || {
        loop {
            if let Err(e) = queue.blocking_dispatch(&mut handler) {
                tracing::warn!(error = %e, "lost the Wayland clipboard connection");
                return;
            }
        }
    });
    spawned.is_ok()
}

pub fn active() -> bool {
    ACTIVE.get().is_some()
}

/// The current clipboard's types, one per line; empty when it holds nothing.
pub fn targets() -> Option<String> {
    let shared = ACTIVE.get()?;
    let selection = shared.selection();
    Some(selection.current.as_ref().map(|o| o.mimes.join("\n")).unwrap_or_default())
}

/// The clipboard's contents as `mime`.
pub fn read(mime: &str) -> Option<Vec<u8>> {
    let shared = ACTIVE.get()?;
    let (reader, writer) = std::io::pipe().ok()?;
    {
        let selection = shared.selection();
        let offer = selection.current.as_ref()?;
        if !offer.mimes.iter().any(|m| m == mime) {
            return None;
        }
        offer.proxy.receive(mime.to_owned(), writer.as_fd());
    }
    shared.conn.flush().ok()?;
    // Ours is the only copy we can close: the reader sees the end of the data
    // when the app sending it closes its own.
    drop(writer);
    read_with_timeout(reader)
}

fn read_with_timeout(mut reader: std::io::PipeReader) -> Option<Vec<u8>> {
    use std::os::fd::AsRawFd;

    let deadline = Instant::now() + READ_TIMEOUT;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let mut pfd = libc::pollfd { fd: reader.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        // SAFETY: `pfd` is one valid `pollfd`, and `reader` keeps its descriptor open.
        let ready = unsafe { libc::poll(&mut pfd, 1, left.as_millis() as libc::c_int) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return None;
        }
        if ready == 0 {
            return None;
        }
        match reader.read(&mut buf) {
            Ok(0) => return Some(out),
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() > MAX_READ {
                    return None;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// Makes `contents`, as (type, data) pairs, the clipboard.
pub fn write(contents: Vec<(String, Vec<u8>)>) -> bool {
    let Some(shared) = ACTIVE.get() else {
        return false;
    };
    let source = shared.manager.create_data_source(&shared.queue, ());
    for (mime, _) in &contents {
        source.offer(mime.clone());
    }
    {
        let mut selection = shared.selection();
        if let Some((old, _)) = selection.source.replace((source.clone(), Arc::new(contents))) {
            old.destroy();
        }
    }
    shared.device.set_selection(Some(&source));
    shared.conn.flush().is_ok()
}

impl Dispatch<WlRegistry, GlobalListContents> for Handler {
    fn event(
        _: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as Proxy>::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for Handler {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlManagerV1, ()> for Handler {
    fn event(
        _: &mut Self,
        _: &ExtDataControlManagerV1,
        _: <ExtDataControlManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtDataControlDeviceV1, ()> for Handler {
    fn event(
        state: &mut Self,
        _: &ExtDataControlDeviceV1,
        event: ext_data_control_device_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_device_v1::Event::DataOffer { id } => {
                state.with_selection(|s| s.pending.push(Offer { proxy: id, mimes: Vec::new() }));
            }
            ext_data_control_device_v1::Event::Selection { id } => {
                state.with_selection(|s| {
                    let chosen = id.and_then(|id| {
                        let at = s.pending.iter().position(|o| o.proxy == id)?;
                        Some(s.pending.swap_remove(at))
                    });
                    if let Some(old) = std::mem::replace(&mut s.current, chosen) {
                        old.proxy.destroy();
                    }
                });
                if state.ready {
                    (state.on_change)();
                }
            }
            // The primary selection isn't ours to follow, but its offer is ours to free.
            ext_data_control_device_v1::Event::PrimarySelection { id: Some(id) } => {
                state.with_selection(|s| s.pending.retain(|o| o.proxy != id));
                id.destroy();
            }
            _ => {}
        }
    }

    event_created_child!(Handler, ExtDataControlDeviceV1, [
        EVT_DATA_OFFER_OPCODE => (ExtDataControlOfferV1, ()),
    ]);
}

impl Dispatch<ExtDataControlOfferV1, ()> for Handler {
    fn event(
        state: &mut Self,
        offer: &ExtDataControlOfferV1,
        event: ext_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_data_control_offer_v1::Event::Offer { mime_type } = event {
            state.with_selection(|s| {
                if let Some(pending) = s.pending.iter_mut().find(|o| o.proxy == *offer) {
                    pending.mimes.push(mime_type);
                }
            });
        }
    }
}

impl Dispatch<ExtDataControlSourceV1, ()> for Handler {
    fn event(
        state: &mut Self,
        source: &ExtDataControlSourceV1,
        event: ext_data_control_source_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_data_control_source_v1::Event::Send { mime_type, fd } => {
                let contents = state.with_selection(|s| {
                    s.source.as_ref().filter(|(ours, _)| ours == source).map(|(_, c)| Arc::clone(c))
                });
                let Some(Some(contents)) = contents else {
                    return;
                };
                // Off this thread: a pipe holds only so much, and an app may read
                // what we're offering while this connection has more to do.
                let _ = std::thread::Builder::new().name("clipboard-send".into()).spawn(move || {
                    use std::io::Write;
                    if let Some((_, bytes)) = contents.iter().find(|(mime, _)| *mime == mime_type) {
                        let _ = std::fs::File::from(fd).write_all(bytes);
                    }
                });
            }
            ext_data_control_source_v1::Event::Cancelled => {
                state.with_selection(|s| {
                    if s.source.as_ref().is_some_and(|(ours, _)| ours == source) {
                        s.source = None;
                    }
                });
                source.destroy();
            }
            _ => {}
        }
    }
}
