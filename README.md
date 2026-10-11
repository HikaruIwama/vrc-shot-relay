# VRC Shot Relay

A tray app that posts new VRChat photos to Discord, with world info when available.

## Setup

1. Build the app with `mise install` and `mise run build`.
2. Copy `config.example.toml` to `target/release/config.toml` and set `webhook_url`.
3. Run `target/release/vrc-shot-relay.exe`.

Leave `watch_folder = ""` for automatic detection. Keep `config.toml` private because it contains your webhook URL.
