use crate::config;
use std::path::{Path, PathBuf};
use vrc_shot_relay::engine;

/// Files and folders that the tray menu opens for the user.
pub struct Locations {
    pub config: PathBuf,
    pub log: PathBuf,
    pub photos: PathBuf,
}

impl Locations {
    pub fn new(directory: &Path, photos: PathBuf) -> Self {
        Self {
            config: directory.join(config::FILE_NAME),
            log: directory.join(engine::LOG_FILE_NAME),
            photos,
        }
    }

    /// The log is created on the first error, so it may not exist yet.
    pub fn log_available(&self) -> bool {
        self.log.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_directory(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("vrc-shot-relay-{name}-{stamp}"));
        std::fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn settings_and_log_are_next_to_the_app() {
        let directory = Path::new(r"C:\Apps\VRC Shot Relay");

        let locations = Locations::new(directory, PathBuf::from(r"D:\VRC Photos"));

        assert_eq!(
            locations.config,
            PathBuf::from(r"C:\Apps\VRC Shot Relay\config.toml")
        );
        assert_eq!(
            locations.log,
            PathBuf::from(r"C:\Apps\VRC Shot Relay\relay.log")
        );
        assert_eq!(locations.photos, PathBuf::from(r"D:\VRC Photos"));
    }

    #[test]
    fn log_is_unavailable_before_first_error() {
        let directory = test_directory("no-log");
        let locations = Locations::new(&directory, directory.clone());

        let available = locations.log_available();
        std::fs::remove_dir_all(&directory).unwrap();

        assert!(!available);
    }

    #[test]
    fn log_is_available_after_it_is_written() {
        let directory = test_directory("log");
        std::fs::write(directory.join("relay.log"), "error\n").unwrap();
        let locations = Locations::new(&directory, directory.clone());

        let available = locations.log_available();
        std::fs::remove_dir_all(&directory).unwrap();

        assert!(available);
    }
}
