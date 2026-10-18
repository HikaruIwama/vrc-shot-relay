use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    pub webhook_url: String,
    pub watch_folder: String,
}

pub fn resolve_watch_folder(
    config: &Config,
    vrchat_config: Option<&str>,
    pictures: &Path,
) -> Result<PathBuf, String> {
    if !config.watch_folder.is_empty() {
        return Ok(PathBuf::from(&config.watch_folder));
    }
    if let Some(raw) = vrchat_config {
        let settings: serde_json::Value = serde_json::from_str(raw)
            .map_err(|error| format!("Could not read VRChat config.json: {error}"))?;
        if let Some(path) = settings
            .get("picture_output_folder")
            .and_then(|value| value.as_str())
            && !path.is_empty()
        {
            return Ok(PathBuf::from(path));
        }
    }
    Ok(pictures.join("VRChat"))
}

pub fn load_or_create(directory: &Path) -> Result<Config, String> {
    let path = directory.join("config.toml");
    if !path.exists() {
        let template =
            toml::to_string_pretty(&Config::default()).map_err(|error| error.to_string())?;
        fs::write(&path, template)
            .map_err(|error| format!("Could not create config.toml: {error}"))?;
    }
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("Could not read config.toml: {error}"))?;
    toml::from_str(&text).map_err(|error| format!("Invalid config.toml: {error}"))
}

pub fn validate_watch_folder(folder: &Path) -> Result<(), String> {
    fs::read_dir(folder).map_err(|error| {
        format!(
            "Could not read photo folder:\n{}\n\n{error}\n\nCheck watch_folder in config.toml or the photo save location in VRChat.",
            folder.display()
        )
    })?;
    Ok(())
}

pub fn webhook_url(config: &Config) -> Result<reqwest::Url, String> {
    if config.webhook_url.trim().is_empty() {
        return Err("Webhook URL is not set. Set webhook_url in config.toml, then restart.".into());
    }
    let url =
        reqwest::Url::parse(&config.webhook_url).map_err(|_| "Invalid Webhook URL".to_string())?;
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !matches!(
            host,
            "discord.com" | "canary.discord.com" | "ptb.discord.com"
        )
        || !url.path().starts_with("/api/webhooks/")
    {
        return Err("Webhook URL must be a Discord HTTPS webhook".into());
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_webhook_prevents_startup() {
        assert!(webhook_url(&Config::default()).is_err());
    }

    #[test]
    fn missing_photo_folder_prevents_startup_without_creating_it() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!("vrc-shot-relay-missing-{stamp}"));

        let result = validate_watch_folder(&folder);

        assert!(result.is_err());
        assert!(!folder.exists());
    }

    #[test]
    fn file_cannot_be_used_as_photo_folder() {
        let file = std::env::current_exe().unwrap();

        assert!(validate_watch_folder(&file).is_err());
    }

    #[test]
    fn readable_photo_folder_allows_startup() {
        assert!(validate_watch_folder(&std::env::temp_dir()).is_ok());
    }

    #[test]
    fn invalid_webhook_prevents_startup() {
        let config = Config {
            webhook_url: "not a URL".into(),
            ..Config::default()
        };

        assert!(webhook_url(&config).is_err());
    }

    #[test]
    fn valid_webhook_allows_startup() {
        let config = Config {
            webhook_url: "https://discord.com/api/webhooks/123/token".into(),
            ..Config::default()
        };

        assert_eq!(
            webhook_url(&config).unwrap().as_str(),
            "https://discord.com/api/webhooks/123/token"
        );
    }

    #[test]
    fn empty_folder_prefers_vrchat_setting() {
        let config = Config::default();

        let result = resolve_watch_folder(
            &config,
            Some(r#"{"picture_output_folder":"D:/VRC Photos"}"#),
            Path::new(r"C:\Users\Alice\Pictures"),
        )
        .unwrap();

        assert_eq!(result, PathBuf::from("D:/VRC Photos"));
    }

    #[test]
    fn empty_folder_uses_pictures_without_vrchat_setting() {
        let config = Config::default();
        let pictures = Path::new(r"C:\Users\Alice\Pictures");

        let result = resolve_watch_folder(&config, None, pictures).unwrap();

        assert_eq!(result, PathBuf::from(r"C:\Users\Alice\Pictures\VRChat"));
    }

    #[test]
    fn explicit_folder_overrides_vrchat_setting() {
        let config = Config {
            watch_folder: r"E:\Photos".into(),
            ..Config::default()
        };

        let result = resolve_watch_folder(
            &config,
            Some(r#"{"picture_output_folder":"D:/VRC Photos"}"#),
            Path::new(r"C:\Users\Alice\Pictures"),
        )
        .unwrap();

        assert_eq!(result, PathBuf::from(r"E:\Photos"));
    }

    #[test]
    fn omitted_watch_folder_uses_default() {
        let text = "webhook_url = 'https://discord.com/api/webhooks/123/token'";

        let config: Config = toml::from_str(text).unwrap();

        assert_eq!(config.watch_folder, "");
    }

    #[test]
    fn first_run_creates_editable_config_without_webhook() {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("vrc-shot-relay-config-{stamp}"));
        std::fs::create_dir_all(&directory).unwrap();

        let config = load_or_create(&directory).unwrap();

        assert!(config.webhook_url.is_empty());
        assert!(config.watch_folder.is_empty());
        assert!(directory.join("config.toml").exists());

        std::fs::remove_dir_all(directory).unwrap();
    }
}
