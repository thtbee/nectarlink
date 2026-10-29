// SPDX-License-Identifier: MPL-2.0
//! Scale and performance checks, and the relayed wake and PC audio round trip.
//!
//! The scale checks move gigabytes and hold timing limits, so they depend on
//! the machine and are opt-in:
//! `cargo test -p nectarlink-core --release --test scale_tests -- --ignored`.

use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use nectarlink_core::{
    ClipboardItemKind, DeviceId, DeviceInfo, DeviceKind, FileSource, LinkState, Node, NodeConfig, NodeEvent,
    NoopPlatform, Notification, NotificationAction, NotificationChatMessage, NotificationConversation,
    OutgoingFile, PlainKeyProtector, TimelineKind, TimelineQuery, TimelineRetention, TransferState,
    clipboard_history::ClipboardHistoryStore,
};
use tempfile::TempDir;

fn make_config(dir: &TempDir, name: &str, kind: DeviceKind) -> NodeConfig {
    let mut cfg = NodeConfig::new(
        dir.path().to_path_buf(),
        DeviceInfo {
            name: name.into(),
            kind,
            os: if kind == DeviceKind::Phone { "android" } else { "windows" }.into(),
            os_ver: "11".into(),
            model: None,
            accent: None,
            screen: None,
            install_id: None,
        },
        "0.1.0-scale",
    );
    cfg.lan_discovery = false;
    cfg.away_mode = false;
    cfg.localsend_port = 0;
    cfg.key_protector = Some(Arc::new(PlainKeyProtector));
    cfg
}

async fn start_paired_nodes() -> (TempDir, TempDir, Node, Node) {
    let phone_dir = TempDir::new().expect("phone temp dir");
    let pc_dir = TempDir::new().expect("pc temp dir");
    let phone =
        Node::start(make_config(&phone_dir, "Scale Phone", DeviceKind::Phone), Arc::new(NoopPlatform))
            .await
            .expect("start phone node");
    let pc = Node::start(make_config(&pc_dir, "Scale PC", DeviceKind::Desktop), Arc::new(NoopPlatform))
        .await
        .expect("start pc node");

    let mut phone_events = phone.events();
    let mut pc_events = pc.events();

    let uri = pc.pairing_start_qr().await.expect("start QR pairing");
    phone.pairing_join(&uri.to_uri()).await.expect("join QR pairing");

    let wait_connected = async |events: &mut tokio::sync::broadcast::Receiver<NodeEvent>,
                                target: DeviceId| {
        loop {
            match events.recv().await {
                Ok(NodeEvent::LinkChanged { device, link: LinkState::Online { .. } }) if device == target => {
                    break;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("event stream closed waiting for LinkState::Online: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            wait_connected(&mut phone_events, pc.device_id()),
            wait_connected(&mut pc_events, phone.device_id()),
        );
    })
    .await
    .expect("nodes connect within 15s");

    (phone_dir, pc_dir, phone, pc)
}

#[tokio::test(flavor = "multi_thread")]
async fn relayed_wake_and_pc_audio_pcm_roundtrip() {
    let (_phone_dir, _pc_dir, phone, pc) = start_paired_nodes().await;
    let mut phone_events = phone.events();

    // 1. Relayed Wake-on-LAN (`wake.relay`) from phone through PC: only for a
    // paired PC's own address (two_devices.rs covers relaying one), so an
    // address nobody gave is refused.
    let relayed = phone
        .wake_via_relay(
            &pc.device_id(),
            [0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc],
            Some(std::net::Ipv4Addr::LOCALHOST),
        )
        .await;
    assert!(matches!(relayed, Err(nectarlink_core::Error::Denied)), "{relayed:?}");

    // 2. Low-latency PC-to-Phone PCM audio stream (`audio.pcm`).
    let pcm_payload: Vec<u8> = (0..1920).map(|i| (i % 251) as u8).collect();
    pc.send_pc_audio_pcm(&phone.device_id(), 48_000, 2, pcm_payload.clone())
        .await
        .expect("send_pc_audio_pcm succeeds");

    let received = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(NodeEvent::PcAudioFrame { device, sample_rate, channels, pcm }) =
                phone_events.recv().await
            {
                return (device, sample_rate, channels, pcm);
            }
        }
    })
    .await
    .expect("phone receives PcAudioFrame within 5s");

    assert_eq!(received.0, pc.device_id());
    assert_eq!(received.1, 48_000);
    assert_eq!(received.2, 2);
    assert_eq!(received.3, pcm_payload);

    phone.shutdown().await;
    pc.shutdown().await;
}

#[ignore = "scale benchmark with timing limits; run with --ignored"]
#[tokio::test(flavor = "multi_thread")]
async fn scale_10000_notifications_in_history_and_search() {
    let dir = TempDir::new().expect("temp dir");
    let node = Node::start(make_config(&dir, "Scale Phone", DeviceKind::Phone), Arc::new(NoopPlatform))
        .await
        .expect("start node");

    let apps = [
        ("org.thoughtcrime.securesms", "Signal"),
        ("com.whatsapp", "WhatsApp"),
        ("com.Slack", "Slack"),
        ("com.google.android.gm", "Gmail"),
        ("org.telegram.messenger", "Telegram"),
    ];

    let mut items = Vec::with_capacity(10_000);
    for i in 0..10_000usize {
        let (pkg, app_name) = apps[i % apps.len()];
        items.push(Notification {
            key: format!("{pkg}|notif|{i:05}"),
            app: pkg.into(),
            app_name: app_name.into(),
            title: Some(format!("{app_name} Contact #{}", i % 250)),
            text: Some(if i == 4242 {
                "Urgent: please approve invoice #4242 before EOD".into()
            } else {
                format!("Notification body message number {i} with searchable content")
            }),
            sub: Some(format!("workspace-{}", i % 10)),
            when: 1_710_000_000_000i64 + (i as i64) * 1000,
            silent: i % 50 == 0,
            actions: Vec::new(),
            icon: None,
            image: None,
            live: None,
            conversation: None,
        });
    }

    let insert_start = Instant::now();
    node.notifications_reset(items).await;
    let insert_elapsed = insert_start.elapsed();

    let query_start = Instant::now();
    let recent = node.notification_history(None, 200);
    let list_elapsed = query_start.elapsed();
    assert_eq!(recent.len(), 200);
    assert_eq!(recent[0].key, "org.telegram.messenger|notif|09999");

    let search_start = Instant::now();
    let needle_hits = node.notification_history(Some("invoice #4242"), 200);
    let app_hits = node.notification_history(Some("Signal"), 200);
    let search_elapsed = search_start.elapsed();
    let total_query_elapsed = list_elapsed + search_elapsed;

    assert_eq!(needle_hits.len(), 1);
    assert_eq!(needle_hits[0].key, "com.Slack|notif|04242");
    assert_eq!(app_hits.len(), 200);

    println!(
        "[SCALE] 10,000 notifications: ingest={:?}, list_200={:?}, search={:?}, total_list_and_search={:?}",
        insert_elapsed, list_elapsed, search_elapsed, total_query_elapsed
    );
    assert!(
        total_query_elapsed < Duration::from_millis(100),
        "10,000 notification list + search took {:?}, expected < 100ms",
        total_query_elapsed
    );

    node.shutdown().await;
}

#[ignore = "scale benchmark with timing limits; run with --ignored"]
#[tokio::test(flavor = "multi_thread")]
async fn scale_5000_timeline_entries_and_200_chat_threads() {
    let dir = TempDir::new().expect("temp dir");
    let node = Node::start(make_config(&dir, "Scale PC", DeviceKind::Desktop), Arc::new(NoopPlatform))
        .await
        .expect("start node");

    let peer = DeviceId([0x42; 32]);
    node.set_timeline_retention(TimelineRetention { max_days: 90, max_entries: 5_000 })
        .expect("set retention");

    let kinds = [
        TimelineKind::File,
        TimelineKind::Clip,
        TimelineKind::Link,
        TimelineKind::Photo,
        TimelineKind::Recording,
        TimelineKind::Session,
    ];

    let tl_insert_start = Instant::now();
    for i in 0..5_000usize {
        let kind = kinds[i % kinds.len()];
        let title =
            if i == 4242 { "quarterly_report_4242.pdf".to_owned() } else { format!("item_{i:04}.dat") };
        node.record_timeline(
            kind,
            peer,
            i % 2 == 0,
            title,
            format!("detail for entry {i}"),
            format!("C:\\temp\\item_{i:04}.dat"),
            ((i + 1) * 1024) as u64,
            (i % 120) as u64,
            Some(format!("ref-{i:04}")),
        )
        .expect("record timeline row");
    }
    let tl_insert_elapsed = tl_insert_start.elapsed();

    let chat_insert_start = Instant::now();
    for t in 0..200usize {
        let mut messages = Vec::with_capacity(8);
        for m in 0..8usize {
            messages.push(NotificationChatMessage {
                sender: Some(format!("Sender {t}")),
                text: format!("Thread {t} message {m}: hello from scale test"),
                time: 1_710_000_000_000i64 + (t as i64) * 10_000 + (m as i64) * 100,
                self_sent: m % 3 == 0,
                avatar: None,
            });
        }
        let notif = Notification {
            key: format!("com.whatsapp|thread|{t:03}"),
            app: "com.whatsapp".into(),
            app_name: "WhatsApp".into(),
            title: Some(format!("Conversation Thread {t:03}")),
            text: Some(format!("Thread {t} latest message")),
            sub: None,
            when: 1_710_000_000_000i64 + (t as i64) * 10_000 + 700,
            silent: false,
            actions: vec![NotificationAction { id: "reply".into(), title: "Reply".into(), reply: true }],
            icon: None,
            image: None,
            live: None,
            conversation: Some(NotificationConversation {
                v: 1,
                title: format!("Conversation Thread {t:03}"),
                group: t % 4 == 0,
                avatar: None,
                messages,
            }),
        };
        node.upsert_chat_notification(peer, &notif, false).expect("upsert chat thread");
    }
    let chat_insert_elapsed = chat_insert_start.elapsed();

    let page_start = Instant::now();
    let page = node
        .timeline_page(&TimelineQuery { kind: None, device: None, search: None, offset: 0, limit: 100 })
        .expect("timeline page query");
    let page_elapsed = page_start.elapsed();
    assert_eq!(page.total, 5_000);
    assert_eq!(page.entries.len(), 100);
    assert!(page.has_more);

    let search_start = Instant::now();
    let search_page = node
        .timeline_page(&TimelineQuery {
            kind: None,
            device: None,
            search: Some("quarterly_report_4242".into()),
            offset: 0,
            limit: 50,
        })
        .expect("timeline search query");
    let search_elapsed = search_start.elapsed();
    assert_eq!(search_page.total, 1);
    assert_eq!(search_page.entries[0].title, "quarterly_report_4242.pdf");

    let threads_start = Instant::now();
    let threads = node.chat_threads(peer);
    let threads_elapsed = threads_start.elapsed();
    assert_eq!(threads.len(), 200);

    let msgs_start = Instant::now();
    let msgs = node.chat_messages(peer, &threads[0].thread_id, 50);
    let msgs_elapsed = msgs_start.elapsed();
    assert_eq!(msgs.len(), 8);

    println!(
        "[SCALE] 5,000 timeline + 200 chat threads: tl_insert={:?}, chat_insert={:?}, tl_page={:?}, tl_search={:?}, chat_threads={:?}, chat_msgs={:?}",
        tl_insert_elapsed, chat_insert_elapsed, page_elapsed, search_elapsed, threads_elapsed, msgs_elapsed
    );
    assert!(
        page_elapsed < Duration::from_millis(50),
        "timeline page query took {:?}, expected < 50ms",
        page_elapsed
    );
    assert!(
        search_elapsed < Duration::from_millis(100),
        "timeline search query took {:?}, expected < 100ms",
        search_elapsed
    );
    assert!(
        threads_elapsed < Duration::from_millis(50),
        "chat_threads(200) query took {:?}, expected < 50ms",
        threads_elapsed
    );
    assert!(
        msgs_elapsed < Duration::from_millis(50),
        "chat_messages query took {:?}, expected < 50ms",
        msgs_elapsed
    );

    node.shutdown().await;
}

#[ignore = "scale benchmark with timing limits; run with --ignored"]
#[test]
fn scale_50_large_clips_encrypted_history() {
    let dir = TempDir::new().expect("temp dir");
    let protector = nectarlink_core::default_protector();

    let store = ClipboardHistoryStore::open(dir.path(), protector.clone());
    let write_start = Instant::now();
    for i in 0..50usize {
        // Each clip is 64 KiB of rich text/HTML-like content (total ~3.2 MiB across 50 clips).
        let header = format!("clip-payload-{i:04} <div>Rich HTML &amp; text block #{i}</div> ");
        let body = "A".repeat(65_536 - header.len());
        let text = format!("{header}{body}");
        assert_eq!(text.len(), 65_536);
        let recorded = store.record_text(&text, "Scale Phone", true);
        assert!(recorded, "clip {i} recorded");
    }
    let write_elapsed = write_start.elapsed();

    // Verify the file on disk does not leak plaintext. Encryption comes from
    // the platform's protector (DPAPI on Windows); elsewhere the default
    // protector stores as is (id 0), so there is nothing to check.
    let enc_bytes =
        std::fs::read(dir.path().join("clipboard_history.enc")).expect("encrypted history exists");
    if protector.id() != 0 {
        assert!(
            !enc_bytes.windows(b"clip-payload-0025".len()).any(|w| w == b"clip-payload-0025"),
            "clipboard_history.enc must be encrypted at rest"
        );
    }

    // Measure cold open + AES-256-GCM decryption + listing + substring search across all 50 x 64 KiB clips.
    let read_start = Instant::now();
    let reopened = ClipboardHistoryStore::open(dir.path(), protector);
    let all = reopened.list(None);
    let filtered = reopened.list(Some("clip-payload-0025"));
    let read_elapsed = read_start.elapsed();

    assert_eq!(all.len(), 50);
    assert!(all.iter().all(|e| e.kind == ClipboardItemKind::Text && e.text.len() == 65_536));
    assert_eq!(filtered.len(), 1);
    assert!(filtered[0].text.starts_with("clip-payload-0025"));

    println!(
        "[SCALE] 50 x 64 KiB encrypted clips (3.2 MiB): write_50={:?}, reopen_decrypt_list_search={:?}",
        write_elapsed, read_elapsed
    );
    let limit = if cfg!(debug_assertions) { Duration::from_millis(750) } else { Duration::from_millis(250) };
    assert!(
        read_elapsed < limit,
        "reopen + decrypt + list + search of 50 large clips took {:?}, expected < {:?}",
        read_elapsed,
        limit
    );
}

#[ignore = "scale benchmark with timing limits; run with --ignored"]
#[tokio::test(flavor = "multi_thread")]
async fn scale_4gb_file_transfer_with_pause_resume_interruption_and_blake3() {
    const FOUR_GIB: u64 = 4 * 1024 * 1024 * 1024; // 4,294,967,296 bytes
    const CHUNK_SIZE: usize = 8 * 1024 * 1024; // 8 MiB

    let (phone_dir, pc_dir, phone, pc) = start_paired_nodes().await;
    let mut phone_events = phone.events();
    let mut pc_events = pc.events();

    let src_path = phone_dir.path().join("scale_4gib_payload.bin");
    let gen_start = Instant::now();
    let expected_blake3 = {
        let mut f = std::fs::File::create(&src_path).expect("create 4 GiB source file");
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        let chunks = (FOUR_GIB / CHUNK_SIZE as u64) as usize;
        for c in 0..chunks {
            // Fill deterministic non-zero pattern per 8 MiB chunk.
            let seed = (c as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            for (idx, slot) in buf.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                let val = seed.wrapping_add(idx as u64).to_le_bytes();
                slot.copy_from_slice(&val);
            }
            f.write_all(&buf).expect("write 8 MiB chunk");
            hasher.update(&buf);
        }
        f.flush().expect("flush 4 GiB file");
        hasher.finalize().to_hex().to_string()
    };
    let gen_elapsed = gen_start.elapsed();
    assert_eq!(std::fs::metadata(&src_path).expect("src metadata").len(), FOUR_GIB);

    let transfer_start = Instant::now();
    let outgoing = OutgoingFile {
        name: "scale_4gib_payload.bin".into(),
        folder: None,
        source: FileSource::Path(src_path.clone()),
    };
    let transfer_id = phone.send_files(pc.device_id(), vec![outgoing]).await.expect("start 4 GiB transfer");

    // 1. Wait until at least 64 MiB has transferred, then pause from sender (`phone`).
    let mut first_pause_done = 0u64;
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match phone_events.recv().await {
                Ok(NodeEvent::Transfer(t))
                    if t.id == transfer_id
                        && t.state == TransferState::Running
                        && t.done >= 64 * 1024 * 1024 =>
                {
                    first_pause_done = t.done;
                    break;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("phone_events closed: {e}"),
            }
        }
    })
    .await
    .expect("transfer reaches >= 64 MiB before sender pause");

    phone.pause_transfer(&transfer_id);

    let wait_paused = async |events: &mut tokio::sync::broadcast::Receiver<NodeEvent>, id: &str| {
        loop {
            match events.recv().await {
                Ok(NodeEvent::Transfer(t)) if t.id == id && t.state == TransferState::Paused => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("events closed waiting for Paused: {e}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            wait_paused(&mut phone_events, &transfer_id),
            wait_paused(&mut pc_events, &transfer_id),
        );
    })
    .await
    .expect("both sides enter Paused state on sender pause");

    // Resume from sender (`phone`).
    phone.resume_transfer(&transfer_id);

    // 2. Wait until transfer resumes and progresses beyond `first_pause_done`, then pause from receiver (`pc`)
    // to exercise mid-transfer interruption and range resumption from the receiving side.
    let mut second_pause_done = 0u64;
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match pc_events.recv().await {
                Ok(NodeEvent::Transfer(t))
                    if t.id == transfer_id
                        && t.state == TransferState::Running
                        && t.done > first_pause_done =>
                {
                    second_pause_done = t.done;
                    break;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("pc_events closed: {e}"),
            }
        }
    })
    .await
    .expect("transfer resumes and advances before receiver pause");

    pc.pause_transfer(&transfer_id);
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            wait_paused(&mut phone_events, &transfer_id),
            wait_paused(&mut pc_events, &transfer_id),
        );
    })
    .await
    .expect("both sides enter Paused state on receiver interruption");

    // Resume from receiver (`pc`), which sends `files.resume` to `phone`.
    pc.resume_transfer(&transfer_id);

    // 3. Wait for completion (`TransferState::Done { saved }`) on receiver (`pc`).
    let saved_path: PathBuf = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            match pc_events.recv().await {
                Ok(NodeEvent::Transfer(t)) if t.id == transfer_id => match t.state {
                    TransferState::Done { mut saved } => {
                        return saved.pop().expect("saved file path");
                    }
                    TransferState::Failed(err) => {
                        panic!("4 GiB transfer failed unexpectedly: {err:?}");
                    }
                    TransferState::Cancelled => {
                        panic!("4 GiB transfer cancelled unexpectedly");
                    }
                    _ => {}
                },
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(e) => panic!("pc_events closed waiting for Done: {e}"),
            }
        }
    })
    .await
    .expect("4 GiB transfer completes within 180s");
    let transfer_elapsed = transfer_start.elapsed();

    // 4. Verify received file size and full BLAKE3 integrity.
    let received_len = std::fs::metadata(&saved_path).expect("received file metadata").len();
    assert_eq!(received_len, FOUR_GIB, "received file size must be exactly 4 GiB");

    let verify_start = Instant::now();
    let actual_blake3 = {
        let mut f = std::fs::File::open(&saved_path).expect("open received 4 GiB file");
        let mut hasher = blake3::Hasher::new();
        let mut buf = vec![0u8; CHUNK_SIZE];
        loop {
            let n = f.read(&mut buf).expect("read received chunk");
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        hasher.finalize().to_hex().to_string()
    };
    let verify_elapsed = verify_start.elapsed();
    assert_eq!(actual_blake3, expected_blake3, "4 GiB BLAKE3 digest mismatch");

    println!(
        "[SCALE] 4 GiB transfer: gen={:?}, first_pause_at={} MiB, second_pause_at={} MiB, transfer_with_pause_resume={:?}, blake3_verify={:?}, digest={}",
        gen_elapsed,
        first_pause_done / (1024 * 1024),
        second_pause_done / (1024 * 1024),
        transfer_elapsed,
        verify_elapsed,
        expected_blake3
    );

    // 5. Immediately delete both 4 GiB files and verify cleanup before dropping TempDirs.
    std::fs::remove_file(&src_path).expect("remove source 4 GiB file");
    std::fs::remove_file(&saved_path).expect("remove received 4 GiB file");
    assert!(!src_path.exists(), "source 4 GiB file must be deleted");
    assert!(!saved_path.exists(), "received 4 GiB file must be deleted");

    phone.shutdown().await;
    pc.shutdown().await;
    drop(phone_dir);
    drop(pc_dir);
}
