// SPDX-License-Identifier: GPL-3.0-or-later

use std::{path::PathBuf, pin::Pin, sync::Arc};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QUrl};
use iroh::RelayUrl;
use tokio::task::JoinHandle;

use crate::{
    anysend::{self, Event, Route, Selection},
    core_host, qr,
};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    #[auto_cxx_name]
    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, phase)]
        #[qproperty(QString, active_route)]
        #[qproperty(QString, error)]
        #[qproperty(QString, saved_path)]
        #[qproperty(f64, progress)]
        #[qproperty(i32, limit)]
        #[qproperty(i32, connected)]
        #[qproperty(i32, completed)]
        #[qproperty(i32, failed)]
        #[qproperty(QString, lan_path)]
        #[qproperty(i32, lan_size)]
        #[qproperty(QString, lan_error)]
        #[qproperty(QString, relay_path)]
        #[qproperty(i32, relay_size)]
        #[qproperty(QString, relay_error)]
        type AnySend = super::AnySendRust;

        #[qinvokable]
        fn start(
            self: Pin<&mut AnySend>,
            receive: bool,
            file_url: &QString,
            site_url: &QString,
            interface: &QString,
            limit: i32,
        );

        #[qinvokable]
        fn cancel(self: Pin<&mut AnySend>);
    }

    impl cxx_qt::Threading for AnySend {}
}

#[derive(Default)]
pub struct AnySendRust {
    phase: QString,
    active_route: QString,
    error: QString,
    saved_path: QString,
    progress: f64,
    limit: i32,
    connected: i32,
    completed: i32,
    failed: i32,
    lan_path: QString,
    lan_size: i32,
    lan_error: QString,
    relay_path: QString,
    relay_size: i32,
    relay_error: QString,
    generation: u64,
    task: Option<JoinHandle<()>>,
}

impl Drop for AnySendRust {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl qobject::AnySend {
    fn start(
        mut self: Pin<&mut Self>,
        receive: bool,
        file_url: &QString,
        site_url: &QString,
        interface: &QString,
        limit: i32,
    ) {
        self.as_mut().cancel();
        let Some(path) = QUrl::from(file_url).to_local_file().map(|path| PathBuf::from(path.to_string()))
        else {
            self.as_mut().event(Event::Failed("Choose a local file or folder.".to_owned()));
            return;
        };
        let site = match anysend::site(&site_url.to_string()) {
            Ok(site) => site,
            Err(error) => {
                self.as_mut().event(Event::Failed(error.to_string()));
                return;
            }
        };
        let selected = if receive { Selection::Receive(path) } else { Selection::Send(path) };
        let interface = interface.to_string();
        let relays = relay_candidates();
        self.as_mut().set_limit(limit);
        self.as_mut().set_phase(QString::from("waiting"));
        let qt = self.qt_thread();
        let generation = self.generation;
        let report = Arc::new(move |event| {
            let _ = qt.queue(move |object| {
                if object.generation == generation {
                    object.event(event);
                }
            });
        });
        let task = core_host::host().runtime.spawn(async move {
            if let Err(error) = anysend::run(selected, site, interface, relays, limit, report.clone()).await {
                report(Event::Failed(error.to_string()));
            }
        });
        self.as_mut().rust_mut().task = Some(task);
    }

    fn cancel(mut self: Pin<&mut Self>) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.generation = rust.generation.wrapping_add(1);
            if let Some(task) = rust.task.take() {
                task.abort();
            }
        }
        self.as_mut().set_phase(QString::default());
        self.as_mut().set_active_route(QString::default());
        self.as_mut().set_error(QString::default());
        self.as_mut().set_saved_path(QString::default());
        self.as_mut().set_progress(0.0);
        self.as_mut().set_limit(0);
        self.as_mut().set_connected(0);
        self.as_mut().set_completed(0);
        self.as_mut().set_failed(0);
        self.as_mut().clear_tickets();
    }

    fn clear_tickets(mut self: Pin<&mut Self>) {
        self.as_mut().set_lan_path(QString::default());
        self.as_mut().set_relay_path(QString::default());
        self.as_mut().set_lan_error(QString::default());
        self.as_mut().set_relay_error(QString::default());
    }

    fn event(mut self: Pin<&mut Self>, event: Event) {
        match event {
            Event::Ticket(route, url) => {
                if self.phase != QString::from("waiting") && self.phase != QString::from("transferring") {
                    return;
                }
                let Ok(code) = qr::encode(url.as_str()) else {
                    self.as_mut().route_error(route, "The address is too long for a QR code.");
                    return;
                };
                match route {
                    Route::Lan => {
                        self.as_mut().set_lan_size(code.size as i32);
                        self.as_mut().set_lan_path(QString::from(&code.path));
                    }
                    Route::Relay => {
                        self.as_mut().set_relay_size(code.size as i32);
                        self.as_mut().set_relay_path(QString::from(&code.path));
                    }
                }
            }
            Event::Unavailable(route, error) => {
                let active = if route == Route::Lan { "lan" } else { "relay" };
                if self.limit == 1
                    && self.phase == QString::from("transferring")
                    && self.active_route == QString::from(active)
                {
                    self.as_mut().set_phase(QString::from("failed"));
                    self.as_mut().set_error(QString::from(&error));
                } else {
                    self.as_mut().route_error(route, &error);
                }
            }
            Event::Started(route) => {
                self.as_mut().set_active_route(QString::from(if route == Route::Lan {
                    "lan"
                } else {
                    "relay"
                }));
                self.as_mut().set_phase(QString::from("transferring"));
            }
            Event::Guests { connected, completed, failed } => {
                self.as_mut().set_connected(connected as i32);
                self.as_mut().set_completed(completed as i32);
                self.as_mut().set_failed(failed as i32);
            }
            Event::Progress(bytes, total) => {
                self.as_mut().set_progress(bytes as f64 / total as f64);
            }
            Event::Finished(path) => {
                if let Some(path) = path {
                    self.as_mut().set_saved_path(QString::from(path.to_string_lossy().as_ref()));
                }
                self.as_mut().set_progress(1.0);
                self.as_mut().set_phase(QString::from("finished"));
            }
            Event::Failed(error) => {
                self.as_mut().clear_tickets();
                self.as_mut().set_error(QString::from(&error));
                self.as_mut().set_phase(QString::from("failed"));
            }
            Event::Expired => {
                self.as_mut().clear_tickets();
                if self.phase == QString::from("waiting") || self.phase == QString::from("transferring") {
                    self.as_mut().set_phase(QString::from("expired"));
                }
            }
        }
    }

    fn route_error(mut self: Pin<&mut Self>, route: Route, error: &str) {
        match route {
            Route::Lan => {
                self.as_mut().set_lan_path(QString::default());
                self.as_mut().set_lan_error(QString::from(error));
            }
            Route::Relay => {
                self.as_mut().set_relay_path(QString::default());
                self.as_mut().set_relay_error(QString::from(error));
            }
        }
    }
}

fn relay_candidates() -> Vec<RelayUrl> {
    let Some(snapshot) = core_host::node().map(|node| node.relay_snapshot()) else {
        return Vec::new();
    };
    let mut servers: Vec<_> = snapshot.servers.into_iter().filter(|server| server.enabled).collect();
    servers.sort_by_key(|server| {
        let home = snapshot.home.iter().any(|entry| entry.connected && entry.url == server.url);
        (!home, !server.provider.is_empty())
    });
    servers
        .into_iter()
        .filter_map(|server| server.url.parse::<RelayUrl>().ok())
        .filter(|url| url.scheme() == "https")
        .collect()
}
