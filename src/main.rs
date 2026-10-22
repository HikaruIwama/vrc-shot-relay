#![windows_subsystem = "windows"]

mod config;
mod dialog;
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
    let startup = (|| -> Result<engine::Engine, String> {
        let config = config::load_or_create(&directory)?;
        let url = config::webhook_url(&config)?;
        let folder = if config.watch_folder.is_empty() {
            let pictures = pictures_folder()?;
            let vrchat_config = read_vrchat_config()?;
            config::resolve_watch_folder(&config, vrchat_config.as_deref(), &pictures)?
        } else {
            config::resolve_watch_folder(&config, None, std::path::Path::new(""))?
        };
        config::validate_watch_folder(&folder)?;
        engine::prepare(folder, directory.clone(), url, config.preserve_metadata)
    })();
    let engine = match startup {
        Ok(engine) => engine,
        Err(error) => {
            engine::log_error(&directory, &error);
            dialog::show_startup_error(&format!(
                "{error}\n\nCheck the settings in:\n{}\n\nCorrect the problem, then restart VRC Shot Relay.",
                directory.join("config.toml").display()
            ));
            return;
        }
    };
    let events = ui_tx.clone();
    let work_directory = directory.clone();
    std::thread::spawn(move || {
        if let Err(error) = engine::run(engine, command_rx, events.clone()) {
            engine::report_error(&work_directory, &events, &error);
        }
    });
    if let Err(error) = tray::run(command_tx.clone(), ui_rx, "Starting".to_string()) {
        let _ = command_tx.send(engine::Command::Stop);
        let error = format!("Could not start tray: {error}");
        engine::log_error(&directory, &error);
        dialog::show_startup_error(&error);
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
