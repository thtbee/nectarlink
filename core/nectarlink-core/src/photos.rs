// SPDX-License-Identifier: MPL-2.0
//! Recent photos (docs/protocol/photos.md): a phone announces each new
//! photo or screenshot to PCs with a preview, and sends the photo itself
//! (as a files transfer) when a PC asks for it.

use std::{collections::VecDeque, sync::Arc};

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{PhotoGet, PhotoNew, PhotoSending, photos, types},
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session};

/// The device toggle that allows recent photos for a device.
pub(crate) const TOGGLE: &str = "photos";
/// A PC can ask for this many of the latest announced photos.
const REMEMBERED: usize = 50;

/// The photos this phone announced lately: the only ones a PC may ask for.
#[derive(Debug, Default)]
pub(crate) struct Recent(std::sync::Mutex<VecDeque<String>>);

impl Recent {
    fn add(&self, id: &str) {
        let mut ids = self.0.lock().unwrap_or_else(|e| e.into_inner());
        ids.retain(|known| known != id);
        ids.push_back(id.to_owned());
        while ids.len() > REMEMBERED {
            ids.pop_front();
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|known| known == id)
    }
}

impl Shared {
    /// PCs that show photos and are allowed them.
    fn photo_targets(&self) -> Vec<Arc<Session>> {
        self.live_sessions()
            .into_iter()
            .filter(|s| {
                self.is_desktop(&s.peer)
                    && self.toggle_on(&s.peer, TOGGLE)
                    && self
                        .store
                        .get_peer(&s.peer)
                        .ok()
                        .flatten()
                        .is_some_and(|p| p.caps.contains(photos::SHOW))
            })
            .collect()
    }

    pub(crate) async fn photo_taken(&self, photo: PhotoNew) -> Result<()> {
        if !photo.is_valid() {
            return Err(if photo.thumb.len() > photos::MAX_THUMB_BYTES {
                Error::TooLarge
            } else {
                Error::Protocol("invalid photo".into())
            });
        }
        self.photos.add(&photo.id);
        let env = Envelope::new(types::PHOTOS_NEW, &photo)?;
        for session in self.photo_targets() {
            let _ = session.send(env.clone()).await;
        }
        Ok(())
    }
}

/// Asks a phone for a photo it announced; returns the transfer that brings it.
pub(crate) async fn fetch(shared: &Shared, session: &Session, id: String) -> Result<String> {
    if !shared.toggle_on(&session.peer, TOGGLE) {
        return Err(Error::Denied);
    }
    let env = Envelope::new(types::PHOTOS_GET, &PhotoGet { id })?;
    let reply = session.request(env, crate::session::REQUEST_TIMEOUT).await?;
    let PhotoSending { transfer } = reply.expect_body(types::PHOTOS_SENDING)?;
    Ok(transfer)
}

/// Handles `photos.*`. Returns false for other types.
pub(crate) async fn handle(shared: &Arc<Shared>, session: &Arc<Session>, env: &Envelope) -> Result<bool> {
    let peer = session.peer;
    match env.t.as_str() {
        types::PHOTOS_NEW => {
            let photo: PhotoNew = env.body()?;
            let shows = shared.local_capabilities().iter().any(|c| c == photos::SHOW);
            if shows && photo.is_valid() && shared.toggle_on(&peer, TOGGLE) {
                shared.emit(NodeEvent::PhotoAdded { device: peer, photo });
            }
            Ok(true)
        }
        types::PHOTOS_GET => {
            let PhotoGet { id } = env.body()?;
            let reply = match send_photo(shared, peer, &id).await {
                Ok(transfer) => Envelope::new(types::PHOTOS_SENDING, &PhotoSending { transfer })?,
                Err(Error::Denied) => {
                    Envelope::error(ErrorCode::Denied, "photos or files are off for this device")
                }
                Err(Error::NotFound) => Envelope::error(ErrorCode::NotFound, "no such photo"),
                Err(e) => {
                    tracing::warn!(error = %e, "can't send a photo");
                    Envelope::error(ErrorCode::Internal, "can't send it")
                }
            };
            session.send(reply.reply_to(env.id)).await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn send_photo(shared: &Arc<Shared>, peer: DeviceId, id: &str) -> Result<String> {
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !shared.photos.contains(id) {
        return Err(Error::NotFound);
    }
    let platform = shared.platform.clone();
    let owned = id.to_owned();
    let file = tokio::task::spawn_blocking(move || platform.open_photo(&owned))
        .await
        .map_err(|e| Error::Internal(e.to_string()))?
        .map_err(|reason| {
            tracing::info!(reason, "an announced photo can't be opened");
            Error::NotFound
        })?;
    crate::transfer::send(shared, peer, vec![file]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_latest_are_remembered() {
        let recent = Recent::default();
        for i in 0..REMEMBERED + 5 {
            recent.add(&i.to_string());
        }
        assert!(!recent.contains("0") && !recent.contains("4"));
        assert!(recent.contains("5") && recent.contains(&(REMEMBERED + 4).to_string()));
        // Announcing one again moves it to the end.
        recent.add("5");
        recent.add("new");
        assert!(recent.contains("5"));
        assert!(!recent.contains("6"));
    }
}
