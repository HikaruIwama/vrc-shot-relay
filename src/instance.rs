use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;

pub fn acquire(directory: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(directory.join(".vrc-shot-relay.lock"))
        .map_err(|error| format!("Another instance may be running: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn only_one_process_can_hold_lock() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("vrc-shot-relay-instance-{stamp}"));
        std::fs::create_dir_all(&directory).unwrap();

        let first = acquire(&directory).unwrap();
        let second = acquire(&directory);

        assert!(second.is_err());

        drop(first);
        let after_release = acquire(&directory);

        assert!(after_release.is_ok());

        drop(after_release);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
