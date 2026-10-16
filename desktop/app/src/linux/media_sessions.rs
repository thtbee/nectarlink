use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use nectarlink_core::{MediaAction, MediaError, MediaPlayer};

const CALL_PAUSE_TTL: Duration = Duration::from_secs(60 * 30);

struct PausedForCall {
    players: Vec<String>,
    paused_at: Instant,
}

impl PausedForCall {
    fn take_valid(&mut self, now: Instant) -> Vec<String> {
        let taken = std::mem::take(&mut self.players);
        if now.saturating_duration_since(self.paused_at) <= CALL_PAUSE_TTL {
            taken
        } else {
            Vec::new()
        }
    }
}

static PAUSED_FOR_CALL: Mutex<Option<PausedForCall>> = Mutex::new(None);

pub fn start<F>(_on_change: F)
where
    F: Fn(Vec<MediaPlayer>) + Send + Sync + 'static,
{
}

pub fn command(player: &str, action: MediaAction, position: Option<u64>) -> Result<(), MediaError> {
    let mut cmd = Command::new("playerctl");
    if !player.is_empty() {
        cmd.args(["--player", player]);
    }
    match action {
        MediaAction::Play => {
            cmd.arg("play");
        }
        MediaAction::Pause => {
            cmd.arg("pause");
        }
        MediaAction::Next => {
            cmd.arg("next");
        }
        MediaAction::Previous => {
            cmd.arg("previous");
        }
        MediaAction::Seek => {
            let secs = (position.unwrap_or(0) as f64) / 1000.0;
            cmd.args(["position", &format!("{secs:.2}")]);
        }
    }
    let _ = cmd.status();
    Ok(())
}

pub fn pause_for_call() {
    if let Ok(out) = Command::new("playerctl")
        .args(["--all-players", "status", "--format", "{{playerName}}\t{{status}}"])
        .output()
        && out.status.success()
    {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut paused = Vec::new();
        for line in stdout.lines() {
            if let Some((name, status)) = line.split_once('\t')
                && status.trim().eq_ignore_ascii_case("Playing")
            {
                let _ = Command::new("playerctl")
                    .args(["--player", name.trim(), "pause"])
                    .status();
                paused.push(name.trim().to_string());
            }
        }
        if !paused.is_empty()
            && let Ok(mut guard) = PAUSED_FOR_CALL.lock()
        {
            *guard = Some(PausedForCall {
                players: paused,
                paused_at: Instant::now(),
            });
        }
    }
}

pub fn resume_after_call() {
    let players = PAUSED_FOR_CALL
        .lock()
        .ok()
        .and_then(|mut g| g.as_mut().map(|p| p.take_valid(Instant::now())))
        .unwrap_or_default();
    for player in players {
        let _ = Command::new("playerctl")
            .args(["--player", &player, "play"])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paused_for_call_expires_after_ttl() {
        let start = Instant::now();
        let mut state = PausedForCall {
            players: vec!["spotify".to_string()],
            paused_at: start,
        };
        let expired = state.take_valid(start + CALL_PAUSE_TTL + Duration::from_secs(1));
        assert!(expired.is_empty());
    }

    #[test]
    fn paused_for_call_returns_players_within_ttl() {
        let start = Instant::now();
        let mut state = PausedForCall {
            players: vec!["spotify".to_string(), "vlc".to_string()],
            paused_at: start,
        };
        let valid = state.take_valid(start + Duration::from_secs(10));
        assert_eq!(valid, vec!["spotify".to_string(), "vlc".to_string()]);
        assert!(state.take_valid(start + Duration::from_secs(11)).is_empty());
    }
}
