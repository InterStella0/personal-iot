#!/usr/bin/env bash
# Sets up the server: config files, the pinned custom integrations, the containers, and then
# Home Assistant itself through its API (setup/configure_ha.py). Safe to run again: every step
# skips what's already done.
set -euo pipefail
cd "$(dirname "$0")"

TUYA_IR_CLIMATE_SHA=9f3516122232b3cc2eef42bace3187f3a8ec4858
TYPESAFE_ASSIST_SHA=7a771aab9933df2b7b7d57eb8fdb3f40f634ac12

say() { printf '\n== %s\n' "$*"; }
need() { command -v "$1" >/dev/null || { echo "Missing: $1. Install it and run this again." >&2; exit 1; }; }
need docker
need curl
need tar
need patch
docker compose version >/dev/null 2>&1 || { echo "Missing: the Docker Compose plugin." >&2; exit 1; }

say "Config files"
[ -f .env ] || { cp .env.example .env; echo "created .env"; }
if ! grep -q '^TAILSCALE_IP=.' .env; then
    ip=$(tailscale ip -4 2>/dev/null | head -n1 || true)
    if [ -z "$ip" ]; then
        read -rp "This server's Tailscale IPv4 address (tailscale ip -4): " ip
    fi
    sed -i '/^TAILSCALE_IP=/d' .env
    echo "TAILSCALE_IP=$ip" >> .env
    echo "HA will listen on 127.0.0.1 and $ip"
fi
if [ ! -f ha-config/secrets.yaml ]; then
    cp ha-config/secrets.yaml.example ha-config/secrets.yaml
    chmod 600 ha-config/secrets.yaml
    echo "created ha-config/secrets.yaml (only the Jev AC automation uses it; it's off by default)"
fi
if [ ! -f ha-config/.storage/http ]; then
    mkdir -p ha-config/.storage
    cp http-seed.json ha-config/.storage/http
    echo "seeded HA's HTTP settings (tunnel proxy trust, login bans)"
fi

say "Custom integrations (pinned commits)"
mkdir -p ha-config/custom_components
install_component() { # repo sha component
    local target="ha-config/custom_components/$3"
    if [ -d "$target" ]; then
        echo "already installed: $3"
        return 1
    fi
    # set -e doesn't apply inside a function called from `if`/`||`, so check by hand.
    if ! curl -fsSL "https://github.com/$1/archive/$2.tar.gz" |
        tar -xz -C ha-config/custom_components --strip-components=2 \
            "${1#*/}-$2/custom_components/$3"; then
        echo "Couldn't download $3 from github.com/$1." >&2
        exit 1
    fi
    echo "installed: $3 at ${2:0:7}"
    installed=1
}
installed=0
install_component gabrieldutra/ha-tuya-ir-climate "$TUYA_IR_CLIMATE_SHA" tuya_ir_climate || true
if install_component JanOstrowka/typesafe-assist "$TYPESAFE_ASSIST_SHA" typesafe_conversation; then
    patch -d ha-config/custom_components/typesafe_conversation -p1 \
        < patches/typesafe_conversation-ha-2026.10.patch
fi

say "Containers"
docker compose --profile voice up -d
# A running HA only loads newly installed integrations after a restart.
if [ "$installed" = 1 ]; then
    docker restart homeassistant >/dev/null
fi

say "Home Assistant"
docker cp setup/configure_ha.py homeassistant:/tmp/configure_ha.py
if [ -t 0 ]; then tty=-it; else tty=-i; fi
docker exec "$tty" homeassistant python3 /tmp/configure_ha.py

cat <<'EOF'

== Left for you
- The fan's remote buttons, if you have an IR fan with learned buttons: in Smart Life, make one
  Tap-to-Run scene per button (Fan off, Set Fan 1 … Set Fan 4), then reload the Tuya integration.
- The phone: install the Home Assistant app and sign in with http://<this server>:8123 on your
  tailnet, or your Cloudflare hostname (see the README).
- The PC: cd desktop && ./install.sh
EOF
