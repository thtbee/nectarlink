use std::fmt;
use std::path::PathBuf;
use std::process::Command;

pub const AUMID: &str = "Nectarlink.Desktop";
pub const CALL_ANSWER: &str = "answer";
pub const CALL_DECLINE: &str = "decline";
pub const CALL_END: &str = "end";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastEvent {
    Opened {
        device: String,
        key: String,
    },
    Action {
        device: String,
        key: String,
        action: String,
    },
    Reply {
        device: String,
        key: String,
        action: String,
        text: String,
    },
    Dismissed {
        device: String,
        key: String,
    },
}

#[derive(Clone)]
pub struct Toast {
    pub device: String,
    pub key: String,
    pub title: String,
    pub body: String,
    pub attribution: String,
    pub icon: Option<PathBuf>,
    pub image: Option<PathBuf>,
    pub actions: Vec<(String, String)>,
    pub reply: Option<(String, String)>,
    pub silent: bool,
    pub progress: Option<Progress>,
    pub call: bool,
}

impl fmt::Debug for Toast {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Toast")
            .field("device", &self.device)
            .field("key", &self.key)
            .field("title", &self.title)
            .field("body", &self.body)
            .field("attribution", &self.attribution)
            .field("icon", &self.icon)
            .field("image", &self.image)
            .field("actions", &self.actions.len())
            .field("reply", &self.reply.is_some())
            .field("silent", &self.silent)
            .field("progress", &self.progress)
            .field("call", &self.call)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub status: String,
    pub value: Option<f64>,
    pub label: String,
}

pub fn enabled() -> bool {
    true
}

pub fn set_process_id() {}

pub fn start<F>(_icon: PathBuf, _on_event: F)
where
    F: Fn(ToastEvent) + Send + Sync + 'static,
{
}

pub fn show(toast: Toast) {
    let mut cmd = Command::new("notify-send");
    cmd.args(["--app-name", "NectarLink"]);
    if toast.call {
        cmd.args(["--urgency", "critical"]);
    }
    if let Some(icon) = &toast.icon {
        cmd.arg("--icon").arg(icon);
    }
    let summary = if toast.title.is_empty() {
        "NectarLink"
    } else {
        &toast.title
    };
    cmd.arg(summary);
    if !toast.body.is_empty() {
        cmd.arg(&toast.body);
    }
    let _ = cmd.spawn();
}

pub fn update_progress(_device: &str, _key: &str, _progress: Progress) {}

pub fn remove(_device: &str, _key: &str) {}

pub fn remove_device(_device: &str) {}

pub fn unregister() {}
