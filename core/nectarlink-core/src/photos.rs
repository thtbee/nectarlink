// SPDX-License-Identifier: MPL-2.0
//! Photos and gallery (docs/protocol/photos.md): a phone announces each new
//! photo or screenshot to PCs with a preview, lists its albums and items,
//! serves thumbnails in batches, and sends full photos or videos (as a files
//! transfer) when a PC asks for them.

use std::sync::Arc;

use nectarlink_protocol::{
    DeviceId, Envelope, ErrorCode,
    messages::{
        PhotoAlbum, PhotoAlbumsGet, PhotoAlbumsList, PhotoGet, PhotoItem, PhotoItems, PhotoListGet, PhotoNew,
        PhotoSending, PhotoThumb, PhotoThumbsGet, PhotoThumbsList, PhotosChanged, is_valid_photo_id, photos,
        types,
    },
};

use crate::{Error, Result, events::NodeEvent, node::Shared, session::Session, transfer::safe_file_name};

/// The device toggle that allows photos and gallery access for a device.
pub(crate) const TOGGLE: &str = "photos";

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
        let env = Envelope::new(types::PHOTOS_NEW, &photo)?;
        for session in self.photo_targets() {
            let _ = session.send(env.clone()).await;
        }
        Ok(())
    }

    pub(crate) async fn photos_changed(&self) {
        let Ok(env) = Envelope::new(types::PHOTOS_CHANGED, &PhotosChanged {}) else { return };
        for session in self.photo_targets() {
            let _ = session.send(env.clone()).await;
        }
    }
}

// ---- A PC asking ----

fn allowed(shared: &Shared, peer: &DeviceId) -> Result<()> {
    if shared.toggle_on(peer, TOGGLE) { Ok(()) } else { Err(Error::Denied) }
}

/// Lists a phone's photo and video albums (`photos.albums`).
pub(crate) async fn albums(shared: &Shared, session: &Session) -> Result<Vec<PhotoAlbum>> {
    allowed(shared, &session.peer)?;
    let env = Envelope::new(types::PHOTOS_ALBUMS, &PhotoAlbumsGet {})?;
    let PhotoAlbumsList { albums } = session
        .request(env, crate::session::REQUEST_TIMEOUT)
        .await?
        .expect_body(types::PHOTOS_ALBUMS_LIST)?;
    Ok(albums)
}

/// Lists a phone's photos and videos (`photos.list`), newest first.
pub(crate) async fn list(
    shared: &Shared,
    session: &Session,
    album: Option<String>,
    before: Option<(i64, String)>,
    limit: u32,
) -> Result<Vec<PhotoItem>> {
    allowed(shared, &session.peer)?;
    let album = album.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty());
    let (before, before_id) = before.map_or((None, None), |(date, id)| (Some(date), Some(id)));
    let env = Envelope::new(
        types::PHOTOS_LIST,
        &PhotoListGet { album, before, before_id, limit: limit.clamp(1, photos::MAX_PAGE) },
    )?;
    let PhotoItems { items } =
        session.request(env, crate::session::REQUEST_TIMEOUT).await?.expect_body(types::PHOTOS_ITEMS)?;
    Ok(items)
}

/// Fetches small JPEG thumbnails for a batch of item IDs (`photos.thumbs`).
pub(crate) async fn thumbs(
    shared: &Shared,
    session: &Session,
    mut ids: Vec<String>,
) -> Result<Vec<PhotoThumb>> {
    allowed(shared, &session.peer)?;
    ids.retain(|id| is_valid_photo_id(id));
    ids.truncate(photos::MAX_THUMB_BATCH);
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let env = Envelope::new(types::PHOTOS_THUMBS, &PhotoThumbsGet { ids })?;
    let PhotoThumbsList { thumbs } = session
        .request(env, crate::session::REQUEST_TIMEOUT)
        .await?
        .expect_body(types::PHOTOS_THUMBS_LIST)?;
    Ok(thumbs)
}

/// Asks a phone for a photo or video; returns the transfer that brings it.
pub(crate) async fn fetch(shared: &Shared, session: &Session, id: String) -> Result<String> {
    fetch_many(shared, session, vec![id]).await
}

/// Asks a phone for one or more photos or videos in a single transfer.
pub(crate) async fn fetch_many(shared: &Shared, session: &Session, mut ids: Vec<String>) -> Result<String> {
    allowed(shared, &session.peer)?;
    ids.retain(|id| is_valid_photo_id(id));
    ids.truncate(photos::MAX_GET_ITEMS);
    if ids.is_empty() {
        return Err(Error::Protocol("no photo IDs requested".into()));
    }
    let id = if ids.len() == 1 { ids[0].clone() } else { String::new() };
    let env = Envelope::new(types::PHOTOS_GET, &PhotoGet { id, ids })?;
    let reply = session.request(env, crate::session::REQUEST_TIMEOUT).await?;
    let PhotoSending { transfer } = reply.expect_body(types::PHOTOS_SENDING)?;
    Ok(transfer)
}

// ---- The phone answering ----

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
        types::PHOTOS_CHANGED => {
            let PhotosChanged {} = env.body()?;
            let shows = shared.local_capabilities().iter().any(|c| c == photos::SHOW);
            if shows && shared.toggle_on(&peer, TOGGLE) {
                shared.emit(NodeEvent::PhotosChanged { device: peer });
            }
            Ok(true)
        }
        types::PHOTOS_ALBUMS | types::PHOTOS_LIST | types::PHOTOS_THUMBS => {
            let reply = answer_query(shared, peer, env).await.unwrap_or_else(|e| {
                tracing::debug!(error = %e, "bad photos request");
                Envelope::error(ErrorCode::BadMessage, "invalid request")
            });
            session.send(reply.reply_to(env.id)).await?;
            Ok(true)
        }
        types::PHOTOS_GET => {
            let get: PhotoGet = env.body()?;
            let reply = match send_photos(shared, peer, get.requested_ids()).await {
                Ok(transfer) => Envelope::new(types::PHOTOS_SENDING, &PhotoSending { transfer })?,
                Err(Error::Denied) => {
                    Envelope::error(ErrorCode::Denied, "photos or files are off for this device")
                }
                Err(Error::Unsupported) => {
                    Envelope::error(ErrorCode::Unsupported, "this phone doesn't share photos")
                }
                Err(Error::NotFound) => Envelope::error(ErrorCode::NotFound, "no such photo"),
                Err(Error::Protocol(_)) => Envelope::error(ErrorCode::BadMessage, "invalid photo request"),
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

async fn answer_query(shared: &Arc<Shared>, peer: DeviceId, env: &Envelope) -> Result<Envelope> {
    if !shared.toggle_on(&peer, TOGGLE) {
        return Ok(Envelope::error(ErrorCode::Denied, "photos are off for this device"));
    }
    if !shared.local_capabilities().iter().any(|c| c == photos::READ) {
        return Ok(Envelope::error(ErrorCode::Unsupported, "this phone doesn't share photos"));
    }
    let platform = shared.platform.clone();
    let result = match env.t.as_str() {
        types::PHOTOS_ALBUMS => {
            let PhotoAlbumsGet {} = env.body()?;
            tokio::task::spawn_blocking(move || {
                let albums = sanitize_albums(platform.photo_albums()?);
                crate::sms::fit(albums, |albums| {
                    Envelope::new(types::PHOTOS_ALBUMS_LIST, &PhotoAlbumsList { albums })
                })
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
        types::PHOTOS_LIST => {
            let PhotoListGet { album, before, before_id, limit } = env.body()?;
            let album = album.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty());
            let limit = limit.clamp(1, photos::MAX_PAGE);
            // An older PC sends only `before`: everything from that date on
            // was on its pages.
            let before_id = before_id.filter(|id| is_valid_photo_id(id)).unwrap_or_default();
            tokio::task::spawn_blocking(move || {
                let before = before.map(|date| (date, before_id.as_str()));
                let items = sanitize_items(platform.photo_list(album.as_deref(), before, limit)?);
                crate::sms::fit(items, |items| Envelope::new(types::PHOTOS_ITEMS, &PhotoItems { items }))
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
        types::PHOTOS_THUMBS => {
            let PhotoThumbsGet { mut ids } = env.body()?;
            ids.retain(|id| is_valid_photo_id(id));
            ids.truncate(photos::MAX_THUMB_BATCH);
            if ids.is_empty() {
                return Envelope::new(types::PHOTOS_THUMBS_LIST, &PhotoThumbsList { thumbs: Vec::new() })
                    .map_err(Into::into);
            }
            tokio::task::spawn_blocking(move || {
                let thumbs = sanitize_thumbs(platform.photo_thumbs(&ids)?);
                crate::sms::fit(thumbs, |thumbs| {
                    Envelope::new(types::PHOTOS_THUMBS_LIST, &PhotoThumbsList { thumbs })
                })
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
        _ => unreachable!(),
    };
    Ok(result.unwrap_or_else(|reason| {
        tracing::warn!(reason, "a photos request failed");
        Envelope::error(ErrorCode::Internal, "the phone couldn't read photos")
    }))
}

fn sanitize_albums(albums: Vec<PhotoAlbum>) -> Vec<PhotoAlbum> {
    albums
        .into_iter()
        .filter_map(|mut a| {
            let id = a.id.trim().to_owned();
            let name = a.name.trim().to_owned();
            if id.is_empty() || name.is_empty() || a.count == 0 {
                return None;
            }
            a.id = id;
            a.name = name;
            a.cover = a.cover.filter(|c| is_valid_photo_id(c));
            Some(a)
        })
        .collect()
}

fn sanitize_items(items: Vec<PhotoItem>) -> Vec<PhotoItem> {
    items
        .into_iter()
        .filter_map(|mut item| {
            if !is_valid_photo_id(&item.id) {
                return None;
            }
            item.name = safe_file_name(&item.name);
            item.album = item.album.map(|a| a.trim().to_owned()).filter(|a| !a.is_empty());
            Some(item)
        })
        .collect()
}

fn sanitize_thumbs(thumbs: Vec<PhotoThumb>) -> Vec<PhotoThumb> {
    thumbs
        .into_iter()
        .filter(|t| is_valid_photo_id(&t.id) && !t.data.is_empty() && t.data.len() <= photos::MAX_THUMB_BYTES)
        .collect()
}

async fn send_photos(shared: &Arc<Shared>, peer: DeviceId, mut ids: Vec<String>) -> Result<String> {
    if !shared.toggle_on(&peer, TOGGLE) {
        return Err(Error::Denied);
    }
    if !shared.local_capabilities().iter().any(|c| c == photos::READ) {
        return Err(Error::Unsupported);
    }
    ids.retain(|id| is_valid_photo_id(id));
    ids.truncate(photos::MAX_GET_ITEMS);
    if ids.is_empty() {
        return Err(Error::Protocol("no valid photo IDs".into()));
    }
    let platform = shared.platform.clone();
    let files = tokio::task::spawn_blocking(move || -> Result<Vec<crate::OutgoingFile>> {
        let mut out = Vec::with_capacity(ids.len());
        for id in &ids {
            match platform.open_photo(id) {
                Ok(file) => out.push(file),
                Err(reason) => {
                    tracing::info!(reason, "a requested photo can't be opened");
                    return Err(Error::NotFound);
                }
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| Error::Internal(e.to_string()))??;
    crate::transfer::send(shared, peer, files).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_cleans_albums_items_and_thumbs() {
        let albums = sanitize_albums(vec![
            PhotoAlbum { id: " b1 ".into(), name: " Camera ".into(), count: 3, cover: Some("p1".into()) },
            PhotoAlbum { id: "b2".into(), name: "Empty".into(), count: 0, cover: None },
            PhotoAlbum { id: "b3".into(), name: "Bad cover".into(), count: 1, cover: Some("bad id".into()) },
        ]);
        assert_eq!(albums.len(), 2);
        assert_eq!((albums[0].id.as_str(), albums[0].name.as_str()), ("b1", "Camera"));
        assert_eq!(albums[1].cover, None);

        let items = sanitize_items(vec![
            PhotoItem {
                id: "p1".into(),
                name: "../IMG_001.jpg".into(),
                date: 100,
                size: 10,
                width: 800,
                height: 600,
                duration: None,
                album: Some(" b1 ".into()),
            },
            PhotoItem {
                id: "bad id".into(),
                name: "x.jpg".into(),
                date: 90,
                size: 10,
                width: 0,
                height: 0,
                duration: None,
                album: None,
            },
        ]);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "IMG_001.jpg");
        assert_eq!(items[0].album.as_deref(), Some("b1"));

        let thumbs = sanitize_thumbs(vec![
            PhotoThumb { id: "p1".into(), data: vec![0xff, 0xd8] },
            PhotoThumb { id: "p2".into(), data: Vec::new() },
            PhotoThumb { id: "p3".into(), data: vec![0; photos::MAX_THUMB_BYTES + 1] },
        ]);
        assert_eq!(thumbs.len(), 1);
        assert_eq!(thumbs[0].id, "p1");
    }
}
