# VRC Shot Relay

A tray app that posts new VRChat photos to Discord, with world info when available.

1. Download the Windows ZIP from [Releases](https://github.com/HikaruIwama/vrc-shot-relay/releases) and extract it.
2. Open `config.toml` and set `webhook_url` to your Discord webhook URL.
3. Run `vrc-shot-relay.exe`.

Leave `watch_folder` empty to detect your VRChat photo folder automatically.

Photos larger than `max_upload_mb` (default 8, allowed 1 to 100) are shrunk before posting. Raise it to match your server's Discord upload limit.

To start VRC Shot Relay when you sign in to Windows, select **Start with Windows** in the tray menu. Select it again to turn it off. If you move the app folder, turn it on again from the new location.
