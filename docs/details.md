# Details

What `server/setup.sh` does and why, how my devices get into Home Assistant (HA), and the
notes worth knowing when something breaks.

## What setup.sh does

1. Creates `.env` and fills in `TAILSCALE_IP` from `tailscale ip -4`. HA listens only on that
   address and on loopback, never on all addresses: Docker-published ports skip the firewall
   and would be public.
2. Creates `ha-config/secrets.yaml` from the example. Only the Jev AC automation uses it, and
   that's off by default.
3. Seeds HA's HTTP settings from `http-seed.json` (see [HTTP settings](#http-settings)).
4. Installs two custom integrations at commits I reviewed: Tuya IR Climate, and typesafe-assist
   with the fix from `patches/`. Read the diff before moving either to a newer commit.
5. Starts HA, whisper, piper and openWakeWord (`docker compose --profile voice up -d`).
6. Runs `setup/configure_ha.py` inside the HA container, which uses HA's API to:
   - create your account (or sign you in) and set up two-factor login with a QR code;
   - add whisper, piper and openWakeWord through the Wyoming integration;
   - if you use Tuya, wait while you add it in the web UI (its QR login needs your phone), then
     add Tuya IR Climate;
   - add DeepSeek as the fallback agent and Jev as the main agent, if you give it the keys;
   - create a voice assistant with all of the above and the "okay nabu" wake word, and make it
     the preferred one;
   - hide scenes from voice, so "turn off the fan" can't press a scene instead of the fan.

Every step checks what already exists, so run it again after adding Tuya or getting a key.

## Devices

Anything HA supports works: add its integration under Settings → Devices & services. These are
mine, which is why setup.sh has steps for Tuya.

| House | Device | In HA through |
|---|---|---|
| Home | IR air conditioner | Tuya IR Climate |
| Home | IR fan with DIY remote buttons | Smart Life scenes, wrapped by `packages/fans.yaml` |
| Home | PC RGB lighting | OpenRGB |
| Second house | IR air conditioner | Tuya IR Climate |
| Second house | Fan on a Tuya switch | not yet: its devices are offline until someone is there |

- **Tuya** (built in): add it in the web UI and scan its QR code with the Smart Life app.
  Devices and Tap-to-Run scenes from every home on the account appear.
- **[Tuya IR Climate](https://github.com/gabrieldutra/ha-tuya-ir-climate)** turns IR air
  conditioners, which the Tuya integration lists as unsupported, into climate entities. It reuses
  the Tuya integration's connection and the remotes set up in Smart Life.
- **IR fan with DIY buttons**: HA can't press learned buttons directly, so each one gets a
  Tap-to-Run scene in Smart Life that presses only that button: `Fan off` and `Set Fan 1` to
  `Set Fan 4`. `packages/fans.yaml` turns them into `fan.home_fan`, remembering the speed it last
  set. Reload the Tuya integration after adding scenes.
- **OpenRGB**: start OpenRGB's SDK server on the PC, listening on the PC's Tailscale address,
  then add the built-in OpenRGB integration with that address and port 6742. Each RGB device
  becomes a light.

## Remote access

- **Tailscale**: `http://<server>:8123` works from any device on the tailnet. If HA isn't back
  after the server reboots, Docker started before Tailscale had its address: run
  `docker compose up -d` again.
- **Cloudflare Tunnel** (optional, once two-factor login is on): with cloudflared on the server,
  point a public hostname at `http://127.0.0.1:8123`, then set it as the Internet URL under
  Settings → System → Network. Don't put Cloudflare Access in front of HA: it breaks the phone
  app. HA's own login protects it, and an IP that fails to log in 5 times is banned. To lift a
  ban, delete its line from `ha-config/ip_bans.yaml` and restart HA.

### HTTP settings

Since HA 2026.8 these live in `.storage/http`, not YAML. `http-seed.json` sets what the tunnel
needs: trust `X-Forwarded-For` from `172.31.0.1`, the compose network's gateway, where requests
through the published port come from. Without it, the tunnel gets `400 Bad Request`, and a login
ban would hit the tunnel itself instead of the visitor. It also bans an IP after 5 failed logins.
Check it with `docker exec homeassistant cat /config/.storage/http`: `stable` should show
`"use_x_forwarded_for": true` and `172.31.0.1/32`.

Don't put an `http:` block back in `configuration.yaml`: HA imports it as a trial that reverts
after 5 minutes unless an admin confirms it. Changes saved in Settings → System → Network are
trials too. To edit the file by hand, stop HA first.

## Voice

- **Jev** ([typesafe-assist](https://github.com/JanOstrowka/typesafe-assist)) is the main agent:
  model pinned to `jev-1.13.0`, cutoffs raised to 0.7 / 0.6, and the fallback agent behind it.
  Every command sends TypeSafe the text plus the names and areas of everything exposed to voice
  (and their states, if "include state" is on: leave it off). It acts without asking, so don't
  expose locks, garage doors or scripts that matter.
- **The fallback** is DeepSeek through the built-in llama.cpp integration (base URL
  `https://api.deepseek.com/v1`, a chat model rather than the reasoner). Claude through the
  Anthropic integration works too.
- **The voice** is piper's `en_US-lessac-medium`. Change it under Settings → Voice assistants →
  your assistant → Text-to-speech.
- **Wake word**: to add one by hand, open the assistant, choose ⋮ → Add streaming wake word, and
  pick openwakeword with "okay nabu".
- For mixed English and Malay commands, try `WHISPER_LANGUAGE=ms` in `.env` against `en`. To bias
  whisper toward your device names, enable the commented name biasing in `compose.yaml`.

## Desktop app

`desktop/` is Home, a native Rust app ([iced](https://iced.rs)). It talks only to HA's own APIs:
the login flow, the websocket API and Assist pipelines. It draws on the CPU (iced's tiny-skia
renderer): the GPU renderer would load every GPU driver into the app, about 95 MB, to draw a
window of buttons. It uses about 40 MB.

It shows every climate, fan, light and switch entity in HA, from any integration, grouped by
area. Other kinds of device are reachable by voice.

The look follows Apple's Home app and Home Assistant's Mushroom cards: dark tiles with an icon
that lights up in the device's colour when it's on (a light's own colour, for RGB lights),
segmented controls for modes and speeds, and room filters. It uses the Inter font and Lucide
icons, bundled in `desktop/assets/`. Entities whose integration no longer provides them (HA
shows them as "not provided") are hidden.

- Needs Rust and libpulse (PipeWire's pulse server works). Tested on KDE Plasma 6 (Wayland).
- First start: sign in with your HA username, password and authenticator code. The login is kept
  in KWallet (any Secret Service); Settings → Sign out revokes it.
- Talk with the button, the tray menu, or the shortcut Plasma asks for on first start (Ctrl+Alt+H
  by default; change it in System Settings → Keyboard → Shortcuts). HA hears when you stop talking.
- Settings: the assistant (HA's preferred one unless you pick another), the microphone
  (Bluetooth mics switch the device to call audio while in use), and "Always listen", which
  streams the mic to HA's wake word engine all the time. Always listen waits until HA has
  finished starting, so it never holds up HA's startup.
- Closing the window keeps it in the tray, and autostart starts it there.
- End-to-end check against a test HA with the `demo` integration, whisper, piper and
  openwakeword (login with MFA, every kind of button, typed, spoken and wake word commands):
  ```sh
  IOT_DESKTOP_MUTE=1 IOT_DESKTOP_TOKEN_FILE=/tmp/token \
    cargo run --example smoke -- <server> <user> <password> <totp-secret>
  ```

## Checking the config

```sh
cd server
docker run --rm -v "$PWD/ha-config":/config --entrypoint python \
  ghcr.io/home-assistant/home-assistant:2026.10.2 -m homeassistant --script check_config -c /config
```

A pass prints only "Testing configuration"; errors are listed with their file and line. Add
`--info all` to see "Successful config (all)" and everything that loaded.

## Jev tests

```sh
cd tests/jev && mkdir -p results
export TYPESAFE_API_KEY=...
uv run --no-project --with 'typesafe-sdk>=0.7.2' python jev_gate_test.py results/gate.json
```

`jev_voice_test.py` covers clean commands, `jev_garbled_test.py` misheard and nonsense
transcripts, and `jev_gate_test.py` all of them with a clear-command gate and a threshold
comparison.

## HACS

Not needed for anything here; setup.sh installs the custom integrations directly. HA 2026.11
ships it built in as the Marketplace. To add it to 2026.10:

```sh
docker exec homeassistant bash -c "wget -O - https://get.hacs.xyz | bash -"
docker restart homeassistant
```
