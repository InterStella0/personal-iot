# personal-iot

Home automation built on [Home Assistant](https://www.home-assistant.io) (HA) in Docker. HA talks to
the devices (Tuya today, Matter/Zigbee later); everything else (voice, Claude, the phone, Jev
automations) talks to HA, so no client code depends on a device brand.

HA runs on stella-server and reaches the devices in both houses through Tuya's cloud. A LAN-only
integration such as tuya-local would need an HA in each house.

```
server/
├── compose.yaml          HA, plus whisper + piper under the `voice` profile
├── .env.example
├── http-seed.json        HA's HTTP settings (tunnel proxy trust), copied in before the first start
└── ha-config/            mounted as /config
    ├── configuration.yaml
    ├── packages/         one YAML file per feature (tracked)
    │   ├── fans.yaml     the home fan as a fan entity (IR fan through scenes)
    │   └── jev_ac.yaml   Jev decides AC on/off + mode, dry run by default
    └── secrets.yaml.example
tests/jev/                Jev voice-routing tests (clean, garbled, gated)
```

UI-created config (`.storage/`), the history database, secrets and custom integrations are
gitignored. Anything worth versioning goes in `packages/*.yaml`.

## Phase 1: HA with the current devices

| House | Device | In HA through |
|---|---|---|
| Home | IR aircond | Tuya IR Climate |
| Home | IR fan with DIY buttons | Tap-to-Run scenes, wrapped by `packages/fans.yaml` |
| Second house | IR aircond | Tuya IR Climate |
| Second house | Fan on a Tuya switch | later: its devices are offline until someone is there |

1. Configure, on stella-server:
   ```sh
   gh repo clone InterStella0/personal-iot ~/personal-iot && cd ~/personal-iot/server
   cp .env.example .env
   cp ha-config/secrets.yaml.example ha-config/secrets.yaml   # Jev key, with the "Bearer " prefix
   mkdir -p ha-config/.storage && cp http-seed.json ha-config/.storage/http   # see HTTP settings
   ```
2. Start HA:
   ```sh
   docker compose up -d
   ```
   Open `http://stella-server:8123` from a device on the tailnet. HA only listens on the server's
   loopback (for cloudflared) and its Tailscale address. If the server reboots and HA isn't
   back, Docker started before Tailscale had the address: run `docker compose up -d` again.
3. Create the owner account and **turn on multi-factor auth** (Profile → Security) before the
   tunnel below goes live.
4. Install [HACS](https://hacs.xyz/docs/use/download/download/) (Container install), restart, then
   add the HACS integration, which logs in to GitHub with a device code. HA 2026.11 ships HACS
   built in as the Marketplace, so skip this after upgrading.
   ```sh
   docker exec homeassistant bash -c "wget -O - https://get.hacs.xyz | bash -"
   docker restart homeassistant
   ```
5. Add the devices:
   - **Tuya** integration (built in): scan its QR code with the Smart Life app. Devices and
     Tap-to-Run scenes from both homes appear.
   - **[Tuya IR Climate](https://github.com/gabrieldutra/ha-tuya-ir-climate)** for the airconds,
     which the Tuya integration lists as unsupported. It reuses the Tuya integration's connection,
     and the IR codes come from the remotes set up in Smart Life. Install the reviewed commit, then
     add it under Settings → Devices & services → Add integration. Read the diff before moving
     to a newer commit.
     ```sh
     SHA=9f3516122232b3cc2eef42bace3187f3a8ec4858
     mkdir -p ha-config/custom_components
     curl -fsSL https://github.com/gabrieldutra/ha-tuya-ir-climate/archive/$SHA.tar.gz | tar -xz \
       -C ha-config/custom_components --strip-components=2 \
       ha-tuya-ir-climate-$SHA/custom_components/tuya_ir_climate
     docker restart homeassistant
     ```
   - **Home fan**: in Smart Life, create five Tap-to-Run scenes that each press one DIY button,
     named `Fan off` and `Set Fan 1` to `Set Fan 4`. `packages/fans.yaml` turns them into
     `fan.home_fan`. Scenes added after the Tuya integration need it reloaded.

**Done when** every current device can be controlled from HA's web UI.

### Cloudflare Tunnel

cloudflared already runs on stella-server (a systemd service, configured from the Cloudflare
dashboard). Once multi-factor auth is on, add a public hostname to the tunnel pointing at
`http://127.0.0.1:8123`. Don't put Cloudflare Access in front of HA: it breaks the Companion app
and Claude's MCP connector. Use HA's own login with MFA.

### HTTP settings

Since HA 2026.8 these live in `.storage/http`, not YAML. `http-seed.json` sets what the tunnel
needs: trust `X-Forwarded-For` from `172.31.0.1`, the compose network's gateway, which is where
requests through the published port come from. Without it, the tunnel gets `400 Bad Request`.
Check it with `docker exec homeassistant cat /config/.storage/http`: `stable` should show
`"use_x_forwarded_for": true` and `172.31.0.1/32`.

Don't put an `http:` block back in `configuration.yaml`: HA imports it as a trial that reverts
after 5 minutes unless an admin confirms it. Changes saved in Settings → System → Network are
trials too, confirmed in the UI. To edit the file by hand, stop HA first.

### Checking the config

```sh
docker run --rm -v "$PWD/ha-config":/config --entrypoint python \
  ghcr.io/home-assistant/home-assistant:2026.10.2 -m homeassistant --script check_config -c /config
```

A pass prints only "Testing configuration"; errors are listed with their file and line. Add
`--info all` to see "Successful config (all)" and everything that loaded.

## Phase 2: voice

- `docker compose --profile voice up -d`, then add whisper (host `whisper`, port 10300) and piper
  (`piper`, 10200) through HA's Wyoming integration. Try `WHISPER_LANGUAGE=ms` against `en` for
  Manglish commands, and enable the commented HA name biasing in `compose.yaml`.
- [linux-voice-assistant](https://github.com/OHF-Voice/linux-voice-assistant) on the desktop as the
  always-listening microphone.
- Anthropic integration (API key, Claude Haiku 5.5, thinking effort low) as the Claude agent.
- [typesafe-assist](https://github.com/JanOstrowka/typesafe-assist) (Jev) as the main agent, with the
  Claude agent as its fallback, model pinned to `jev-1.13.0`, cutoffs raised to about 0.7 / 0.6.
- Model Context Protocol Server integration, for Claude Desktop / Claude Code over the tunnel.
- HA Companion app on the phone, through the tunnel hostname, or `http://stella-server:8123` for
  access over Tailscale only.
- Record 20–30 real commands and run them through `tests/jev`.

## Phase 3: decide from real voice data

Keep typesafe-assist if it copes with real transcripts. Otherwise build `services/voice-router`
behind [webhook-conversation](https://github.com/eulemitkeule/webhook-conversation), using the
clear-command gate from `tests/jev/jev_gate_test.py`. Switch `packages/jev_ac.yaml` from dry run to
live once its logbook decisions look right.

## Phase 4 (optional)

Custom Tauri desktop app, Google Home, Matter/Zigbee devices, retiring the old Discord bot.

## Jev tests

```sh
cd tests/jev && mkdir -p results
export TYPESAFE_API_KEY=...
uv run --no-project --with 'typesafe-sdk>=0.7.2' python jev_gate_test.py results/gate.json
```

`jev_voice_test.py` covers clean commands, `jev_garbled_test.py` mis-heard and nonsense transcripts,
`jev_gate_test.py` all of them with the clear-command gate and a threshold comparison.
