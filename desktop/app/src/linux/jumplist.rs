use std::path::Path;

pub const ARG: &str = "--task";

pub const TASKS: &[(&str, &str, &str)] = &[
    (
        "send-files",
        "Send files to phone",
        "Pick files to send to your paired phone",
    ),
    (
        "take-photo",
        "Take photo with phone",
        "Capture a photo from your phone camera",
    ),
    (
        "scan-document",
        "Scan document",
        "Scan a document with your phone camera",
    ),
    (
        "find-phone",
        "Find my phone",
        "Ring your paired phone at full volume",
    ),
    ("open-shelf", "Open File Shelf", "Open the NectarLink File Shelf"),
];

pub fn build_task_arguments(action: &str, custom_data_dir: Option<&Path>) -> String {
    match custom_data_dir {
        Some(dir) => format!(
            "--data-dir \"{}\" {ARG} {action}",
            dir.to_string_lossy().replace('"', "\\\"")
        ),
        None => format!("{ARG} {action}"),
    }
}

pub fn register(_custom_data_dir: Option<&Path>) {}

pub fn unregister() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn task_arguments_without_custom_data_dir() {
        assert_eq!(
            build_task_arguments("send-files", None),
            "--task send-files"
        );
    }

    #[test]
    fn task_arguments_with_custom_data_dir_preserves_spaces() {
        let dir = PathBuf::from(r"C:\Users\Alice Smith\Nectar Data");
        assert_eq!(
            build_task_arguments("find-phone", Some(&dir)),
            r#"--data-dir "C:\Users\Alice Smith\Nectar Data" --task find-phone"#
        );
    }
}
