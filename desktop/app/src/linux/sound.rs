use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

static RINGING: AtomicBool = AtomicBool::new(false);

const MAX_RING_DURATION: Duration = Duration::from_secs(60);
const RING_INTERVAL: Duration = Duration::from_millis(2200);

pub fn start_ringing() {
    if RINGING.swap(true, Ordering::SeqCst) {
        return;
    }
    thread::spawn(|| {
        let started = Instant::now();
        while RINGING.load(Ordering::SeqCst) && started.elapsed() < MAX_RING_DURATION {
            let _ = Command::new("canberra-gtk-play")
                .args(["-i", "phone-incoming-call"])
                .status()
                .or_else(|_| {
                    Command::new("paplay")
                        .arg("/usr/share/sounds/freedesktop/stereo/phone-incoming-call.oga")
                        .status()
                });
            let step = Duration::from_millis(100);
            let mut waited = Duration::ZERO;
            while waited < RING_INTERVAL && RINGING.load(Ordering::SeqCst) {
                thread::sleep(step);
                waited += step;
            }
        }
        RINGING.store(false, Ordering::SeqCst);
    });
}

pub fn stop_ringing() {
    RINGING.store(false, Ordering::SeqCst);
}
