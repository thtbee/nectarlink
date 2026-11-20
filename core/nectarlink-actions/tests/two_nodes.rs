// SPDX-License-Identifier: MPL-2.0
//! The phone actions end to end: a real phone node and PC node in one
//! process, paired, with the gateway on the PC and a fake host that approves.
//! Photos, phone storage and quick settings go over the real protocol.

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use nectarlink_actions::{
    Access, ActionHost, Caller, Code, ConfirmRequest, Decision, Gateway, Grant, Scope, Store, Switches,
    host::BoxFuture,
};
use nectarlink_core::{
    DeviceId, DeviceInfo, DeviceKind, FolderStorage, LinkState, MediaPlayer, Node, NodeConfig, NodeEvent,
    Notification, PhoneToggleValue, PhoneToggles, PhotoAlbum, PhotoItem, PhotoThumb, PlainKeyProtector,
    Platform, StorageEntry, StorageError, StorageReadFile, StorageWriteDone,
};
use serde_json::{Value, json};
use tempfile::TempDir;

/// A phone with two photos and a folder of files.
#[derive(Debug)]
struct Phone {
    storage: FolderStorage,
    set: Mutex<Vec<(String, PhoneToggleValue)>>,
    deleted: Mutex<Vec<String>>,
}

impl Platform for Phone {
    fn photo_albums(&self) -> Result<Vec<PhotoAlbum>, String> {
        Ok(vec![PhotoAlbum { id: "cam".into(), name: "Camera".into(), count: 2, cover: None }])
    }
    fn photo_list(
        &self,
        _album: Option<&str>,
        _before: Option<(i64, &str)>,
        _limit: u32,
    ) -> Result<Vec<PhotoItem>, String> {
        let item = |id: &str, date| PhotoItem {
            id: id.into(),
            name: format!("{id}.jpg"),
            date,
            size: 1000,
            width: 4,
            height: 3,
            duration: None,
            album: Some("cam".into()),
        };
        Ok(vec![item("p2", 2_000), item("p1", 1_000)])
    }
    fn photo_thumbs(&self, ids: &[String]) -> Result<Vec<PhotoThumb>, String> {
        Ok(ids.iter().map(|id| PhotoThumb { id: id.clone(), data: vec![0xFF, 0xD8, 0xFF, 0xE0] }).collect())
    }
    fn photo_delete(&self, ids: &[String]) -> Result<u32, String> {
        self.deleted.lock().unwrap().extend(ids.iter().cloned());
        Ok(ids.len() as u32)
    }
    fn set_phone_toggle(&self, id: &str, value: &PhoneToggleValue) -> Result<(), String> {
        self.set.lock().unwrap().push((id.to_owned(), value.clone()));
        Ok(())
    }
    fn storage_list(&self, path: &str) -> Result<Vec<StorageEntry>, StorageError> {
        self.storage.list(path)
    }
    fn storage_open_read(&self, path: &str) -> Result<StorageReadFile, StorageError> {
        self.storage.open_read(path)
    }
    fn storage_write(
        &self,
        path: &str,
        staged: &std::path::Path,
        modified: Option<i64>,
    ) -> Result<StorageWriteDone, StorageError> {
        self.storage.write(path, staged, modified)
    }
    fn storage_mkdir(&self, path: &str) -> Result<(), StorageError> {
        self.storage.mkdir(path)
    }
    fn storage_rename(&self, from: &str, to: &str) -> Result<(), StorageError> {
        self.storage.rename(from, to)
    }
    fn storage_delete(&self, path: &str, confirmed: bool) -> Result<(), StorageError> {
        self.storage.delete(path, confirmed)
    }
}

/// Approves everything and saves into a temp folder.
#[derive(Debug)]
struct Host {
    save: PathBuf,
}

impl ActionHost for Host {
    fn confirm(&self, _request: ConfirmRequest, _expires_s: u64) -> BoxFuture<Decision> {
        Box::pin(async { Decision::Allow })
    }
    fn notifications(&self) -> Vec<(DeviceId, Notification)> {
        Vec::new()
    }
    fn media_players(&self) -> Vec<(Option<DeviceId>, MediaPlayer)> {
        Vec::new()
    }
    fn pc_clipboard_read(&self) -> Result<String, String> {
        Ok(String::new())
    }
    fn pc_clipboard_write(&self, _text: &str) -> Result<(), String> {
        Ok(())
    }
    fn pc_lock(&self) -> Result<(), String> {
        Ok(())
    }
    fn transfers(&self) -> Vec<Value> {
        Vec::new()
    }
    fn calls(&self) -> Vec<(DeviceId, Value)> {
        Vec::new()
    }
    fn save_dir(&self) -> Option<PathBuf> {
        Some(self.save.clone())
    }
}

fn config(dir: &TempDir, name: &str, kind: DeviceKind) -> NodeConfig {
    let mut cfg = NodeConfig::new(
        dir.path().to_path_buf(),
        DeviceInfo {
            name: name.into(),
            kind,
            os: if kind == DeviceKind::Phone { "android" } else { "windows" }.into(),
            os_ver: "14".into(),
            model: None,
            accent: None,
            screen: None,
            install_id: None,
        },
        "0.1.0-agents",
    );
    cfg.lan_discovery = false;
    cfg.away_mode = false;
    cfg.localsend_port = 0;
    cfg.key_protector = Some(Arc::new(PlainKeyProtector));
    cfg
}

async fn online(events: &mut tokio::sync::broadcast::Receiver<NodeEvent>, target: DeviceId) {
    loop {
        match events.recv().await {
            Ok(NodeEvent::LinkChanged { device, link: LinkState::Online { .. } }) if device == target => {
                return;
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(e) => panic!("events closed: {e}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn photos_storage_and_quick_settings_through_the_gateway() {
    let (phone_dir, pc_dir, files, save) =
        (TempDir::new().unwrap(), TempDir::new().unwrap(), TempDir::new().unwrap(), TempDir::new().unwrap());
    std::fs::create_dir_all(files.path().join("Download")).unwrap();
    std::fs::write(files.path().join("Download").join("notes.txt"), "Buy oats. Your code is 482913.")
        .unwrap();
    std::fs::write(files.path().join("Download").join("cat.png"), [0x89, b'P', b'N', b'G']).unwrap();
    let platform = Arc::new(Phone {
        storage: FolderStorage::new(files.path().to_path_buf())
            .with_trash_dir(phone_dir.path().join("trash")),
        set: Mutex::new(Vec::new()),
        deleted: Mutex::new(Vec::new()),
    });
    let mut phone_cfg = config(&phone_dir, "Test Phone", DeviceKind::Phone);
    phone_cfg.capabilities.push(nectarlink_core::STORAGE_READ.into());
    phone_cfg.capabilities.push(nectarlink_core::STORAGE_WRITE.into());
    phone_cfg.capabilities.push(nectarlink_core::PHOTOS_READ.into());
    for t in [nectarlink_core::TOGGLES_READ, nectarlink_core::TOGGLES_DND, nectarlink_core::TOGGLES_RINGER] {
        phone_cfg.capabilities.push(t.into());
    }
    let phone = Node::start(phone_cfg, platform.clone()).await.unwrap();
    let mut pc_cfg = config(&pc_dir, "Test PC", DeviceKind::Desktop);
    pc_cfg.capabilities.push(nectarlink_core::TOGGLES_SHOW.into());
    let pc = Node::start(pc_cfg, Arc::new(nectarlink_core::NoopPlatform)).await.unwrap();
    let (mut phone_events, mut pc_events) = (phone.events(), pc.events());
    let uri = pc.pairing_start_qr().await.unwrap();
    phone.pairing_join(&uri.to_uri()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        tokio::join!(online(&mut phone_events, pc.device_id()), online(&mut pc_events, phone.device_id()));
    })
    .await
    .expect("paired and connected");
    // Phone storage is off for a PC until the user turns it on, on both sides.
    pc.set_device_toggle(phone.device_id(), "storage", true).unwrap();
    phone.set_device_toggle(pc.device_id(), "storage", true).unwrap();
    phone
        .toggles_changed(PhoneToggles {
            dnd: false,
            ringer: "ring".into(),
            flashlight: Some(false),
            volume: 40,
            brightness: 60,
            wifi: true,
            bluetooth: false,
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        while pc.phone_toggles(phone.device_id()).is_none() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the PC got the phone's quick settings");

    let store = Arc::new(Store::in_memory().unwrap());
    let grants: BTreeSet<Grant> = [Scope::Devices, Scope::Photos, Scope::Storage, Scope::PhoneSettings]
        .into_iter()
        .map(|s| Grant::new(s, Access::Write))
        .collect();
    let (_client, token) = store.create_client("e2e", "E2E", "cli", &grants).unwrap();
    let gw = Gateway::new(Arc::new(Host { save: save.path().to_path_buf() }), store);
    gw.set_switches(Switches { enabled: true, read_only: false });
    gw.set_node(Some(Arc::new(pc.clone())));
    let caller = Caller { token, channel: "cli".into() };
    let call = |action: &'static str, args: Value| {
        let (gw, caller) = (&gw, &caller);
        async move { gw.invoke(caller, action, args).await }
    };

    // Photos.
    let albums = call("photos.albums", json!({})).await.unwrap();
    assert_eq!(albums["albums"][0]["name"], "Camera");
    let list = call("photos.list", json!({ "limit": 10 })).await.unwrap();
    assert_eq!(list["items"][0]["id"], "p2");
    let thumbs = call("photos.thumbnails", json!({ "ids": ["p1", "p2"] })).await.unwrap();
    assert_eq!(thumbs["_images"].as_array().unwrap().len(), 2);
    assert_eq!(thumbs["_images"][0]["mime"], "image/jpeg");
    assert_eq!(thumbs["_images"][0]["data"], "/9j/4A==");
    let gone = call("photos.delete", json!({ "ids": ["p1"] })).await.unwrap();
    assert_eq!(gone["moved_to_trash"], 1);
    assert_eq!(*platform.deleted.lock().unwrap(), ["p1"]);

    // Phone storage.
    let listed = call("storage.list", json!({ "path": "Download" })).await.unwrap();
    let names: Vec<_> = listed["entries"].as_array().unwrap().iter().map(|e| e["name"].clone()).collect();
    assert!(names.contains(&json!("notes.txt")) && names.contains(&json!("cat.png")), "{names:?}");
    let text = call("storage.read", json!({ "path": "Download/notes.txt" })).await.unwrap();
    let body = text["text"].as_str().unwrap();
    assert!(body.starts_with("Buy oats.") && !body.contains("482913"), "{body}");
    let picture = call("storage.read", json!({ "path": "Download/cat.png" })).await.unwrap();
    assert_eq!(picture["_images"][0]["mime"], "image/png");
    let saved = call("storage.save", json!({ "path": "Download/notes.txt" })).await.unwrap();
    let saved_path = PathBuf::from(saved["saved"].as_str().unwrap());
    assert_eq!(std::fs::read_to_string(&saved_path).unwrap(), "Buy oats. Your code is 482913.");
    assert!(saved_path.starts_with(save.path()));
    let again = call("storage.save", json!({ "path": "Download/notes.txt" })).await.unwrap();
    assert_ne!(again["saved"], saved["saved"], "a second copy gets its own name");
    call("storage.mkdir", json!({ "path": "Download/Agent" })).await.unwrap();
    call(
        "storage.upload",
        json!({ "source": saved_path.to_string_lossy(), "path": "Download/Agent/copy.txt" }),
    )
    .await
    .unwrap();
    assert!(files.path().join("Download").join("Agent").join("copy.txt").is_file());
    call("storage.rename", json!({ "from": "Download/Agent/copy.txt", "to": "Download/Agent/moved.txt" }))
        .await
        .unwrap();
    assert!(files.path().join("Download").join("Agent").join("moved.txt").is_file());
    call("storage.delete", json!({ "path": "Download/Agent" })).await.unwrap();
    assert!(!files.path().join("Download").join("Agent").exists());
    let escape = call("storage.list", json!({ "path": "../.." })).await.unwrap_err();
    assert_eq!(escape.code, Code::InvalidArguments);

    // Quick settings.
    let settings = call("phone.settings", json!({})).await.unwrap();
    assert_eq!(settings["volume"], 40);
    call("phone.settings_set", json!({ "setting": "dnd", "on": true })).await.unwrap();
    call("phone.settings_set", json!({ "setting": "ringer", "mode": "vibrate" })).await.unwrap();
    let bad = call("phone.settings_set", json!({ "setting": "volume" })).await.unwrap_err();
    assert_eq!(bad.code, Code::InvalidArguments);
    let set = platform.set.lock().unwrap().clone();
    assert_eq!(set[0], ("dnd".into(), PhoneToggleValue::Bool(true)));
    assert_eq!(set[1], ("ringer".into(), PhoneToggleValue::Mode("vibrate".into())));

    // With the device's own storage permission off, the gateway says no first.
    pc.set_device_toggle(phone.device_id(), "storage", false).unwrap();
    let off = call("storage.list", json!({ "path": "" })).await.unwrap_err();
    assert_eq!(off.code, Code::Denied);

    pc.shutdown().await;
    phone.shutdown().await;
}
