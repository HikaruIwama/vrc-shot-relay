#![windows_subsystem = "windows"]

mod config;
mod instance;
mod tray;

use std::path::PathBuf;
use std::sync::mpsc;
use vrc_shot_relay::engine;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{FOLDERID_Pictures, KF_FLAG_DEFAULT, SHGetKnownFolderPath};

fn main() {
    let directory = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let _instance = match instance::acquire(&directory) {
        Ok(lock) => lock,
        Err(_) => return,
    };
    let (ui_tx, ui_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();
    let startup = (|| -> Result<Option<(reqwest::Url, PathBuf)>, String> {
        let config = config::load_or_create(&directory)?;
        let Some(url) = config::webhook_url(&config)? else {
            return Ok(None);
        };
        let folder = if config.watch_folder.is_empty() {
            let pictures = pictures_folder()?;
            let vrchat_config = read_vrchat_config()?;
            config::resolve_watch_folder(&config, vrchat_config.as_deref(), &pictures)?
        } else {
            config::resolve_watch_folder(&config, None, std::path::Path::new(""))?
        };
        Ok(Some((url, folder)))
    })();
    let (sender, initial_status) = match startup {
        Ok(Some((url, folder))) => {
            let events = ui_tx.clone();
            let work_directory = directory.clone();
            std::thread::spawn(move || {
                if let Err(error) = engine::run(
                    folder,
                    work_directory.clone(),
                    url,
                    command_rx,
                    events.clone(),
                ) {
                    engine::report_error(&work_directory, &events, &error);
                }
            });
            (Some(command_tx), "Starting".to_string())
        }
        Ok(None) => (
            None,
            "Set webhook_url in config.toml, then restart".to_string(),
        ),
        Err(error) => {
            engine::report_error(&directory, &ui_tx, &error);
            (None, "Configuration error".to_string())
        }
    };
    if let Err(error) = tray::run(sender, ui_rx, initial_status) {
        engine::report_error(&directory, &ui_tx, &format!("Tray error: {error}"));
    }
}

fn pictures_folder() -> Result<PathBuf, String> {
    unsafe {
        let value = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None)
            .map_err(|error| format!("Could not locate Pictures folder: {error}"))?;
        let result = value
            .to_string()
            .map(PathBuf::from)
            .map_err(|error| format!("Could not read Pictures folder: {error}"));
        CoTaskMemFree(Some(value.0.cast()));
        result
    }
}

fn read_vrchat_config() -> Result<Option<String>, String> {
    let home = std::env::var_os("USERPROFILE").ok_or("Could not locate user profile")?;
    let path = PathBuf::from(home).join("AppData/LocalLow/VRChat/VRChat/config.json");
    if path.exists() {
        std::fs::read_to_string(path)
            .map(Some)
            .map_err(|error| format!("Could not read VRChat config.json: {error}"))
    } else {
        Ok(None)
    }
}
