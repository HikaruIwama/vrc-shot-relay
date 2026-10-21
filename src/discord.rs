use crate::metadata::WorldInfo;
use crate::photo::PreparedPhoto;
use reqwest::blocking::{Client, multipart};
use serde_json::json;
use std::time::Duration;

fn parse_retry_after(value: &str) -> Option<Duration> {
    let seconds: f64 = value.trim().parse().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(seconds.min(3600.0)))
}

pub struct SendError {
    pub message: String,
    pub retry_after: Option<Duration>,
}

impl SendError {
    pub fn new(message: String) -> Self {
        Self {
            message,
            retry_after: None,
        }
    }
}

fn message_payload(
    world: Option<&WorldInfo>,
    captured_at: Option<&str>,
    extension: &str,
) -> serde_json::Value {
    let Some(world) = world else {
        return json!({"allowed_mentions": {"parse": []}});
    };
    let mut embed = json!({
        "title": world.name.chars().take(256).collect::<String>(),
        "url": format!("https://vrchat.com/home/world/{}", world.id),
        "color": 0x3399ff,
        "image": {"url": format!("attachment://photo.{extension}")},
    });
    if let Some(captured_at) = captured_at {
        embed["footer"] = json!({"text": format!("Taken · {captured_at}")});
    }
    json!({"allowed_mentions": {"parse": []}, "embeds": [embed]})
}

pub struct Discord {
    client: Client,
    url: reqwest::Url,
}

impl Discord {
    pub fn new(url: reqwest::Url) -> Result<Self, String> {
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(45))
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { client, url })
    }

    pub fn verify(&self) -> Result<(), String> {
        let response = self
            .client
            .get(self.url.clone())
            .timeout(Duration::from_secs(10))
            .send()
            .map_err(|error| {
                if error.is_timeout() {
                    "Could not verify Discord webhook: request timed out. Check your connection, then restart.".to_string()
                } else {
                    "Could not verify Discord webhook: network request failed. Check your connection, then restart.".to_string()
                }
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!(
                "Could not verify Discord webhook: HTTP {}. Check webhook_url in config.toml, then restart.",
                response.status().as_u16()
            ))
        }
    }

    pub fn send(
        &self,
        photo: PreparedPhoto,
        world: Option<&WorldInfo>,
        captured_at: Option<&str>,
    ) -> Result<(), SendError> {
        let payload = message_payload(world, captured_at, photo.extension);
        let file = multipart::Part::bytes(photo.bytes)
            .file_name(format!("photo.{}", photo.extension))
            .mime_str(photo.mime)
            .map_err(|error| SendError::new(error.to_string()))?;
        let form = multipart::Form::new()
            .text("payload_json", payload.to_string())
            .part("files[0]", file);
        let mut url = self.url.clone();
        url.query_pairs_mut().append_pair("wait", "true");
        let response = self
            .client
            .post(url)
            .multipart(form)
            .send()
            .map_err(|error| {
                SendError::new(if error.is_timeout() {
                    "Discord request timed out".to_string()
                } else if error.is_connect() {
                    "Discord connection failed".to_string()
                } else if error.is_body() {
                    "Discord upload failed".to_string()
                } else {
                    "Discord network request failed".to_string()
                })
            })?;
        if response.status().is_success() {
            Ok(())
        } else {
            let retry_after = if response.status().as_u16() == 429 {
                response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(parse_retry_after)
            } else {
                None
            };
            Err(SendError {
                message: format!("Discord returned HTTP {}", response.status().as_u16()),
                retry_after,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photo_without_world_has_no_embed() {
        let captured_at = "2026-10-11 12:34:56";

        let result = message_payload(None, Some(captured_at), "png");

        assert!(result.get("embeds").is_none());
        assert!(result.get("content").is_none());
    }

    #[test]
    fn world_embed_links_title_and_displays_photo_with_capture_time() {
        let world = WorldInfo {
            name: "Test World".into(),
            id: "wrld_f100c62e-3121-4380-9452-d5601269f49a".into(),
        };

        let result = message_payload(Some(&world), Some("2026-10-11 12:34:56"), "png");

        assert_eq!(result["embeds"][0]["title"], "Test World");
        assert_eq!(
            result["embeds"][0]["url"],
            "https://vrchat.com/home/world/wrld_f100c62e-3121-4380-9452-d5601269f49a"
        );
        assert_eq!(
            result["embeds"][0]["image"]["url"],
            "attachment://photo.png"
        );
        assert_eq!(
            result["embeds"][0]["footer"]["text"],
            "Taken · 2026-10-11 12:34:56"
        );
        assert!(result.get("content").is_none());
        assert!(result.get("flags").is_none());
    }

    #[test]
    fn accepts_fractional_retry_after_seconds() {
        let fractional = "0.5";
        let negative = "-1";

        let accepted = parse_retry_after(fractional);
        let rejected = parse_retry_after(negative);

        assert_eq!(accepted, Some(Duration::from_millis(500)));
        assert_eq!(rejected, None);
    }
}
