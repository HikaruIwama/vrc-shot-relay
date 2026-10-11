use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Default, Deserialize, Serialize)]
struct State {
    pending: VecDeque<PathBuf>,
}

pub struct Queue {
    file: PathBuf,
    state: State,
}

impl Queue {
    pub fn load(file: &Path) -> Result<Self, String> {
        let state = if file.exists() {
            let text = fs::read_to_string(file)
                .map_err(|error| format!("Could not read state: {error}"))?;
            serde_json::from_str(&text).map_err(|error| format!("Invalid state file: {error}"))?
        } else {
            State::default()
        };
        Ok(Self {
            file: file.to_owned(),
            state,
        })
    }

    pub fn enqueue(&mut self, path: PathBuf) -> Result<bool, String> {
        if self.state.pending.contains(&path) {
            return Ok(false);
        }
        self.state.pending.push_back(path);
        if let Err(error) = self.save() {
            self.state.pending.pop_back();
            return Err(error);
        }
        Ok(true)
    }

    pub fn front(&self) -> Option<&PathBuf> {
        self.state.pending.front()
    }

    pub fn complete_front(&mut self) -> Result<(), String> {
        let Some(front) = self.state.pending.pop_front() else {
            return Ok(());
        };
        if let Err(error) = self.save() {
            self.state.pending.push_front(front);
            return Err(error);
        }
        Ok(())
    }

    pub fn defer_front(&mut self) -> Result<(), String> {
        if self.state.pending.len() < 2 {
            return Ok(());
        }
        let front = self.state.pending.pop_front().unwrap();
        self.state.pending.push_back(front);
        if let Err(error) = self.save() {
            let front = self.state.pending.pop_back().unwrap();
            self.state.pending.push_front(front);
            return Err(error);
        }
        Ok(())
    }

    fn save(&self) -> Result<(), String> {
        let temporary = self.file.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(&self.state).map_err(|error| error.to_string())?;
        let mut file =
            File::create(&temporary).map_err(|error| format!("Could not save state: {error}"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| format!("Could not save state: {error}"))?;
        fs::rename(&temporary, &self.file)
            .map_err(|error| format!("Could not replace state: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn pending_photo_survives_restart_without_duplicate() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = std::env::temp_dir().join(format!("vrc-shot-relay-test-{stamp}.json"));
        let photo = PathBuf::from(r"C:\Photos\new.png");

        {
            let mut queue = Queue::load(&file).unwrap();
            let first = queue.enqueue(photo.clone()).unwrap();
            let duplicate = queue.enqueue(photo.clone()).unwrap();

            assert!(first);
            assert!(!duplicate);
        }

        let mut queue = Queue::load(&file).unwrap();
        assert_eq!(queue.front(), Some(&photo));

        queue.complete_front().unwrap();
        let reloaded = Queue::load(&file).unwrap();

        assert_eq!(reloaded.front(), None);

        std::fs::remove_file(file).unwrap();
    }

    #[test]
    fn failed_photo_does_not_block_next_photo() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = std::env::temp_dir().join(format!("vrc-shot-relay-test-{stamp}.json"));
        let mut queue = Queue::load(&file).unwrap();

        queue.enqueue(PathBuf::from("failed.png")).unwrap();
        queue.enqueue(PathBuf::from("next.png")).unwrap();
        queue.defer_front().unwrap();

        let reloaded = Queue::load(&file).unwrap();

        assert_eq!(reloaded.front(), Some(&PathBuf::from("next.png")));

        std::fs::remove_file(file).unwrap();
    }
}
