use crate::{discord, metadata, photo, queue::Queue, watch};
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

pub enum Command {
    Pause(bool),
    Stop,
}

pub enum UiEvent {
    Status(String),
    Error(String),
}

pub fn run(
    folder: PathBuf,
    directory: PathBuf,
    url: reqwest::Url,
    commands: Receiver<Command>,
    ui: Sender<UiEvent>,
) -> Result<(), String> {
    fs::create_dir_all(&folder)
        .map_err(|error| format!("Could not access watch folder: {error}"))?;
    let mut seen = HashSet::new();
    watch::discover_images(&folder, &mut seen)?;
    let mut queue = Queue::load(&directory.join("state.json"))?;
    let discord = discord::Discord::new(url)?;
    let (_watcher, events_rx) = watch::start(&folder)?;
    let mut ready_at = HashMap::new();
    let mut stability = watch::FileStability::default();
    add_new_photos(&folder, &mut seen, &mut queue, &mut ready_at)?;
    let _ = ui.send(UiEvent::Status("Watching photos".into()));
    let mut paused = false;
    let mut next_attempt = Instant::now();
    let mut failures = 0u32;
    let mut last_error: Option<(PathBuf, String)> = None;
    let mut last_scan = Instant::now();

    loop {
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Pause(value) => {
                    paused = value;
                    if !paused {
                        next_attempt = Instant::now();
                    }
                    let _ = ui.send(UiEvent::Status(
                        if paused { "Paused" } else { "Watching photos" }.into(),
                    ));
                }
                Command::Stop => return Ok(()),
            }
        }
        let mut changed = false;
        while let Ok(event) = events_rx.try_recv() {
            match event {
                Ok(_) => changed = true,
                Err(error) => {
                    report_error(&directory, &ui, &format!("Photo watcher error: {error}"));
                    changed = true;
                }
            }
        }
        if changed || last_scan.elapsed() >= Duration::from_secs(5) {
            let fresh = watch::discover_images(&folder, &mut seen)?;
            for path in fresh {
                if !paused {
                    queue.enqueue(path.clone())?;
                    ready_at.insert(path, Instant::now() + Duration::from_secs(2));
                }
            }
            last_scan = Instant::now();
        }
        if !paused
            && Instant::now() >= next_attempt
            && let Some(path) = queue.front().cloned()
            && ready_at
                .get(&path)
                .is_none_or(|ready| Instant::now() >= *ready)
        {
            if !stability.ready(&path) {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            let result = photo::prepare_file(&path)
                .map_err(discord::SendError::new)
                .and_then(|prepared| {
                    let world = metadata::read_world(&path);
                    let captured_at = metadata::capture_time(&path);
                    discord.send(prepared, world.as_ref(), captured_at.as_deref())
                });
            match result {
                Ok(()) => {
                    queue.complete_front()?;
                    ready_at.remove(&path);
                    stability.forget(&path);
                    failures = 0;
                    last_error = None;
                    let _ = ui.send(UiEvent::Status("Watching photos".into()));
                }
                Err(error) => {
                    if last_error.as_ref() != Some(&(path.clone(), error.message.clone())) {
                        let name = path.file_name().unwrap_or_default().to_string_lossy();
                        report_error(&directory, &ui, &format!("{name}: {}", error.message));
                        last_error = Some((path.clone(), error.message));
                    }
                    failures = failures.saturating_add(1);
                    queue.defer_front()?;
                    next_attempt =
                        Instant::now() + error.retry_after.unwrap_or_else(|| retry_delay(failures));
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn add_new_photos(
    folder: &Path,
    seen: &mut HashSet<PathBuf>,
    queue: &mut Queue,
    ready_at: &mut HashMap<PathBuf, Instant>,
) -> Result<(), String> {
    for path in watch::discover_images(folder, seen)? {
        queue.enqueue(path.clone())?;
        ready_at.insert(path, Instant::now() + Duration::from_secs(2));
    }
    Ok(())
}

fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((5u64.saturating_mul(1u64 << failures.min(8))).min(1800))
}

pub fn report_error(directory: &Path, ui: &Sender<UiEvent>, error: &str) {
    if let Ok(mut log) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("relay.log"))
    {
        let timestamp = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_else(|_| "unknown-time".into());
        let _ = writeln!(log, "{timestamp} {error}");
    }
    let _ = ui.send(UiEvent::Error(error.into()));
}
