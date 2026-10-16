use std::sync::Mutex;

use nectarlink_core::{DeviceId, MediaAction};

use crate::state::PlayerView;

#[derive(Debug, Clone)]
pub struct Shown {
    pub device: DeviceId,
    pub device_name: String,
    pub view: PlayerView,
}

static SHOWN: Mutex<Option<Shown>> = Mutex::new(None);

pub fn start<F>(_on_command: F)
where
    F: Fn(DeviceId, String, MediaAction, Option<u64>) + Send + Sync + 'static,
{
}

pub fn show(current: Option<Shown>) {
    if let Ok(mut guard) = SHOWN.lock() {
        *guard = current;
    }
}
