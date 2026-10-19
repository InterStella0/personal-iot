# personal-iot

Home automation built on [Home Assistant](https://www.home-assistant.io) (HA) in Docker. HA talks to
the devices (Tuya today, Matter/Zigbee later); everything else (voice, Claude, the phone, Jev
automations) talks to HA, so no client code depends on a device brand.

```
server/
├── compose.yaml          HA, plus whisper + piper under the `voice` profile
├── .env.example
└── ha-config/            mounted as /config
    ├── configuration.yaml
    ├── packages/         one YAML file per feature (tracked)
    │   └── jev_ac.yaml   Jev decides AC on/off + mode, dry run by default
    └── secrets.yaml.example
tests/jev/                Jev voice-routing tests (clean, garbled, gated)
```

UI-created config (`.storage/`), the history database, secrets and custom integrations are
gitignored. Anything worth versioning goes in `packages/*.yaml`.

## Phase 1: HA with the current devices

1. Configure:
   ```sh
   cd server
   cp .env.example .env
   cp ha-config/secrets.yaml.example ha-config/secrets.yaml   # Jev key, with the "Bearer " prefix
   ```
2. Start HA and open `http://<server-tailscale-ip>:8123`:
   ```sh
   docker compose up -d
   ```
3. Create the owner account and **turn on multi-factor auth** (Profile → Security), since HA is
   reachable from the internet through the tunnel.
4. Install [HACS](https://hacs.xyz/docs/use/download/download/) (Container install).
5. Add the devices:
   - **Tuya** integration (built in) for cloud control.
   - **[tuya-local](https://github.com/make-all/tuya-local)** from HACS for LAN control. The IR blaster
     appears as a `remote` entity: learn each button with `remote.learn_command`.
6. Wrap the learned IR buttons as scripts plus a template fan / AC in `packages/`, so voice, Claude and
   Google see devices instead of buttons.

**Done when** every current device can be controlled from HA's web UI.

### Cloudflare Tunnel

Point a public hostname at `http://localhost:8123` (cloudflared on this host). If cloudflared runs
elsewhere, add its address to `http.trusted_proxies` in `configuration.yaml`, otherwise HA answers
`400 Bad Request`. Don't put Cloudflare Access in front of HA: it breaks the Companion app and
Claude's MCP connector. Use HA's own login with MFA, and Tailscale for personal devices.

### Checking the config

```sh
docker run --rm -v "$PWD/ha-config":/config --entrypoint python \
  ghcr.io/home-assistant/home-assistant:2026.10.2 -m homeassistant --script check_config -c /config
```

## Phase 2: voice

- `docker compose --profile voice up -d`, then add whisper (port 10300) and piper (10200) through
  HA's Wyoming integration. Try `WHISPER_LANGUAGE=ms` against `en` for Manglish commands, and enable
  the commented HA name biasing in `compose.yaml`.
- [linux-voice-assistant](https://github.com/OHF-Voice/linux-voice-assistant) on the desktop as the
  always-listening microphone.
- Anthropic integration (API key, Claude Haiku 5.5, thinking effort low) as the Claude agent.
- [typesafe-assist](https://github.com/JanOstrowka/typesafe-assist) (Jev) as the main agent, with the
  Claude agent as its fallback, model pinned to `jev-1.13.0`, cutoffs raised to about 0.7 / 0.6.
- Model Context Protocol Server integration, for Claude Desktop / Claude Code over the tunnel.
- HA Companion app on the phone, over Tailscale.
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
