// SPDX-License-Identifier: MPL-2.0
//! The capability matrix (docs/architecture/capabilities.md): for every
//! feature and paired device, whether the feature is available, partly
//! available, locked behind something the user can do, or unsupported.
//!
//! Requirements name the **phone** and the **desktop**, never "local" and
//! "peer", so both apps compute the same matrix for a pair and can never
//! disagree about what works. Everything here is pure; [`crate::Node`]
//! gathers the inputs and emits the result.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use nectarlink_protocol::{
    DeviceId,
    messages::{DeviceInfo, DeviceKind, PowerLevel},
};

use crate::events::ConnectionPath;

/// Which device of a pair a requirement is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Phone,
    Desktop,
}

/// An Android runtime permission (or special access) a feature needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Permission {
    NotificationAccess,
    Sms,
    Phone,
    Contacts,
    Photos,
    Camera,
    Microphone,
}

impl Permission {
    pub const fn as_str(self) -> &'static str {
        match self {
            Permission::NotificationAccess => "notification_access",
            Permission::Sms => "sms",
            Permission::Phone => "phone",
            Permission::Contacts => "contacts",
            Permission::Photos => "photos",
            Permission::Camera => "camera",
            Permission::Microphone => "microphone",
        }
    }
}

/// How a missing capability can be obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unlock {
    /// The phone offers it from this power level on.
    Power(PowerLevel),
    /// The phone offers it once this permission is granted.
    Permission(Permission),
    /// The desktop offers it once this add-on is installed.
    Addon(&'static str),
    /// Every current app version offers it, so the device's app is too old.
    UpdateApp,
}

/// One condition a feature depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    /// The device in `role` announces capability `id` (protocol §6).
    Cap { role: Role, id: &'static str, unlock: Unlock },
    /// The phone runs at this power level or higher.
    PowerAtLeast(PowerLevel),
    /// The phone runs this Android release or newer.
    AndroidAtLeast(u32),
    /// The desktop runs this Windows build or newer.
    WindowsBuildAtLeast(u32),
    /// This connection path is enabled (e.g. Away mode for relays).
    Path(ConnectionPath),
    /// The user allows this for the device (see [`DEVICE_TOGGLES`]).
    DeviceToggle(&'static str),
}

/// A weaker set of requirements that still gives a useful, limited feature.
#[derive(Debug, Clone, Copy)]
pub struct PartialDef {
    pub requires: &'static [Requirement],
    /// Translation key describing the limitation, e.g. `clipboard.limit.manual`.
    pub limit: &'static str,
}

/// UI grouping of features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FeatureGroup {
    Device,
    Notifications,
    Messages,
    Calls,
    Clipboard,
    Files,
    Mirroring,
    Media,
    Camera,
    Input,
    Remote,
}

/// A feature, declared once and shared by both apps.
#[derive(Debug, Clone, Copy)]
pub struct FeatureDef {
    /// Stable ID, e.g. `clipboard.auto_phone_to_pc`.
    pub id: &'static str,
    pub group: FeatureGroup,
    /// Every requirement must hold for the feature to be available.
    pub requires: &'static [Requirement],
    pub partial: Option<PartialDef>,
}

/// What the user can do to unlock a feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeAction {
    /// Open "Choose your power" at this level.
    RaisePower(PowerLevel),
    /// Ask for a permission on the phone.
    GrantPermission(Permission),
    /// Install a desktop add-on (e.g. the virtual camera).
    EnableAddon(&'static str),
    /// Turn on a connection path (e.g. Away mode).
    EnablePath(ConnectionPath),
    /// Allow something for this device.
    EnableDeviceToggle(&'static str),
    /// Update the app on that device.
    UpdateApp(Role),
}

impl UpgradeAction {
    /// A stable `(action, target)` pair for UIs and bindings, e.g.
    /// `("raisePower", "elevated")` or `("enableToggle", "clipboard")`.
    pub fn describe(self) -> (&'static str, String) {
        match self {
            UpgradeAction::RaisePower(level) => (
                "raisePower",
                match level {
                    PowerLevel::Assist => "assist",
                    PowerLevel::Elevated => "elevated",
                    _ => "basic",
                }
                .into(),
            ),
            UpgradeAction::GrantPermission(p) => ("grantPermission", p.as_str().into()),
            UpgradeAction::EnableAddon(addon) => ("enableAddon", addon.into()),
            UpgradeAction::EnablePath(ConnectionPath::Relay) => ("enablePath", "relay".into()),
            UpgradeAction::EnablePath(ConnectionPath::Lan) => ("enablePath", "lan".into()),
            UpgradeAction::EnableDeviceToggle(toggle) => ("enableToggle", toggle.into()),
            UpgradeAction::UpdateApp(Role::Phone) => ("updateApp", "phone".into()),
            UpgradeAction::UpdateApp(Role::Desktop) => ("updateApp", "desktop".into()),
        }
    }
}

/// Roughly how long an upgrade takes, so the UI can say "~2 min".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effort {
    Instant,
    Minutes(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Upgrade {
    pub action: UpgradeAction,
    pub effort: Effort,
}

impl Upgrade {
    fn new(action: UpgradeAction) -> Self {
        let effort = match action {
            UpgradeAction::RaisePower(PowerLevel::Elevated) => Effort::Minutes(2),
            UpgradeAction::RaisePower(_) => Effort::Minutes(1),
            UpgradeAction::EnableAddon(_) => Effort::Minutes(1),
            UpgradeAction::UpdateApp(_) => Effort::Minutes(2),
            UpgradeAction::GrantPermission(_)
            | UpgradeAction::EnablePath(_)
            | UpgradeAction::EnableDeviceToggle(_) => Effort::Instant,
        };
        Upgrade { action, effort }
    }
}

/// Why a feature can't work on this pair. Explained, not actionable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedReason {
    /// Features need one phone and one desktop.
    DeviceKinds,
    AndroidTooOld {
        needs: u32,
    },
    WindowsTooOld {
        needs_build: u32,
    },
    /// The phone has the needed power level but still doesn't offer the
    /// capability (hardware or OEM limitation).
    NotOnThisDevice,
}

impl UnsupportedReason {
    /// A stable key for UIs and bindings: `deviceKinds`, `android:<release>`,
    /// `windows:<build>` or `notOnThisDevice`.
    pub fn describe(self) -> String {
        match self {
            UnsupportedReason::DeviceKinds => "deviceKinds".into(),
            UnsupportedReason::AndroidTooOld { needs } => format!("android:{needs}"),
            UnsupportedReason::WindowsTooOld { needs_build } => format!("windows:{needs_build}"),
            UnsupportedReason::NotOnThisDevice => "notOnThisDevice".into(),
        }
    }
}

impl Effort {
    /// Whole minutes (0 for instant).
    pub fn minutes(self) -> u8 {
        match self {
            Effort::Instant => 0,
            Effort::Minutes(m) => m,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    Available,
    /// Works, with a limitation shown inline, and how to remove it if possible.
    Partial {
        limit: &'static str,
        upgrade: Option<Upgrade>,
    },
    /// Doesn't work yet, but the user can unlock it.
    Locked {
        upgrade: Upgrade,
    },
    Unsupported {
        reason: UnsupportedReason,
    },
}

/// Every feature's state for one paired device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityMatrix {
    pub device: DeviceId,
    pub features: BTreeMap<&'static str, FeatureState>,
}

impl CapabilityMatrix {
    pub fn state(&self, feature: &str) -> Option<FeatureState> {
        self.features.get(feature).copied()
    }
}

/// What the matrix knows about one device of the pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceFacts {
    pub kind: DeviceKind,
    pub os: String,
    pub os_ver: String,
    pub caps: BTreeSet<String>,
    pub power: PowerLevel,
}

impl DeviceFacts {
    pub fn new(info: &DeviceInfo, caps: BTreeSet<String>, power: PowerLevel) -> Self {
        DeviceFacts { kind: info.kind, os: info.os.clone(), os_ver: info.os_ver.clone(), caps, power }
    }

    fn role(&self) -> Option<Role> {
        role_from(self.kind, &self.os)
    }

    /// The Android release (`"16"` → 16), if this is an Android device.
    fn android_release(&self) -> Option<u32> {
        (self.os == "android").then(|| leading_number(&self.os_ver)).flatten()
    }

    /// The Windows build (`"10.0.26200"` → 26200), if this is a Windows device.
    fn windows_build(&self) -> Option<u32> {
        (self.os == "windows").then(|| self.os_ver.split('.').nth(2).and_then(leading_number)).flatten()
    }
}

/// Which role a device plays in a pair, from its kind (or OS when the kind
/// is unknown to this version).
pub(crate) fn role_of(info: &DeviceInfo) -> Option<Role> {
    role_from(info.kind, &info.os)
}

fn role_from(kind: DeviceKind, os: &str) -> Option<Role> {
    match kind {
        DeviceKind::Phone | DeviceKind::Tablet => Some(Role::Phone),
        DeviceKind::Desktop | DeviceKind::Laptop => Some(Role::Desktop),
        DeviceKind::Unknown => match os {
            "android" => Some(Role::Phone),
            "windows" | "macos" => Some(Role::Desktop),
            _ => None,
        },
    }
}

fn leading_number(s: &str) -> Option<u32> {
    let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    s[..end].parse().ok()
}

/// Most capabilities a device may announce; extra ones are ignored.
const MAX_CAPABILITIES: usize = 256;
const MAX_CAPABILITY_LEN: usize = 64;

/// Keeps the well-formed capability IDs a peer announced (`[a-z0-9._-]`,
/// bounded in length and count), so stored and displayed data stays sane.
pub(crate) fn sanitize_capabilities(caps: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    caps.into_iter()
        .filter(|c| {
            (1..=MAX_CAPABILITY_LEN).contains(&c.len())
                && c.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
        .take(MAX_CAPABILITIES)
        .collect()
}

/// Per-device permissions the user can switch, with their defaults.
pub const DEVICE_TOGGLES: &[(&str, bool)] = &[
    ("notifications", true),
    ("messages", true),
    ("calls", true),
    ("clipboard", true),
    ("files", true),
    ("media", true),
    ("photos", true),
    ("pc_actions", true),
    ("mirroring", true),
    // Browsing the PC's files from anywhere is opt-in.
    ("remote_files", false),
];

/// The default for a toggle, or `None` if no such toggle exists.
pub fn toggle_default(toggle: &str) -> Option<bool> {
    DEVICE_TOGGLES.iter().find(|(name, _)| *name == toggle).map(|(_, on)| *on)
}

/// Inputs for one pair.
#[derive(Debug, Clone, Copy)]
pub struct MatrixInputs<'a> {
    pub local: &'a DeviceFacts,
    pub peer: &'a DeviceFacts,
    /// Connection paths that are enabled on this device.
    pub paths: &'a [ConnectionPath],
    /// The user's per-device toggles; missing ones use their default.
    pub toggles: &'a HashMap<String, bool>,
}

/// Computes the state of every feature in [`FEATURES`] for one pair.
pub fn compute(device: DeviceId, inputs: MatrixInputs<'_>) -> CapabilityMatrix {
    let pair = match (inputs.local.role(), inputs.peer.role()) {
        (Some(Role::Phone), Some(Role::Desktop)) => Some((inputs.local, inputs.peer)),
        (Some(Role::Desktop), Some(Role::Phone)) => Some((inputs.peer, inputs.local)),
        _ => None,
    };
    let features = FEATURES
        .iter()
        .map(|def| {
            let state = match pair {
                Some((phone, desktop)) => {
                    let ctx = Context { phone, desktop, paths: inputs.paths, toggles: inputs.toggles };
                    evaluate(def, &ctx)
                }
                None => FeatureState::Unsupported { reason: UnsupportedReason::DeviceKinds },
            };
            (def.id, state)
        })
        .collect();
    CapabilityMatrix { device, features }
}

struct Context<'a> {
    phone: &'a DeviceFacts,
    desktop: &'a DeviceFacts,
    paths: &'a [ConnectionPath],
    toggles: &'a HashMap<String, bool>,
}

/// The outcome of checking one requirement.
enum Check {
    Met,
    Unlockable(Upgrade),
    Impossible(UnsupportedReason),
}

/// Basic < Assist < Elevated. Desktops and unknown values count as Basic.
fn power_rank(level: PowerLevel) -> u8 {
    match level {
        PowerLevel::Assist => 1,
        PowerLevel::Elevated => 2,
        PowerLevel::Basic | PowerLevel::NotApplicable | PowerLevel::Unknown => 0,
    }
}

fn check(req: &Requirement, ctx: &Context<'_>) -> Check {
    match *req {
        Requirement::Cap { role, id, unlock } => {
            let facts = match role {
                Role::Phone => ctx.phone,
                Role::Desktop => ctx.desktop,
            };
            if facts.caps.contains(id) {
                return Check::Met;
            }
            match unlock {
                Unlock::Power(level) if power_rank(ctx.phone.power) < power_rank(level) => {
                    Check::Unlockable(Upgrade::new(UpgradeAction::RaisePower(level)))
                }
                Unlock::Power(_) => Check::Impossible(UnsupportedReason::NotOnThisDevice),
                Unlock::Permission(p) => Check::Unlockable(Upgrade::new(UpgradeAction::GrantPermission(p))),
                Unlock::Addon(addon) => Check::Unlockable(Upgrade::new(UpgradeAction::EnableAddon(addon))),
                Unlock::UpdateApp => Check::Unlockable(Upgrade::new(UpgradeAction::UpdateApp(role))),
            }
        }
        Requirement::PowerAtLeast(level) => {
            if power_rank(ctx.phone.power) >= power_rank(level) {
                Check::Met
            } else {
                Check::Unlockable(Upgrade::new(UpgradeAction::RaisePower(level)))
            }
        }
        // An unparseable version doesn't block a feature: the phone only
        // advertises capabilities it can actually provide.
        Requirement::AndroidAtLeast(needs) => match ctx.phone.android_release() {
            Some(release) if release < needs => Check::Impossible(UnsupportedReason::AndroidTooOld { needs }),
            _ => Check::Met,
        },
        Requirement::WindowsBuildAtLeast(needs_build) => match ctx.desktop.windows_build() {
            Some(build) if build < needs_build => {
                Check::Impossible(UnsupportedReason::WindowsTooOld { needs_build })
            }
            _ => Check::Met,
        },
        Requirement::Path(path) => {
            if ctx.paths.contains(&path) {
                Check::Met
            } else {
                Check::Unlockable(Upgrade::new(UpgradeAction::EnablePath(path)))
            }
        }
        Requirement::DeviceToggle(toggle) => {
            let on = ctx.toggles.get(toggle).copied().or_else(|| toggle_default(toggle)).unwrap_or(false);
            if on {
                Check::Met
            } else {
                Check::Unlockable(Upgrade::new(UpgradeAction::EnableDeviceToggle(toggle)))
            }
        }
    }
}

/// Lowest-effort upgrade among unmet requirements, and the first reason the
/// set can't be met at all.
struct Summary {
    met: bool,
    upgrade: Option<Upgrade>,
    impossible: Option<UnsupportedReason>,
}

fn summarize(requires: &[Requirement], ctx: &Context<'_>) -> Summary {
    let mut summary = Summary { met: true, upgrade: None, impossible: None };
    for req in requires {
        match check(req, ctx) {
            Check::Met => {}
            Check::Unlockable(upgrade) => {
                summary.met = false;
                // Keep the first of equally easy upgrades (declaration order).
                if summary.upgrade.is_none_or(|best| upgrade.effort < best.effort) {
                    summary.upgrade = Some(upgrade);
                }
            }
            Check::Impossible(reason) => {
                summary.met = false;
                summary.impossible.get_or_insert(reason);
            }
        }
    }
    summary
}

/// Picks the state (docs/architecture/capabilities.md, "How a state is chosen"):
/// Available if everything holds; else Partial if the weaker set holds (part
/// of the feature works even if the rest never will); else Unsupported if
/// something can't be met on this pair; else Locked with the easiest upgrade.
fn evaluate(def: &FeatureDef, ctx: &Context<'_>) -> FeatureState {
    let full = summarize(def.requires, ctx);
    if full.met {
        return FeatureState::Available;
    }
    if let Some(partial) = def.partial
        && summarize(partial.requires, ctx).met
    {
        let upgrade = if full.impossible.is_some() { None } else { full.upgrade };
        return FeatureState::Partial { limit: partial.limit, upgrade };
    }
    match (full.impossible, full.upgrade) {
        (Some(reason), _) => FeatureState::Unsupported { reason },
        (None, Some(upgrade)) => FeatureState::Locked { upgrade },
        // Unreachable: an unmet set has an upgrade or a reason.
        (None, None) => FeatureState::Unsupported { reason: UnsupportedReason::NotOnThisDevice },
    }
}

// ---- The registry ----

use Requirement::{AndroidAtLeast, DeviceToggle, Path, PowerAtLeast, WindowsBuildAtLeast};

const fn phone(id: &'static str, unlock: Unlock) -> Requirement {
    Requirement::Cap { role: Role::Phone, id, unlock }
}

const fn desktop(id: &'static str, unlock: Unlock) -> Requirement {
    Requirement::Cap { role: Role::Desktop, id, unlock }
}

const ELEVATED: Unlock = Unlock::Power(PowerLevel::Elevated);
const ASSIST: Unlock = Unlock::Power(PowerLevel::Assist);
const UPDATE: Unlock = Unlock::UpdateApp;

/// Every feature, in display order. Capability IDs are listed in
/// docs/protocol/capabilities.md.
pub const FEATURES: &[FeatureDef] = &[
    // Device
    FeatureDef {
        id: "device.find_phone",
        group: FeatureGroup::Device,
        requires: &[phone("device.ring", UPDATE)],
        partial: None,
    },
    FeatureDef {
        id: "device.battery",
        group: FeatureGroup::Device,
        requires: &[phone("device.battery", UPDATE)],
        partial: None,
    },
    FeatureDef {
        id: "device.pc_actions",
        group: FeatureGroup::Device,
        requires: &[desktop("pc.power", UPDATE), DeviceToggle("pc_actions")],
        partial: None,
    },
    FeatureDef {
        id: "device.links_to_pc",
        group: FeatureGroup::Device,
        requires: &[desktop("link.open", UPDATE)],
        partial: None,
    },
    FeatureDef {
        id: "device.links_to_phone",
        group: FeatureGroup::Device,
        requires: &[phone("link.open", UPDATE)],
        partial: None,
    },
    // Notifications
    FeatureDef {
        id: "notifications.mirror",
        group: FeatureGroup::Notifications,
        requires: &[
            phone("notify.mirror", Unlock::Permission(Permission::NotificationAccess)),
            DeviceToggle("notifications"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "notifications.reply",
        group: FeatureGroup::Notifications,
        requires: &[
            phone("notify.reply", Unlock::Permission(Permission::NotificationAccess)),
            DeviceToggle("notifications"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "notifications.sensitive",
        group: FeatureGroup::Notifications,
        requires: &[phone("notify.sensitive", ELEVATED), DeviceToggle("notifications")],
        partial: None,
    },
    // Messages & calls
    FeatureDef {
        id: "messages.sms",
        group: FeatureGroup::Messages,
        requires: &[
            phone("sms.read", Unlock::Permission(Permission::Sms)),
            phone("sms.send", Unlock::Permission(Permission::Sms)),
            DeviceToggle("messages"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "calls.alerts",
        group: FeatureGroup::Calls,
        requires: &[phone("call.state", Unlock::Permission(Permission::Phone)), DeviceToggle("calls")],
        partial: None,
    },
    FeatureDef {
        id: "calls.control",
        group: FeatureGroup::Calls,
        requires: &[
            phone("call.state", Unlock::Permission(Permission::Phone)),
            phone("call.control", Unlock::Permission(Permission::Phone)),
            DeviceToggle("calls"),
        ],
        partial: None,
    },
    // Clipboard
    FeatureDef {
        id: "clipboard.pc_to_phone",
        group: FeatureGroup::Clipboard,
        requires: &[phone("clip.write", UPDATE), DeviceToggle("clipboard")],
        partial: None,
    },
    FeatureDef {
        id: "clipboard.auto_phone_to_pc",
        group: FeatureGroup::Clipboard,
        requires: &[phone("clip.read.auto", ELEVATED), DeviceToggle("clipboard")],
        partial: Some(PartialDef {
            requires: &[phone("clip.share", UPDATE), DeviceToggle("clipboard")],
            limit: "clipboard.limit.manual",
        }),
    },
    // Files & photos
    FeatureDef {
        id: "files.send",
        group: FeatureGroup::Files,
        requires: &[
            phone("files.transfer", UPDATE),
            desktop("files.transfer", UPDATE),
            DeviceToggle("files"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "files.recent_photos",
        group: FeatureGroup::Files,
        requires: &[phone("photos.read", Unlock::Permission(Permission::Photos)), DeviceToggle("photos")],
        partial: None,
    },
    // Mirroring
    FeatureDef {
        id: "mirroring.view",
        group: FeatureGroup::Mirroring,
        requires: &[
            phone("mirror.capture", UPDATE),
            desktop("mirror.view", UPDATE),
            DeviceToggle("mirroring"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "mirroring.control",
        group: FeatureGroup::Mirroring,
        requires: &[
            phone("mirror.capture", UPDATE),
            phone("mirror.input", ASSIST),
            desktop("mirror.view", UPDATE),
            DeviceToggle("mirroring"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "mirroring.app_windows",
        group: FeatureGroup::Mirroring,
        requires: &[
            phone("mirror.virtual_display", ELEVATED),
            PowerAtLeast(PowerLevel::Elevated),
            AndroidAtLeast(11),
            desktop("mirror.view", UPDATE),
            DeviceToggle("mirroring"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "mirroring.audio",
        group: FeatureGroup::Mirroring,
        requires: &[
            phone("mirror.audio", ELEVATED),
            desktop("mirror.listen", UPDATE),
            DeviceToggle("mirroring"),
        ],
        partial: Some(PartialDef {
            requires: &[
                phone("mirror.audio.playback", UPDATE),
                desktop("mirror.listen", UPDATE),
                DeviceToggle("mirroring"),
            ],
            limit: "mirroring.limit.audio_some_apps",
        }),
    },
    // Media
    FeatureDef {
        id: "media.phone_control",
        group: FeatureGroup::Media,
        requires: &[
            phone("media.control", Unlock::Permission(Permission::NotificationAccess)),
            desktop("media.remote", UPDATE),
            DeviceToggle("media"),
        ],
        partial: None,
    },
    FeatureDef {
        id: "media.pc_control",
        group: FeatureGroup::Media,
        requires: &[desktop("media.control", UPDATE), phone("media.remote", UPDATE), DeviceToggle("media")],
        partial: None,
    },
    // Camera
    FeatureDef {
        id: "camera.webcam",
        group: FeatureGroup::Camera,
        requires: &[
            phone("camera.stream", Unlock::Permission(Permission::Camera)),
            desktop("addon.vcam", Unlock::Addon("vcam")),
            WindowsBuildAtLeast(22000),
        ],
        partial: None,
    },
    // Input
    FeatureDef {
        id: "input.remote",
        group: FeatureGroup::Input,
        requires: &[desktop("input.inject", UPDATE)],
        partial: None,
    },
    FeatureDef {
        id: "input.deck",
        group: FeatureGroup::Input,
        requires: &[desktop("deck.actions", UPDATE)],
        partial: None,
    },
    // Away
    FeatureDef {
        id: "remote.pc_files",
        group: FeatureGroup::Remote,
        requires: &[
            desktop("files.browse", UPDATE),
            Path(ConnectionPath::Relay),
            DeviceToggle("remote_files"),
        ],
        partial: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(kind: DeviceKind, os: &str, os_ver: &str, power: PowerLevel, caps: &[&str]) -> DeviceFacts {
        DeviceFacts {
            kind,
            os: os.into(),
            os_ver: os_ver.into(),
            caps: caps.iter().map(|c| (*c).to_owned()).collect(),
            power,
        }
    }

    /// What a phone at Basic offers with every Basic permission granted.
    const BASIC_PHONE: &[&str] = &[
        "device.ring",
        "device.battery",
        "notify.mirror",
        "notify.reply",
        "sms.read",
        "sms.send",
        "call.state",
        "clip.write",
        "clip.share",
        "files.transfer",
        "photos.read",
        "mirror.capture",
        "mirror.audio.playback",
        "media.control",
        "media.remote",
        "camera.stream",
    ];
    const PC: &[&str] = &[
        "pc.power",
        "files.transfer",
        "files.browse",
        "media.control",
        "media.remote",
        "input.inject",
        "deck.actions",
        "mirror.view",
        "mirror.listen",
    ];

    fn pc() -> DeviceFacts {
        facts(DeviceKind::Laptop, "windows", "10.0.26200", PowerLevel::NotApplicable, PC)
    }

    fn matrix_with(
        phone: &DeviceFacts,
        pc: &DeviceFacts,
        toggles: &HashMap<String, bool>,
    ) -> CapabilityMatrix {
        let paths = [ConnectionPath::Lan];
        compute(DeviceId([7; 32]), MatrixInputs { local: pc, peer: phone, paths: &paths, toggles })
    }

    fn matrix(phone: &DeviceFacts) -> CapabilityMatrix {
        matrix_with(phone, &pc(), &HashMap::new())
    }

    fn locked(action: UpgradeAction, effort: Effort) -> FeatureState {
        FeatureState::Locked { upgrade: Upgrade { action, effort } }
    }

    #[test]
    fn basic_phone_matches_the_documented_examples() {
        let m = matrix(&facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, BASIC_PHONE));
        assert_eq!(m.state("clipboard.pc_to_phone"), Some(FeatureState::Available));
        assert_eq!(
            m.state("clipboard.auto_phone_to_pc"),
            Some(FeatureState::Partial {
                limit: "clipboard.limit.manual",
                upgrade: Some(Upgrade {
                    action: UpgradeAction::RaisePower(PowerLevel::Elevated),
                    effort: Effort::Minutes(2)
                }),
            })
        );
        assert_eq!(
            m.state("notifications.sensitive"),
            Some(locked(UpgradeAction::RaisePower(PowerLevel::Elevated), Effort::Minutes(2)))
        );
        assert_eq!(
            m.state("mirroring.control"),
            Some(locked(UpgradeAction::RaisePower(PowerLevel::Assist), Effort::Minutes(1)))
        );
        assert_eq!(
            m.state("camera.webcam"),
            Some(locked(UpgradeAction::EnableAddon("vcam"), Effort::Minutes(1)))
        );
        // Away mode is off and remote files are opt-in: both are instant fixes,
        // the first one listed is offered.
        assert_eq!(
            m.state("remote.pc_files"),
            Some(locked(UpgradeAction::EnablePath(ConnectionPath::Relay), Effort::Instant))
        );
    }

    #[test]
    fn elevated_phone_unlocks_everything_it_offers() {
        let mut caps = BASIC_PHONE.to_vec();
        caps.extend([
            "notify.sensitive",
            "clip.read.auto",
            "mirror.input",
            "mirror.virtual_display",
            "mirror.audio",
        ]);
        let m = matrix(&facts(DeviceKind::Phone, "android", "16", PowerLevel::Elevated, &caps));
        for id in [
            "notifications.sensitive",
            "clipboard.auto_phone_to_pc",
            "mirroring.control",
            "mirroring.app_windows",
            "mirroring.audio",
        ] {
            assert_eq!(m.state(id), Some(FeatureState::Available), "{id}");
        }
    }

    #[test]
    fn missing_capability_at_the_right_power_is_unsupported() {
        // Elevated, but this phone can't do auto clipboard: manual still works.
        let m = matrix(&facts(DeviceKind::Phone, "android", "16", PowerLevel::Elevated, BASIC_PHONE));
        assert_eq!(
            m.state("clipboard.auto_phone_to_pc"),
            Some(FeatureState::Partial { limit: "clipboard.limit.manual", upgrade: None })
        );
        assert_eq!(
            m.state("notifications.sensitive"),
            Some(FeatureState::Unsupported { reason: UnsupportedReason::NotOnThisDevice })
        );
    }

    #[test]
    fn missing_permission_offers_the_permission() {
        let caps: Vec<&str> = BASIC_PHONE.iter().copied().filter(|c| *c != "notify.mirror").collect();
        let m = matrix(&facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, &caps));
        assert_eq!(
            m.state("notifications.mirror"),
            Some(locked(UpgradeAction::GrantPermission(Permission::NotificationAccess), Effort::Instant))
        );
    }

    #[test]
    fn the_easiest_upgrade_is_offered_first() {
        // Webcam needs the camera permission (instant) and the add-on (~1 min).
        let caps: Vec<&str> = BASIC_PHONE.iter().copied().filter(|c| *c != "camera.stream").collect();
        let m = matrix(&facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, &caps));
        assert_eq!(
            m.state("camera.webcam"),
            Some(locked(UpgradeAction::GrantPermission(Permission::Camera), Effort::Instant))
        );
    }

    #[test]
    fn toggles_lock_features_and_default_sensibly() {
        let phone = facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, BASIC_PHONE);
        let toggles = HashMap::from([("notifications".to_owned(), false)]);
        let m = matrix_with(&phone, &pc(), &toggles);
        assert_eq!(
            m.state("notifications.mirror"),
            Some(locked(UpgradeAction::EnableDeviceToggle("notifications"), Effort::Instant))
        );
        assert_eq!(m.state("messages.sms"), Some(FeatureState::Available));
    }

    #[test]
    fn old_systems_are_unsupported() {
        let phone =
            facts(DeviceKind::Phone, "android", "10", PowerLevel::Elevated, &["mirror.virtual_display"]);
        let win10 =
            facts(DeviceKind::Desktop, "windows", "10.0.19045", PowerLevel::NotApplicable, &["addon.vcam"]);
        let m = matrix_with(&phone, &win10, &HashMap::new());
        assert_eq!(
            m.state("mirroring.app_windows"),
            Some(FeatureState::Unsupported { reason: UnsupportedReason::AndroidTooOld { needs: 11 } })
        );
        let mut caps = BASIC_PHONE.to_vec();
        caps.push("camera.stream");
        let m = matrix_with(
            &facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, &caps),
            &win10,
            &HashMap::new(),
        );
        assert_eq!(
            m.state("camera.webcam"),
            Some(FeatureState::Unsupported {
                reason: UnsupportedReason::WindowsTooOld { needs_build: 22000 }
            })
        );
    }

    #[test]
    fn same_on_both_devices() {
        let phone = facts(DeviceKind::Phone, "android", "16", PowerLevel::Basic, BASIC_PHONE);
        let pc = pc();
        let paths = [ConnectionPath::Lan];
        let toggles = HashMap::new();
        let on_pc = compute(
            DeviceId([1; 32]),
            MatrixInputs { local: &pc, peer: &phone, paths: &paths, toggles: &toggles },
        );
        let on_phone = compute(
            DeviceId([1; 32]),
            MatrixInputs { local: &phone, peer: &pc, paths: &paths, toggles: &toggles },
        );
        assert_eq!(on_pc, on_phone);
    }

    #[test]
    fn pairs_without_a_phone_and_a_desktop_are_unsupported() {
        let m = matrix_with(&pc(), &pc(), &HashMap::new());
        assert!(
            m.features
                .values()
                .all(|s| *s == FeatureState::Unsupported { reason: UnsupportedReason::DeviceKinds })
        );
    }

    #[test]
    fn unknown_kinds_fall_back_to_the_os() {
        let phone = facts(DeviceKind::Unknown, "android", "16", PowerLevel::Basic, BASIC_PHONE);
        assert_eq!(matrix(&phone).state("device.find_phone"), Some(FeatureState::Available));
    }

    #[test]
    fn os_versions_parse() {
        let f = facts(DeviceKind::Desktop, "windows", "10.0.26200.6584", PowerLevel::NotApplicable, &[]);
        assert_eq!(f.windows_build(), Some(26200));
        let f = facts(DeviceKind::Phone, "android", "12L", PowerLevel::Basic, &[]);
        assert_eq!(f.android_release(), Some(12));
        let f = facts(DeviceKind::Phone, "android", "beta", PowerLevel::Basic, &[]);
        assert_eq!(f.android_release(), None);
    }

    #[test]
    fn sanitizes_announced_capabilities() {
        let caps = sanitize_capabilities(
            ["clip.write", "", "Bad.Upper", "has space", "a,b", "mirror.audio-v2_x", &"x".repeat(65)]
                .map(str::to_owned),
        );
        assert_eq!(caps.into_iter().collect::<Vec<_>>(), ["clip.write", "mirror.audio-v2_x"]);
        assert_eq!(sanitize_capabilities((0..1000).map(|i| format!("c{i}"))).len(), MAX_CAPABILITIES);
    }

    #[test]
    fn registry_is_consistent() {
        let mut ids = BTreeSet::new();
        for def in FEATURES {
            assert!(ids.insert(def.id), "duplicate feature {}", def.id);
            let partial = def.partial.map(|p| p.requires).unwrap_or(&[]);
            for req in def.requires.iter().chain(partial) {
                if let Requirement::DeviceToggle(t) = req {
                    assert!(toggle_default(t).is_some(), "{}: unknown toggle {t}", def.id);
                }
                if let Requirement::Cap { role: Role::Desktop, unlock: Unlock::Power(_), .. } = req {
                    panic!("{}: power levels only apply to phones", def.id);
                }
            }
        }
    }
}
