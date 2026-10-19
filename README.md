# personal-iot

My home automation, built around [Home Assistant](https://www.home-assistant.io) in Docker. One
server controls the air conditioners and fans in two houses, understands voice commands, and
comes with a native desktop app.

- **Two houses, one server.** Devices are reached through Tuya's cloud, so no house needs a hub.
- **IR devices as real devices.** IR air conditioners get temperature, mode and fan speed; an IR
  fan with learned remote buttons gets speeds 1–4.
- **Local voice.** Speech-to-text, text-to-speech and the "okay nabu" wake word run on the server.
- **Home, a desktop app.** Native Rust, no browser: device buttons, a talk shortcut, a tray icon.
- **Private by default.** Reachable over Tailscale; a Cloudflare Tunnel is optional.

```mermaid
flowchart LR
    phone["Phone<br/>Home Assistant app"] -->|Tailscale or<br/>Cloudflare Tunnel| ha
    pc["PC<br/>Home desktop app"] -->|Tailscale| ha
    ha["<b>Server (Docker)</b><br/>Home Assistant<br/>whisper · piper · openWakeWord"]
    ha <-->|Tuya cloud| devices["ACs, fans, switches<br/>in both houses"]
    ha <-->|Tailscale| rgb["PC lights<br/>OpenRGB"]
    ha -->|voice commands| agents["Jev, with an<br/>LLM fallback"]
```

## Quick start

You need a Linux server with Docker and [Tailscale](https://tailscale.com), and a Smart Life
(Tuya) account for the devices.

**1. The server.** One script does the rest, asking for what it needs as it goes:

```sh
git clone https://github.com/InterStella0/personal-iot.git ~/personal-iot
~/personal-iot/server/setup.sh
```

It creates your account with two-factor login, starts the speech services, adds the integrations,
and builds the voice assistant. The one step it can't do is Tuya's QR login: it tells you when to
add Tuya in the web UI and waits. Keys for Jev and DeepSeek are optional; rerun the script
whenever you have them.

**2. The PC.** Build and install the desktop app, then sign in with the same account:

```sh
cd ~/personal-iot/desktop && ./install.sh
```

**3. The phone.** Install the Home Assistant app and connect to `http://<server>:8123` over
Tailscale, or set up a [Cloudflare Tunnel](docs/details.md#remote-access) to skip Tailscale.

## What's where

```
server/
├── setup.sh              sets everything up; safe to rerun
├── setup/                the part of setup that talks to Home Assistant's API
├── compose.yaml          Home Assistant, plus whisper, piper and openWakeWord
├── patches/              fixes for third-party integrations
└── ha-config/            Home Assistant's config (packages/ holds the tracked YAML)
desktop/                  Home, the desktop app (Rust)
docs/details.md           how each piece works, and what to check when something breaks
tests/jev/                voice-routing tests
```

## Roadmap

- Test Jev on 20–30 real voice commands, and switch the AC automation from dry run to live.
- The Model Context Protocol Server integration, for Claude Desktop and Claude Code.
- The second house's fan, Google Home, and Matter/Zigbee devices.

## License

[MIT](LICENSE). The patch in `server/patches/` changes
[typesafe-assist](https://github.com/JanOstrowka/typesafe-assist), which keeps its own terms.
