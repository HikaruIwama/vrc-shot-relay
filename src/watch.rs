use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant, SystemTime};

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: SystemTime,
}

struct Observation {
    stamp: FileStamp,
    since: Instant,
}

#[derive(Default)]
pub struct FileStability {
    observations: HashMap<PathBuf, Observation>,
}

impl FileStability {
    pub fn ready(&mut self, path: &Path) -> bool {
        let Ok(metadata) = fs::metadata(path) else {
            return true;
        };
        let stamp = FileStamp {
            len: metadata.len(),
            modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        };
        file_is_stable(&mut self.observations, path, stamp, Instant::now())
    }

    pub fn forget(&mut self, path: &Path) {
        self.observations.remove(path);
    }
}

fn file_is_stable(
    observations: &mut HashMap<PathBuf, Observation>,
    path: &Path,
    stamp: FileStamp,
    now: Instant,
) -> bool {
    match observations.get_mut(path) {
        Some(observation) if observation.stamp == stamp => {
            now.duration_since(observation.since) >= Duration::from_secs(1)
        }
        Some(observation) => {
            *observation = Observation { stamp, since: now };
            false
        }
        None => {
            observations.insert(path.to_owned(), Observation { stamp, since: now });
            false
        }
    }
}

pub fn start(
    folder: &Path,
) -> Result<(RecommendedWatcher, Receiver<notify::Result<notify::Event>>), String> {
    let (events_tx, events_rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = events_tx.send(event);
    })
    .map_err(|error| format!("Could not start photo watcher: {error}"))?;
    watcher
        .watch(folder, RecursiveMode::Recursive)
        .map_err(|error| format!("Could not watch photo folder: {error}"))?;
    Ok((watcher, events_rx))
}

pub fn discover_images(root: &Path, seen: &mut HashSet<PathBuf>) -> Result<Vec<PathBuf>, String> {
    let mut discovered = Vec::new();
    let mut folders = vec![root.to_owned()];
    while let Some(folder) = folders.pop() {
        let entries =
            fs::read_dir(&folder).map_err(|error| format!("Could not scan photos: {error}"))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("Could not scan photos: {error}"))?;
            let kind = entry
                .file_type()
                .map_err(|error| format!("Could not scan photos: {error}"))?;
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() && is_photo(&entry.path()) && seen.insert(entry.path()) {
                discovered.push(entry.path());
            }
        }
    }
    discovered.sort();
    Ok(discovered)
}

fn is_photo(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tif" | "tiff"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn finds_only_images_added_after_baseline_in_nested_folder() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("vrc-shot-relay-scan-{stamp}"));
        let month = root.join("2026-10");
        fs::create_dir_all(&month).unwrap();
        fs::write(month.join("old.png"), []).unwrap();
        let mut seen = HashSet::new();

        let baseline = discover_images(&root, &mut seen).unwrap();
        fs::write(month.join("new.png"), []).unwrap();
        fs::write(month.join("ignore.txt"), []).unwrap();
        let discovered = discover_images(&root, &mut seen).unwrap();
        let repeated = discover_images(&root, &mut seen).unwrap();

        assert_eq!(baseline, vec![month.join("old.png")]);
        assert_eq!(discovered, vec![month.join("new.png")]);
        assert!(repeated.is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn waits_until_file_stops_changing() {
        let mut observations = HashMap::new();
        let path = Path::new("photo.png");
        let now = Instant::now();
        let first = FileStamp {
            len: 10,
            modified: SystemTime::UNIX_EPOCH,
        };
        let changed = FileStamp {
            len: 20,
            modified: SystemTime::UNIX_EPOCH,
        };

        let first_observation = file_is_stable(&mut observations, path, first, now);
        let changed_observation = file_is_stable(
            &mut observations,
            path,
            changed,
            now + Duration::from_secs(2),
        );
        let repeated_observation = file_is_stable(
            &mut observations,
            path,
            changed,
            now + Duration::from_secs(2),
        );
        let stable_observation = file_is_stable(
            &mut observations,
            path,
            changed,
            now + Duration::from_secs(3),
        );

        assert!(!first_observation);
        assert!(!changed_observation);
        assert!(!repeated_observation);
        assert!(stable_observation);
    }
}
