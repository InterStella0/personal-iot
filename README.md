# personal-iot

My home automation, built around [Home Assistant](https://www.home-assistant.io) (HA) in Docker:

- **One HA for two houses.** It runs on a server and reaches the devices through Tuya's cloud, so
  no house needs its own hub.
- **IR devices as real devices.** The air conditioners are climate entities (temperature, mode,
  fan speed), and an IR fan with learned remote buttons is a proper fan with speeds 1–4.
- **Local voice.** Speech-to-text (whisper), text-to-speech (piper) and wake word detection
  (openWakeWord) run next to HA. Commands go to Jev, with an LLM as its fallback.
- **Home, a native desktop app.** Rust, no browser: device cards, a Talk button and global
  shortcut, an optional wake word, and a tray icon.
- **Nothing public by default.** HA listens only on the server's loopback and its Tailscale
  address. A Cloudflare Tunnel is optional.

Everything talks to HA, and only HA talks to devices, so no client depends on a device brand.

```
 Smart Life devices ── Tuya cloud ──┐
 PC lights (OpenRGB) ───────────────┤
                                    ▼
              Home Assistant (Docker, on the server)
               ├─ whisper · piper · openWakeWord (Wyoming)
               └─ Jev (typesafe-assist), LLM fallback
                                    ▲
 Phone (Companion app) ─────────────┤  Tailscale, or a
 PC (Home desktop app) ─────────────┘  Cloudflare Tunnel
```

## Repository

```
server/
├── compose.yaml          HA, plus whisper, piper and openwakeword (`voice` profile)
├── .env.example
├── http-seed.json        HA's HTTP settings (tunnel proxy trust), copied in before the first start
├── patches/              fixes for third-party integrations
└── ha-config/            mounted as /config
    ├── configuration.yaml
    ├── packages/         one YAML file per feature
    │   ├── fans.yaml     an IR fan with DIY remote buttons as a fan entity
    │   └── jev_ac.yaml   Jev decides AC on/off and mode, dry run by default
    └── secrets.yaml.example
desktop/                  Home, the desktop app (Rust, iced)
tests/jev/                Jev voice-routing tests (clean, garbled, gated)
```

HA's UI-created config (`.storage/`), its database, secrets and custom integrations are
gitignored. Anything worth versioning goes in `packages/*.yaml`.

## Server setup

You need a Linux server with Docker and [Tailscale](https://tailscale.com), and a Smart Life
(Tuya) account for the devices.

1. Clone and configure:
   ```sh
   git clone https://github.com/InterStella0/personal-iot.git ~/personal-iot
   cd ~/personal-iot/server
   cp .env.example .env
   cp ha-config/secrets.yaml.example ha-config/secrets.yaml   # TypeSafe key, with "Bearer "
   mkdir -p ha-config/.storage && cp http-seed.json ha-config/.storage/http   # see HTTP settings
   ```
   In `compose.yaml`, replace `100.83.100.35` with your server's Tailscale address
   (`tailscale ip -4`).
2. Start HA:
   ```sh
   docker compose up -d
   ```
   Open `http://<server>:8123` from a device on your tailnet. If HA isn't back after the server
   reboots, Docker started before Tailscale had its address: run `docker compose up -d` again.
3. Create the owner account and turn on multi-factor auth (Profile → Security).
4. Install [HACS](https://hacs.xyz/docs/use/download/download/), restart, then add the HACS
   integration, which logs in to GitHub with a device code. HA 2026.11 ships HACS built in as the
   Marketplace, so skip this after upgrading.
   ```sh
   docker exec homeassistant bash -c "wget -O - https://get.hacs.xyz | bash -"
   docker restart homeassistant
   ```

### Checking the config

```sh
docker run --rm -v "$PWD/ha-config":/config --entrypoint python \
  ghcr.io/home-assistant/home-assistant:2026.10.2 -m homeassistant --script check_config -c /config
```

A pass prints only "Testing configuration"; errors are listed with their file and line. Add
`--info all` to see "Successful config (all)" and everything that loaded.

## Devices

| House | Device | In HA through |
|---|---|---|
| Home | IR air conditioner | Tuya IR Climate |
| Home | IR fan with DIY remote buttons | Smart Life scenes, wrapped by `packages/fans.yaml` |
| Home | PC RGB lighting | OpenRGB |
| Second house | IR air conditioner | Tuya IR Climate |
| Second house | Fan on a Tuya switch | not yet: its devices are offline until someone is there |

- **Tuya** (built in): add the integration and scan its QR code with the Smart Life app. Devices
  and Tap-to-Run scenes from every home on the account appear.
- **[Tuya IR Climate](https://github.com/gabrieldutra/ha-tuya-ir-climate)** turns IR air
  conditioners, which the Tuya integration lists as unsupported, into climate entities. It reuses
  the Tuya integration's connection and the remotes set up in Smart Life. Install the commit I
  reviewed, then add it under Settings → Devices & services. Read the diff before moving to a
  newer one.
  ```sh
  SHA=9f3516122232b3cc2eef42bace3187f3a8ec4858
  mkdir -p ha-config/custom_components
  curl -fsSL https://github.com/gabrieldutra/ha-tuya-ir-climate/archive/$SHA.tar.gz | tar -xz \
    -C ha-config/custom_components --strip-components=2 \
    ha-tuya-ir-climate-$SHA/custom_components/tuya_ir_climate
  docker restart homeassistant
  ```
- **IR fan with DIY buttons**: HA can't press learned buttons directly, so each one gets a
  Tap-to-Run scene in Smart Life that presses only that button: `Fan off` and `Set Fan 1` to
  `Set Fan 4`. `packages/fans.yaml` turns them into `fan.home_fan`, remembering the speed it last
  set. Reload the Tuya integration after adding scenes.
- **OpenRGB**: start OpenRGB's SDK server on the PC, listening on the PC's Tailscale address,
  then add the built-in OpenRGB integration with that address and port 6742. Each RGB device
  becomes a light.

## Remote access

- **Tailscale**: `http://<server>:8123` works from any device on the tailnet. This is how the
  phone and the desktop app connect.
- **Cloudflare Tunnel** (optional, once MFA is on): with cloudflared on the server, point a public
  hostname at `http://127.0.0.1:8123`. Don't put Cloudflare Access in front of HA: it breaks the
  Companion app. Use HA's own login with MFA.

### HTTP settings

Since HA 2026.8 these live in `.storage/http`, not YAML. `http-seed.json` sets what the tunnel
needs: trust `X-Forwarded-For` from `172.31.0.1`, the compose network's gateway, where requests
through the published port come from. Without it, the tunnel gets `400 Bad Request`. Check it with
`docker exec homeassistant cat /config/.storage/http`: `stable` should show
`"use_x_forwarded_for": true` and `172.31.0.1/32`.

Don't put an `http:` block back in `configuration.yaml`: HA imports it as a trial that reverts
after 5 minutes unless an admin confirms it. Changes saved in Settings → System → Network are
trials too. To edit the file by hand, stop HA first.

## Voice

1. Start the speech services and add each through HA's Wyoming integration:
   ```sh
   docker compose --profile voice up -d
   ```
   whisper (host `whisper`, port 10300), piper (`piper`, 10200), openwakeword (`openwakeword`,
   10400).
2. Add a fallback agent. I use DeepSeek through the built-in llama.cpp integration (base URL
   `https://api.deepseek.com/v1`, a chat model rather than the reasoner). Claude through the
   Anthropic integration works too.
3. Add [typesafe-assist](https://github.com/JanOstrowka/typesafe-assist) (Jev) as the main agent.
   Install v0.2.1 with the HA 2026.10 fix from `patches/`, restart, then add it with your TypeSafe
   key. In its options, pin the model to `jev-1.13.0`, raise the cutoffs to about 0.7 / 0.6, and
   pick the fallback agent.
   ```sh
   SHA=7a771aab9933df2b7b7d57eb8fdb3f40f634ac12
   curl -fsSL https://github.com/JanOstrowka/typesafe-assist/archive/$SHA.tar.gz | tar -xz \
     -C ha-config/custom_components --strip-components=2 \
     typesafe-assist-$SHA/custom_components/typesafe_conversation
   patch -d ha-config/custom_components/typesafe_conversation -p1 \
     < patches/typesafe_conversation-ha-2026.10.patch
   docker restart homeassistant
   ```
   Every command sends TypeSafe the text plus the names and areas of everything exposed to Assist
   (and their states, if "include state" is on: leave it off). It acts without asking for
   confirmation, so don't expose locks, garage doors or scripts that matter.
4. Create an assistant (Settings → Voice assistants) with Jev, whisper and piper, and set it as
   preferred. For the wake word, open it, choose ⋮ → Add streaming wake word, and pick
   openwakeword with "okay nabu".
5. Expose only what voice should control. Hide helper scenes like the fan's, or "turn off the fan"
   may press a scene instead of the fan.

For mixed English and Malay commands, try `WHISPER_LANGUAGE=ms` against `en`. To bias whisper
toward your device names, enable the commented name biasing in `compose.yaml`.

## Desktop app

`desktop/` is Home, a native Rust app ([iced](https://iced.rs)) for the PC: device cards grouped by
area, a Talk button and global shortcut, typed commands, an optional wake word, and a tray icon.
It talks only to HA's own APIs: the login flow, the websocket API and Assist pipelines.

```sh
cd desktop && ./install.sh   # builds, installs ~/.local/bin/iot-desktop, app menu + autostart
```

- Needs Rust and libpulse (PipeWire's pulse server works). Tested on KDE Plasma 6 (Wayland).
- First start: sign in with your HA username, password and authenticator code. The login is kept
  in KWallet (any Secret Service); Settings → Sign out revokes it.
- Talk with the button, the tray menu, or the shortcut Plasma asks for on first start (Ctrl+Alt+H
  by default; change it in System Settings → Keyboard → Shortcuts). HA hears when you stop talking.
- Settings: the microphone (Bluetooth mics switch the device to call audio while in use), and
  "Always listen", which streams the mic to HA's wake word engine all the time.
- Closing the window keeps it in the tray, and autostart starts it there.
- End-to-end check against a test HA with the `demo` integration, whisper, piper and
  openwakeword (login with MFA, every kind of button, typed, spoken and wake word commands):
  ```sh
  IOT_DESKTOP_MUTE=1 IOT_DESKTOP_TOKEN_FILE=/tmp/token \
    cargo run --example smoke -- <server> <user> <password> <totp-secret>
  ```

## Jev tests

```sh
cd tests/jev && mkdir -p results
export TYPESAFE_API_KEY=...
uv run --no-project --with 'typesafe-sdk>=0.7.2' python jev_gate_test.py results/gate.json
```

`jev_voice_test.py` covers clean commands, `jev_garbled_test.py` misheard and nonsense
transcripts, and `jev_gate_test.py` all of them with a clear-command gate and a threshold
comparison.

## Roadmap

- Record 20–30 real commands and run them through `tests/jev`. Keep Jev if it copes with real
  transcripts; otherwise build a voice router behind
  [webhook-conversation](https://github.com/eulemitkeule/webhook-conversation) using the gate from
  `jev_gate_test.py`.
- Switch `packages/jev_ac.yaml` from dry run to live once its logbook decisions look right.
- The Model Context Protocol Server integration, for Claude Desktop and Claude Code.
- The second house's fan, Google Home, and Matter/Zigbee devices.

## License

[MIT](LICENSE). The patch in `server/patches/` changes
[typesafe-assist](https://github.com/JanOstrowka/typesafe-assist), which keeps its own terms.
