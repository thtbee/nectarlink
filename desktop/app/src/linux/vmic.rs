use std::process::Command;

pub const SOURCE_NAME: &str = "NectarLink_Virtual_Mic";

pub fn is_available() -> bool {
    Command::new("pactl")
        .arg("info")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn create_source(rate: u32, channels: u8) -> Result<u32, String> {
    let output = Command::new("pactl")
        .args([
            "load-module",
            "module-pipe-source",
            &format!("source_name={SOURCE_NAME}"),
            "file=/tmp/nectarlink-vmic.pcm",
            "format=s16le",
            &format!("rate={rate}"),
            &format!("channels={channels}"),
        ])
        .output()
        .map_err(|e| format!("failed to run pactl: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|e| format!("invalid pactl module id: {e}"))
}

pub fn destroy_source(module_id: u32) {
    let _ = Command::new("pactl")
        .args(["unload-module", &module_id.to_string()])
        .status();
}
