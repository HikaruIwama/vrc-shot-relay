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

pub fn webhook_url(config: &Config) -> Result<Option<reqwest::Url>, String> {
    if config.webhook_url.is_empty() {
        return Ok(None);
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
    Ok(Some(url))
}

#[cfg(test)]
mod tests {
    use super::*;

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
