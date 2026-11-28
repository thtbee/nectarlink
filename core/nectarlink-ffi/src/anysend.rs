// SPDX-License-Identifier: MPL-2.0

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use nectarlink_anysend::{self as anysend, Event, Network, Route, Selection};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{FileToSend, NectarlinkError, NectarlinkNode, Result, file_to_send};

#[derive(Debug, uniffi::Enum)]
pub enum AnySendRoute {
    Lan,
    Relay,
}

impl From<Route> for AnySendRoute {
    fn from(route: Route) -> Self {
        match route {
            Route::Lan => Self::Lan,
            Route::Relay => Self::Relay,
        }
    }
}

#[derive(Debug, uniffi::Enum)]
pub enum AnySendEvent {
    Ticket { route: AnySendRoute, url: String },
    Unavailable { route: AnySendRoute, reason: String },
    Started { route: AnySendRoute },
    Guests { connected: u32, completed: u32, failed: u32 },
    Progress { bytes: u64, total: u64 },
    Received { path: String },
    Finished,
    Failed { reason: String },
    Expired,
}

impl From<Event> for AnySendEvent {
    fn from(event: Event) -> Self {
        match event {
            Event::Ticket(route, url) => Self::Ticket { route: route.into(), url: url.into() },
            Event::Unavailable(route, reason) => Self::Unavailable { route: route.into(), reason },
            Event::Started(route) => Self::Started { route: route.into() },
            Event::Guests { connected, completed, failed } => Self::Guests {
                connected: connected as u32,
                completed: completed as u32,
                failed: failed as u32,
            },
            Event::Progress(bytes, total) => Self::Progress { bytes, total },
            Event::Received(path) => Self::Received { path: path.to_string_lossy().into_owned() },
            Event::Finished(_) => Self::Finished,
            Event::Failed(reason) => Self::Failed { reason },
            Event::Expired => Self::Expired,
        }
    }
}

#[derive(Debug, uniffi::Object)]
pub struct AnySendSession {
    events: tokio::sync::Mutex<mpsc::UnboundedReceiver<AnySendEvent>>,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for AnySendSession {
    fn drop(&mut self) {
        if let Some(task) = self.task.get_mut().expect("AnySend task mutex").take() {
            task.abort();
        }
    }
}

#[uniffi::export]
impl AnySendSession {
    pub async fn next_event(&self) -> Option<AnySendEvent> {
        self.events.lock().await.recv().await
    }

    pub async fn stop(&self) {
        let task = self.task.lock().expect("AnySend task mutex").take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
}

#[uniffi::export]
impl NectarlinkNode {
    pub fn start_any_send(
        &self,
        directory: String,
        file: Option<FileToSend>,
        site: String,
        lan_ip: Option<String>,
        limit: i32,
    ) -> Result<Arc<AnySendSession>> {
        let selection = match file {
            Some(file) => {
                let file = file_to_send(file)?;
                Selection::Send { source: file.source, name: Some(file.name) }
            }
            None => Selection::Receive(PathBuf::from(directory)),
        };
        let site =
            anysend::site(&site).map_err(|error| NectarlinkError::Internal { reason: error.to_string() })?;
        let network = Network {
            lan_ip: lan_ip.and_then(|ip| ip.parse().ok()),
            relays: anysend::relay_candidates(Some(self.node.relay_snapshot())),
            ..Network::default()
        };
        let (send, events) = mpsc::unbounded_channel();
        let report = Arc::new(move |event: Event| {
            let _ = send.send(event.into());
        });
        let task = self.runtime.spawn(async move {
            if let Err(error) = anysend::run(selection, site, network, limit, report.clone()).await {
                report(Event::Failed(error.to_string()));
            }
        });
        Ok(Arc::new(AnySendSession { events: events.into(), task: Mutex::new(Some(task)) }))
    }
}
