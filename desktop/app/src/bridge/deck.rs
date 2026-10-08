// SPDX-License-Identifier: GPL-3.0-or-later
//! `DeckController`: QML bridge for the PC-owned Deck editor and live preview.

#![allow(clippy::too_many_arguments)]

use std::pin::Pin;

use cxx_qt::Threading;
use cxx_qt_lib::{QString, QUrl};
use nectarlink_core::{DeckAction, KeyMod, deck_kinds};
use serde::Serialize;

use super::prefs::qobject;
use crate::{bridge::app::show_message, deck, state::Changes};

#[derive(Default)]
pub struct DeckControllerRust {
    pub(super) pages_json: QString,
    pub(super) volume: i32,
    pub(super) muted: bool,
    pub(super) mic_state: i32,
    pub(super) playing: bool,
}

#[derive(Serialize)]
struct UiTile {
    id: String,
    label: String,
    icon: String,
    color: String,
    kind: String,
    param: String,
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
    status: String,
    subtitle: String,
}

#[derive(Serialize)]
struct UiPage {
    id: String,
    name: String,
    tiles: Vec<UiTile>,
}

fn action_fields(action: &DeckAction) -> (String, bool, bool, bool, bool, String) {
    match action {
        DeckAction::MediaPlayPause => (String::new(), false, false, false, false, "Media key".into()),
        DeckAction::MediaNext => (String::new(), false, false, false, false, "Media key".into()),
        DeckAction::MediaPrevious => (String::new(), false, false, false, false, "Media key".into()),
        DeckAction::VolumeUp => (String::new(), false, false, false, false, "+2%".into()),
        DeckAction::VolumeDown => (String::new(), false, false, false, false, "−2%".into()),
        DeckAction::VolumeMute => (String::new(), false, false, false, false, "Speaker".into()),
        DeckAction::MicMute => (String::new(), false, false, false, false, "Microphone".into()),
        DeckAction::LockPc => (String::new(), false, false, false, false, "Win + L".into()),
        DeckAction::ShowDesktop => (String::new(), false, false, false, false, "Win + D".into()),
        DeckAction::SwitchWindow => (String::new(), false, false, false, false, "Alt + Tab".into()),
        DeckAction::Screenshot => (String::new(), false, false, false, false, "Win + Shift + S".into()),
        DeckAction::Shortcut { key, mods } => {
            let ctrl = mods.contains(&KeyMod::Ctrl);
            let alt = mods.contains(&KeyMod::Alt);
            let shift = mods.contains(&KeyMod::Shift);
            let win = mods.contains(&KeyMod::Win);
            let mut parts = Vec::new();
            if ctrl {
                parts.push("Ctrl".to_owned());
            }
            if alt {
                parts.push("Alt".to_owned());
            }
            if shift {
                parts.push("Shift".to_owned());
            }
            if win {
                parts.push("Win".to_owned());
            }
            parts.push(key.to_ascii_uppercase());
            (key.clone(), ctrl, alt, shift, win, parts.join(" + "))
        }
        DeckAction::OpenUrl { url } => (url.clone(), false, false, false, false, url.clone()),
        DeckAction::TypeText { text } => {
            let preview: String = text.chars().take(28).collect();
            (text.clone(), false, false, false, false, format!("\"{preview}\""))
        }
        DeckAction::LaunchApp { path } => {
            let file_name =
                std::path::Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or(path).to_owned();
            (path.clone(), false, false, false, false, file_name)
        }
        DeckAction::RunCommand { command } => {
            let preview: String = command.chars().take(28).collect();
            (command.clone(), false, false, false, false, format!("cmd · {preview}"))
        }
    }
}

impl cxx_qt::Initialize for qobject::DeckController {
    fn initialize(mut self: Pin<&mut Self>) {
        self.as_mut().refresh();
        super::subscribe(self.qt_thread(), Changes::DECK, Self::refresh);
    }
}

impl qobject::DeckController {
    fn refresh(mut self: Pin<&mut Self>) {
        let cfg = deck::config();
        let st = deck::state();
        let ui_pages: Vec<UiPage> = cfg
            .pages
            .iter()
            .map(|p| UiPage {
                id: p.id.clone(),
                name: p.name.clone(),
                tiles: p
                    .tiles
                    .iter()
                    .map(|t| {
                        let kind = t.action.kind().to_owned();
                        let (param, ctrl, alt, shift, win, raw_subtitle) = action_fields(&t.action);
                        let subtitle = if raw_subtitle.eq_ignore_ascii_case(t.label.trim()) {
                            String::new()
                        } else {
                            raw_subtitle
                        };
                        let status = st.tile_status(&kind).unwrap_or_default();
                        UiTile {
                            id: t.id.clone(),
                            label: t.label.clone(),
                            icon: t.icon.clone(),
                            color: t.color.clone(),
                            kind,
                            param,
                            ctrl,
                            alt,
                            shift,
                            win,
                            status,
                            subtitle,
                        }
                    })
                    .collect(),
            })
            .collect();
        let json = serde_json::to_string(&ui_pages).unwrap_or_else(|_| "[]".into());
        self.as_mut().set_pages_json(QString::from(&json));
        self.as_mut().set_volume(i32::from(st.volume));
        self.as_mut().set_muted(st.muted);
        self.as_mut().set_mic_state(match st.mic_muted {
            None => -1,
            Some(false) => 0,
            Some(true) => 1,
        });
        self.as_mut().set_playing(st.playing);
    }

    pub fn default_label_for(&self, kind: &QString) -> QString {
        QString::from(deck_kinds::default_label(&String::from(kind)))
    }

    pub fn default_icon_for(&self, kind: &QString) -> QString {
        QString::from(deck_kinds::default_icon(&String::from(kind)))
    }

    pub fn default_color_for(&self, kind: &QString) -> QString {
        QString::from(deck_kinds::default_color(&String::from(kind)))
    }

    pub fn add_page(self: Pin<&mut Self>, name: &QString) -> QString {
        let id = deck::add_page(&String::from(name));
        self.refresh();
        QString::from(&id)
    }

    pub fn rename_page(self: Pin<&mut Self>, page_id: &QString, name: &QString) {
        deck::rename_page(&String::from(page_id), &String::from(name));
        self.refresh();
    }

    pub fn remove_page(self: Pin<&mut Self>, page_id: &QString) {
        deck::remove_page(&String::from(page_id));
        self.refresh();
    }

    pub fn reset_default(self: Pin<&mut Self>) {
        deck::reset_default();
        self.refresh();
    }

    pub fn move_tile(self: Pin<&mut Self>, page_id: &QString, from_index: i32, to_index: i32) {
        if let (Ok(from), Ok(to)) = (usize::try_from(from_index), usize::try_from(to_index)) {
            deck::move_tile(&String::from(page_id), from, to);
            self.refresh();
        }
    }

    pub fn remove_tile(self: Pin<&mut Self>, page_id: &QString, tile_id: &QString) {
        deck::remove_tile(&String::from(page_id), &String::from(tile_id));
        self.refresh();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn save_tile(
        self: Pin<&mut Self>,
        page_id: &QString,
        tile_id: &QString,
        label: &QString,
        icon: &QString,
        color: &QString,
        kind: &QString,
        param: &QString,
        ctrl: bool,
        alt: bool,
        shift: bool,
        win_mod: bool,
    ) -> QString {
        let action =
            match deck::build_action(&String::from(kind), &String::from(param), ctrl, alt, shift, win_mod) {
                Ok(a) => a,
                Err(e) => return QString::from(&e),
            };
        match deck::save_tile(
            &String::from(page_id),
            &String::from(tile_id),
            &String::from(label),
            &String::from(icon),
            &String::from(color),
            action,
        ) {
            Ok(_) => {
                self.refresh();
                QString::from("")
            }
            Err(e) => QString::from(&e),
        }
    }

    pub fn test_tile(self: Pin<&mut Self>, tile_id: &QString) {
        let id = String::from(tile_id);
        if let Err(e) = deck::execute_tile(&id) {
            show_message(e);
        }
    }

    pub fn url_to_local_path(&self, url: &QString) -> QString {
        let raw = QUrl::from(url)
            .to_local_file()
            .map(|p| String::from(&p))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| String::from(url));
        QString::from(raw.trim())
    }

    pub fn set_page_active(&self, active: bool) {
        deck::set_page_active(active);
    }
}
