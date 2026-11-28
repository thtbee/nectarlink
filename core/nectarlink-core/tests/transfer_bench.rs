// SPDX-License-Identifier: MPL-2.0
//! How fast sharing between two devices is: file throughput, time to first
//! byte, many small files, and how soon small things (clipboard text) arrive,
//! alone and while a big file is on its way.
//!
//! Two real nodes on loopback, so it shows what the code costs, not what a
//! network does. Opt-in, and meant for release builds:
//! `cargo test --release -p nectarlink-core --test transfer_bench -- --ignored --nocapture --test-threads 1`.
//! `NECTARLINK_BENCH_MB` sets the big file's size (default 500).

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use nectarlink_core::{
    DeviceId, DeviceInfo, DeviceKind, FileSource, LinkState, Node, NodeConfig, NodeEvent, OutgoingFile,
    PlainKeyProtector, Platform, TransferState,
};
use tempfile::TempDir;
use tokio::sync::broadcast::{Receiver, error::RecvError};

fn config(dir: &TempDir, name: &str, kind: DeviceKind) -> NodeConfig {
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
        "0.1.0-bench",
    );
    cfg.lan_discovery = false;
    cfg.away_mode = false;
    cfg.localsend_port = 0;
    cfg.key_protector = Some(Arc::new(PlainKeyProtector));
    cfg
}

/// A device whose clipboard takes whatever comes.
struct Clipboard;

impl Platform for Clipboard {
    fn set_clipboard(&self, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

struct Pair {
    _pc_dir: TempDir,
    _phone_dir: TempDir,
    pc: Node,
    phone: Node,
}

async fn paired() -> Pair {
    let pc_dir = TempDir::new().unwrap();
    let phone_dir = TempDir::new().unwrap();
    let pc =
        Node::start(config(&pc_dir, "Bench PC", DeviceKind::Desktop), Arc::new(Clipboard)).await.unwrap();
    let phone =
        Node::start(config(&phone_dir, "Bench Phone", DeviceKind::Phone), Arc::new(Clipboard)).await.unwrap();
    let mut pc_events = pc.events();
    let mut phone_events = phone.events();
    let uri = pc.pairing_start_qr().await.unwrap();
    phone.pairing_join(&uri.to_uri()).await.unwrap();
    let online = async |events: &mut Receiver<NodeEvent>, peer: DeviceId| {
        wait(events, Duration::from_secs(15), |e| match e {
            NodeEvent::LinkChanged { device, link: LinkState::Online { .. } } if *device == peer => Some(()),
            _ => None,
        })
        .await
    };
    tokio::join!(online(&mut pc_events, phone.device_id()), online(&mut phone_events, pc.device_id()));
    Pair { _pc_dir: pc_dir, _phone_dir: phone_dir, pc, phone }
}

async fn wait<T>(
    events: &mut Receiver<NodeEvent>,
    limit: Duration,
    mut pick: impl FnMut(&NodeEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(limit, async {
        loop {
            match events.recv().await {
                Ok(e) => {
                    if let Some(found) = pick(&e) {
                        return found;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(e) => panic!("events ended: {e}"),
            }
        }
    })
    .await
    .expect("in time")
}

/// Writes `size` bytes of a pattern that doesn't compress, in 8 MiB pieces.
fn write_file(path: &Path, size: u64, seed: u64) {
    let mut f = std::io::BufWriter::with_capacity(8 << 20, std::fs::File::create(path).unwrap());
    let mut state = seed | 1;
    let mut buf = vec![0u8; (8 << 20).min(size as usize)];
    let mut left = size;
    while left > 0 {
        for slot in buf.as_chunks_mut::<8>().0 {
            // xorshift64
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            slot.copy_from_slice(&state.to_le_bytes());
        }
        let n = buf.len().min(left as usize);
        f.write_all(&buf[..n]).unwrap();
        left -= n as u64;
    }
    f.flush().unwrap();
}

struct Timing {
    first_byte: Duration,
    total: Duration,
    saved: Vec<PathBuf>,
}

/// Sends `files` from `from` to `to`; times when the receiver first hears of
/// the transfer (the offer arrived: everything before the first byte) and
/// when it's saved.
async fn send(from: &Node, to: &Node, files: Vec<OutgoingFile>) -> Timing {
    let mut events = to.events();
    let start = Instant::now();
    let id = from.send_files(to.device_id(), files).await.unwrap();
    let first_byte = wait(&mut events, Duration::from_secs(120), |e| match e {
        NodeEvent::Transfer(t) if t.id == id => Some(start.elapsed()),
        _ => None,
    })
    .await;
    let saved = wait(&mut events, Duration::from_secs(600), |e| match e {
        NodeEvent::Transfer(t) if t.id == id => match &t.state {
            TransferState::Done { saved } => Some(saved.clone()),
            TransferState::Failed(f) => panic!("transfer failed: {f:?}"),
            TransferState::Cancelled => panic!("transfer cancelled"),
            _ => None,
        },
        _ => None,
    })
    .await;
    Timing { first_byte, total: start.elapsed(), saved }
}

/// CPU time this process has used so far (both nodes run in it).
#[cfg(windows)]
#[allow(unsafe_code)]
fn cpu_time() -> Duration {
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let (mut c, mut e, mut k, mut u) = Default::default();
    // SAFETY: the pseudo handle needs no closing; the out pointers are to locals.
    let ok = unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
    assert_ne!(ok, 0);
    let ticks = |t: &FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
    Duration::from_nanos((ticks(&k) + ticks(&u)) * 100)
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn cpu_time() -> Duration {
    // SAFETY: getrusage fills the zeroed struct it's given.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) }, 0);
    let secs = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    secs(usage.ru_utime) + secs(usage.ru_stime)
}

fn mb_per_s(bytes: u64, d: Duration) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / d.as_secs_f64()
}

fn big_size() -> u64 {
    std::env::var("NECTARLINK_BENCH_MB").ok().and_then(|m| m.parse::<u64>().ok()).unwrap_or(500) << 20
}

#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_one_big_file() {
    let pair = paired().await;
    let src_dir = TempDir::new().unwrap();
    let size = big_size();
    let path = src_dir.path().join("big.bin");
    write_file(&path, size, 7);
    for (label, from, to) in [("pc->phone", &pair.pc, &pair.phone), ("phone->pc", &pair.phone, &pair.pc)] {
        let file =
            OutgoingFile { name: "big.bin".into(), folder: None, source: FileSource::Path(path.clone()) };
        let cpu = cpu_time();
        let t = send(from, to, vec![file]).await;
        let cpu = cpu_time() - cpu;
        let after_first = t.total - t.first_byte;
        println!(
            "[BENCH] 1 x {} MiB {label}: first byte {:?}, total {:?} = {:.0} MiB/s overall, {:.0} MiB/s once moving, CPU {:?}",
            size >> 20,
            t.first_byte,
            t.total,
            mb_per_s(size, t.total),
            mb_per_s(size, after_first),
            cpu,
        );
        assert_eq!(std::fs::metadata(&t.saved[0]).unwrap().len(), size);
        for p in t.saved {
            std::fs::remove_file(p).unwrap();
        }
    }
    pair.pc.shutdown().await;
    pair.phone.shutdown().await;
}

#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_one_big_file_from_a_handle() {
    // How the phone sends: an open file (a content URI's descriptor), not a path.
    let pair = paired().await;
    let src_dir = TempDir::new().unwrap();
    let size = big_size();
    let path = src_dir.path().join("big.bin");
    write_file(&path, size, 11);
    let file = OutgoingFile {
        name: "big.bin".into(),
        folder: None,
        source: FileSource::File(std::fs::File::open(&path).unwrap()),
    };
    let t = send(&pair.phone, &pair.pc, vec![file]).await;
    println!(
        "[BENCH] 1 x {} MiB phone->pc from a handle: first byte {:?}, total {:?} = {:.0} MiB/s overall",
        size >> 20,
        t.first_byte,
        t.total,
        mb_per_s(size, t.total),
    );
    for p in t.saved {
        std::fs::remove_file(p).unwrap();
    }
    pair.pc.shutdown().await;
    pair.phone.shutdown().await;
}

#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_many_small_files() {
    let pair = paired().await;
    let src_dir = TempDir::new().unwrap();
    let folder = src_dir.path().join("Small");
    std::fs::create_dir(&folder).unwrap();
    const COUNT: u64 = 1000;
    const SIZE: u64 = 10 * 1024;
    for i in 0..COUNT {
        write_file(&folder.join(format!("{i:04}.bin")), SIZE, i + 1);
    }
    for (label, from, to) in [("pc->phone", &pair.pc, &pair.phone), ("phone->pc", &pair.phone, &pair.pc)] {
        let files = nectarlink_core::outgoing_paths(std::slice::from_ref(&folder)).unwrap();
        assert_eq!(files.len() as u64, COUNT);
        let t = send(from, to, files).await;
        println!(
            "[BENCH] {COUNT} x {} KiB folder {label}: first byte {:?}, total {:?} = {:.0} files/s",
            SIZE >> 10,
            t.first_byte,
            t.total,
            COUNT as f64 / t.total.as_secs_f64(),
        );
        assert_eq!(std::fs::read_dir(&t.saved[0]).unwrap().count() as u64, COUNT);
        std::fs::remove_dir_all(&t.saved[0]).unwrap();
    }
    pair.pc.shutdown().await;
    pair.phone.shutdown().await;
}

/// Sends `n` clipboard texts one after another and waits until all arrived;
/// returns each one's latency (send to `ClipboardReceived`) and the total.
async fn clipboard_burst(from: &Node, to: &Node, n: usize, tag: &str) -> (Vec<Duration>, Duration) {
    let mut events = to.events();
    let peer = to.device_id();
    let from_id = from.device_id();
    let start = Instant::now();
    let mut sent_at = Vec::with_capacity(n);
    let mut arrived = Vec::with_capacity(n);
    // Sent and counted together: `send_clipboard` may return before the text arrives.
    let receive = async {
        while arrived.len() < n {
            wait(&mut events, Duration::from_secs(600), |e| match e {
                NodeEvent::ClipboardReceived { device } if *device == from_id => Some(()),
                _ => None,
            })
            .await;
            arrived.push(start.elapsed());
        }
    };
    let send = async {
        for i in 0..n {
            sent_at.push(start.elapsed());
            from.send_clipboard(peer, format!("{tag} {i}: some copied text")).await.unwrap();
        }
    };
    tokio::join!(receive, send);
    let total = start.elapsed();
    let latencies = sent_at.iter().zip(&arrived).map(|(s, a)| a.saturating_sub(*s)).collect();
    (latencies, total)
}

fn summary(mut latencies: Vec<Duration>) -> String {
    latencies.sort();
    let n = latencies.len();
    let at = |q: f64| latencies[((n as f64 * q) as usize).min(n - 1)];
    format!("p50 {:?}, p90 {:?}, max {:?}", at(0.5), at(0.9), latencies[n - 1])
}

#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_clipboard_burst() {
    let pair = paired().await;
    // Warm up the path once.
    clipboard_burst(&pair.pc, &pair.phone, 1, "warm").await;
    let (lat, total) = clipboard_burst(&pair.pc, &pair.phone, 100, "burst").await;
    println!("[BENCH] 100 clipboard texts pc->phone: total {total:?}, latency {}", summary(lat));
    pair.pc.shutdown().await;
    pair.phone.shutdown().await;
}

#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_clipboard_during_a_big_transfer() {
    let pair = paired().await;
    let src_dir = TempDir::new().unwrap();
    let size = big_size().max(256 << 20);
    let path = src_dir.path().join("big.bin");
    write_file(&path, size, 13);
    let mut phone_events = pair.phone.events();
    let file = OutgoingFile { name: "big.bin".into(), folder: None, source: FileSource::Path(path.clone()) };
    let start = Instant::now();
    let id = pair.pc.send_files(pair.phone.device_id(), vec![file]).await.unwrap();
    // Once bytes are moving.
    wait(&mut phone_events, Duration::from_secs(120), |e| match e {
        NodeEvent::Transfer(t) if t.id == id && t.done > 0 => Some(()),
        _ => None,
    })
    .await;
    // One at a time: a text queued for a device replaces the one waiting before it.
    let mut lat = Vec::new();
    let during = Instant::now();
    for i in 0..5 {
        lat.extend(clipboard_burst(&pair.pc, &pair.phone, 1, &format!("during {i}")).await.0);
    }
    let total = during.elapsed();
    let saved = wait(&mut phone_events, Duration::from_secs(600), |e| match e {
        NodeEvent::Transfer(t) if t.id == id => match &t.state {
            TransferState::Done { saved } => Some(saved.clone()),
            TransferState::Failed(f) => panic!("transfer failed: {f:?}"),
            _ => None,
        },
        _ => None,
    })
    .await;
    println!(
        "[BENCH] 5 clipboard texts while {} MiB moves pc->phone: total {total:?}, latency {}; file done after {:?}",
        size >> 20,
        summary(lat),
        start.elapsed()
    );
    for p in saved {
        std::fs::remove_file(p).unwrap();
    }
    pair.pc.shutdown().await;
    pair.phone.shutdown().await;
}

/// What the transport alone moves on loopback, with the settings the app ships
/// (`TransportTuning`): the ceiling for [`bench_one_big_file`].
#[ignore = "benchmark; run in release with --ignored --nocapture"]
#[tokio::test(flavor = "multi_thread")]
async fn bench_raw_quic() {
    use iroh::endpoint::{QuicTransportConfig, VarInt, presets};
    const ALPN: &[u8] = b"nectarlink-bench/0";
    let size = big_size();
    let tuning = nectarlink_core::TransportTuning::from_env();
    let bind = async || {
        let controller: Arc<dyn iroh::endpoint::ControllerFactory + Send + Sync> = match tuning.congestion {
            nectarlink_core::Congestion::Cubic => Arc::new(noq_proto::congestion::CubicConfig::default()),
            nectarlink_core::Congestion::Bbr3 => Arc::new(noq_proto::congestion::Bbr3Config::default()),
            nectarlink_core::Congestion::NewReno => Arc::new(noq_proto::congestion::NewRenoConfig::default()),
        };
        let transport = QuicTransportConfig::builder()
            .stream_receive_window(VarInt::from_u32(tuning.stream_window))
            .receive_window(VarInt::from_u32(tuning.connection_window))
            .send_window(u64::from(tuning.connection_window))
            .congestion_controller_factory(controller)
            .build();
        iroh::Endpoint::builder(presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .alpns(vec![ALPN.to_vec()])
            .transport_config(transport)
            .clear_ip_transports()
            .bind_addr(std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            .unwrap()
            .bind()
            .await
            .unwrap()
    };
    let (a, b) = (bind().await, bind().await);
    let b_addr = iroh::EndpointAddr::new(b.id()).with_ip_addr(b.bound_sockets()[0]);
    let server = tokio::spawn(async move {
        let conn = b.accept().await.unwrap().await.unwrap();
        let (_send, mut recv) = conn.accept_bi().await.unwrap();
        let mut got = 0u64;
        let mut chunks = 0u64;
        while let Some(chunk) = recv.read_chunk(256 * 1024).await.unwrap() {
            got += chunk.len() as u64;
            chunks += 1;
        }
        (got, chunks, conn, b)
    });
    let conn = a.connect(b_addr, ALPN).await.unwrap();
    let (mut send, _recv) = conn.open_bi().await.unwrap();
    let block = bytes::Bytes::from(vec![5u8; 1 << 20]);
    let start = Instant::now();
    let mut left = size;
    while left > 0 {
        let n = left.min(block.len() as u64) as usize;
        send.write_chunk(block.slice(..n)).await.unwrap();
        left -= n as u64;
    }
    send.finish().unwrap();
    let (got, chunks, _conn, _b) = server.await.unwrap();
    let took = start.elapsed();
    println!(
        "[BENCH] raw QUIC {} MiB: {took:?} = {:.0} MiB/s, {} bytes per read_chunk on average",
        got >> 20,
        mb_per_s(got, took),
        got / chunks.max(1)
    );
}
