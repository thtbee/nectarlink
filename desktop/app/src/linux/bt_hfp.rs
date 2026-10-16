use std::process::Command;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HfpStatus {
    pub available: bool,
    pub connected: bool,
    pub device_name: String,
    pub detail: String,
}

pub fn match_endpoint<'a>(
    endpoints: &'a [(String, String)],
    phone_name: Option<&str>,
) -> Option<&'a (String, String)> {
    if endpoints.is_empty() {
        return None;
    }
    if let Some(needle) = phone_name.map(str::trim).filter(|s| !s.is_empty()) {
        let needle_lower = needle.to_ascii_lowercase();
        if let Some(found) = endpoints
            .iter()
            .find(|(_, name)| name.to_ascii_lowercase().contains(&needle_lower))
        {
            return Some(found);
        }
    }
    endpoints.first()
}

pub fn probe_status(phone_name: Option<&str>) -> HfpStatus {
    HfpStatus {
        available: false,
        connected: false,
        device_name: phone_name.unwrap_or_default().to_string(),
        detail: "Pair your phone over system Bluetooth to route call audio through this PC."
            .to_string(),
    }
}

pub fn connect_audio(phone_name: Option<&str>) -> Result<HfpStatus, String> {
    Ok(probe_status(phone_name))
}

pub fn disconnect_audio() {}

pub fn open_bluetooth_settings() {
    let _ = Command::new("blueman-manager")
        .spawn()
        .or_else(|_| Command::new("gnome-control-center").arg("bluetooth").spawn())
        .or_else(|_| Command::new("xdg-open").arg("settings://bluetooth").spawn());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn match_endpoint_prefers_phone_name_case_insensitive() {
        let list = vec![
            ("id-1".to_string(), "Galaxy Buds".to_string()),
            ("id-2".to_string(), "Pixel 9 Pro".to_string()),
        ];
        let matched = match_endpoint(&list, Some("pixel 9")).unwrap();
        assert_eq!(matched.0, "id-2");
    }

    #[test]
    fn match_endpoint_falls_back_to_first_when_unmatched() {
        let list = vec![("id-1".to_string(), "Phone A".to_string())];
        let matched = match_endpoint(&list, Some("Other Phone")).unwrap();
        assert_eq!(matched.0, "id-1");
    }
}
