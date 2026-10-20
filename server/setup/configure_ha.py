"""Configures Home Assistant through its API, so setup needs as few clicks as possible.

setup.sh copies this into the HA container and runs it there, because the container already has
Python, aiohttp, pyotp and pyqrcode. Every step checks what exists first, so it's safe to rerun.
"""

import asyncio
import base64
import getpass
import hashlib
import json
import secrets
import sys

import aiohttp
import pyotp
import pyqrcode

BASE = "http://127.0.0.1:8123"
CLIENT_ID = "http://127.0.0.1/personal-iot-setup"
REDIRECT_URI = CLIENT_ID + "/callback"
WYOMING = [("whisper", 10300), ("piper", 10200), ("openwakeword", 10400)]
DEEPSEEK_URL = "https://api.deepseek.com/v1"
VOICE = "en_US-lessac-medium"


def say(text=""):
    print(text, flush=True)


def step(title):
    say(f"\n== {title}")


def ask(prompt, secret=False, default=""):
    try:
        answer = (getpass.getpass if secret else input)(prompt)
    except EOFError:
        sys.exit("\nNo input. Run setup.sh in a terminal.")
    return answer.strip() or default


def yes(prompt, default=True):
    answer = ask(f"{prompt} [{'Y/n' if default else 'y/N'}] ").lower()
    return default if not answer else answer.startswith("y")


class HA:
    def __init__(self, http):
        self.http = http
        self.token = None
        self.ws = None
        self.ws_id = 0

    @property
    def auth(self):
        return {"Authorization": f"Bearer {self.token}"}

    async def get(self, path):
        async with self.http.get(BASE + path, headers=self.auth) as r:
            r.raise_for_status()
            return await r.json()

    async def post(self, path, body=None, auth=True):
        async with self.http.post(BASE + path, json=body or {}, headers=self.auth if auth else {}) as r:
            text = await r.text()
            if r.status >= 400:
                raise RuntimeError(f"{path}: HTTP {r.status}: {text[:300]}")
            return json.loads(text) if text else {}

    async def call(self, message):
        if self.ws is None:
            self.ws = await self.http.ws_connect(BASE + "/api/websocket")
            await self.ws.receive_json()
            await self.ws.send_json({"type": "auth", "access_token": self.token})
            if (await self.ws.receive_json())["type"] != "auth_ok":
                raise RuntimeError("websocket login failed")
        self.ws_id += 1
        await self.ws.send_json({"id": self.ws_id, **message})
        while True:
            reply = await self.ws.receive_json()
            if reply.get("id") == self.ws_id and reply.get("type") == "result":
                if not reply["success"]:
                    raise RuntimeError(reply["error"]["message"])
                return reply["result"]

    async def entries(self, domain=None):
        entries = await self.get("/api/config/config_entries/entry")
        return [e for e in entries if domain is None or e["domain"] == domain]

    async def states(self, domain):
        return [s for s in await self.get("/api/states") if s["entity_id"].startswith(domain + ".")]

    async def flow(self, handler, *answers):
        """Runs a config flow, answering each form in turn. Returns the final result."""
        result = await self.post("/api/config/config_entries/flow", {"handler": handler})
        for answer in answers:
            if result.get("type") != "form":
                break
            if result.get("errors"):
                raise RuntimeError(f"{handler} refused it: {result['errors']}")
            if callable(answer):
                answer = answer(result)
            result = await self.post(f"/api/config/config_entries/flow/{result['flow_id']}", answer)
        if result.get("type") == "form" and result.get("errors"):
            raise RuntimeError(f"{handler} refused it: {result['errors']}")
        return result

    async def options(self, entry_id, answer):
        result = await self.post("/api/config/config_entries/options/flow", {"handler": entry_id})
        return await self.post(f"/api/config/config_entries/options/flow/{result['flow_id']}", answer)


async def wait_for_ha(http):
    for _ in range(180):
        try:
            async with http.get(BASE + "/api/onboarding") as r:
                if r.status < 500:
                    return
        except aiohttp.ClientError:
            pass
        await asyncio.sleep(1)
    sys.exit("Home Assistant didn't start. Check: docker logs homeassistant")


async def sign_in(ha):
    async with ha.http.get(BASE + "/api/onboarding") as r:
        onboarding = await r.json() if r.status == 200 else []
    user_done = any(s["step"] == "user" and s["done"] for s in onboarding) if onboarding else True

    if not user_done:
        step("Create your Home Assistant account")
        name = ask("Your name: ", default="Owner")
        username = ask("Username: ", default=name.lower().split()[0])
        while True:
            password = ask("Password: ", secret=True)
            if len(password) >= 8 and password == ask("Again: ", secret=True):
                break
            say("They didn't match, or it's shorter than 8 characters. Try again.")
        result = await ha.post("/api/onboarding/users", {
            "client_id": CLIENT_ID, "name": name, "username": username,
            "password": password, "language": "en",
        }, auth=False)
        await exchange(ha, result["auth_code"], None)
        for part, body in (("core_config", {}), ("analytics", {}),
                           ("integration", {"client_id": CLIENT_ID, "redirect_uri": REDIRECT_URI})):
            await ha.post(f"/api/onboarding/{part}", body)
        say("  done: account created")
        return

    step("Sign in to Home Assistant")
    verifier = base64.urlsafe_b64encode(secrets.token_bytes(32)).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    while True:
        flow = await ha.post("/auth/login_flow", {
            "client_id": CLIENT_ID, "redirect_uri": REDIRECT_URI, "handler": ["homeassistant", None],
            "type": "authorize", "code_challenge": challenge, "code_challenge_method": "S256",
        }, auth=False)
        result = await ha.post(f"/auth/login_flow/{flow['flow_id']}", {
            "client_id": CLIENT_ID, "username": ask("Username: "), "password": ask("Password: ", secret=True),
        }, auth=False)
        while result.get("type") == "form" and result.get("step_id") in ("mfa", "select_mfa_module"):
            if result["step_id"] == "select_mfa_module":
                answer = {"multi_factor_auth_module": "totp"}
            else:
                answer = {"code": ask("6-digit code from your authenticator app: ")}
            result = await ha.post(f"/auth/login_flow/{flow['flow_id']}",
                                   {"client_id": CLIENT_ID, **answer}, auth=False)
        if result.get("type") == "create_entry":
            await exchange(ha, result["result"], verifier)
            say("  done: signed in")
            return
        say(f"  That didn't work ({result.get('errors') or result.get('reason')}). Try again.")


async def exchange(ha, code, verifier):
    form = {"grant_type": "authorization_code", "code": code, "client_id": CLIENT_ID}
    if verifier:
        form["code_verifier"] = verifier
    async with ha.http.post(BASE + "/auth/token", data=form) as r:
        ha.token = (await r.json())["access_token"]


async def speech_services(ha):
    step("Speech services (whisper, piper, openWakeWord)")
    titles = " ".join(e["title"].lower() for e in await ha.entries("wyoming"))
    for host, port in WYOMING:
        if host in titles:
            say(f"  already added: {host}")
            continue
        for attempt in range(12):
            try:
                result = await ha.flow("wyoming", {"host": host, "port": port})
                if result.get("type") in ("create_entry", "abort"):
                    say(f"  added: {host}")
                    break
            except RuntimeError as err:
                if attempt == 11:
                    say(f"  couldn't add {host}: {err}")
            # Whisper downloads its model on first start, so give it time.
            await asyncio.sleep(10)


async def tuya(ha):
    step("Tuya devices (type skip if you don't use Tuya)")
    if not await ha.entries("tuya"):
        say("  This one needs your phone, so add it in Home Assistant's web UI:")
        say("  Settings → Devices & services → Add integration → Tuya. Enter the user code from")
        say("  Smart Life (Me → Settings → Account and Security → User Code), then scan the QR")
        say("  code with Smart Life.")
        while not await ha.entries("tuya"):
            if ask("  Press Enter when it's added, or type skip: ").lower() == "skip":
                say("  skipped: Tuya, and Tuya IR Climate with it. Rerun setup.sh after adding Tuya.")
                return
    else:
        say("  already added: Tuya")
    if await ha.entries("tuya_ir_climate"):
        say("  already added: Tuya IR Climate")
        return
    result = await ha.flow("tuya_ir_climate", {})
    say(f"  added: Tuya IR Climate ({result.get('type')})")


async def fallback_agent(ha):
    step("Fallback AI agent (DeepSeek)")
    if not await ha.entries("llama_cpp") and not await ha.entries("anthropic"):
        key = ask("  DeepSeek API key (Enter to skip): ", secret=True)
        if key:
            def pick_model(form):
                options = []
                for field in form.get("data_schema", []):
                    for option in field.get("options", []) or []:
                        options.append(option[0] if isinstance(option, list) else option.get("value", option))
                    if field.get("selector", {}).get("select"):
                        for option in field["selector"]["select"].get("options", []):
                            options.append(option["value"] if isinstance(option, dict) else option)
                chat = [o for o in options if "reason" not in o and ("chat" in o or "flash" in o)]
                return {"chat_model": (chat or options or ["deepseek-chat"])[0]}

            try:
                await ha.flow("llama_cpp", {"base_url": DEEPSEEK_URL, "api_key": key}, pick_model)
                say("  added: DeepSeek")
            except RuntimeError as err:
                say(f"  couldn't add DeepSeek: {err}")
    else:
        say("  already added")
    agents = [s["entity_id"] for s in await ha.states("conversation")
              if s["entity_id"] not in ("conversation.home_assistant",) and "jev" not in s["entity_id"]]
    return agents[0] if agents else None


async def jev(ha, fallback):
    step("Jev (TypeSafe) as the main agent")
    entries = await ha.entries("typesafe_conversation")
    if not entries:
        key = ask("  TypeSafe API key (Enter to skip): ", secret=True)
        if not key:
            say("  skipped: Jev")
            return None
        try:
            await ha.flow("typesafe_conversation", {"api_key": key, "name": "Jev"})
        except RuntimeError as err:
            say(f"  couldn't add Jev: {err}")
            return None
        entries = await ha.entries("typesafe_conversation")
        say("  added: Jev")
    else:
        say("  already added: Jev")
    options = {"model": "jev-1.13.0", "intent_confidence": 0.7, "target_confidence": 0.6,
               "include_state": False, "timeout": 8}
    if fallback:
        options["fallback_agent"] = fallback
    await ha.options(entries[0]["entry_id"], options)
    say(f"  set: model jev-1.13.0, cutoffs 0.7 / 0.6, fallback {fallback or 'none'}")
    agents = [s["entity_id"] for s in await ha.states("conversation") if "jev" in s["entity_id"]]
    return agents[0] if agents else None


async def assistant(ha, agent):
    step("Voice assistant")
    states = await ha.get("/api/states")

    def first(domain, prefer=""):
        ids = [s["entity_id"] for s in states if s["entity_id"].startswith(domain + ".")]
        return next((i for i in ids if prefer in i), ids[0] if ids else None)

    stt, tts, wake = first("stt", "whisper"), first("tts", "piper"), first("wake_word")
    wake_id = None
    if wake:
        words = [w["id"] for w in (await ha.call({"type": "wake_word/info", "entity_id": wake}))["wake_words"]]
        wake_id = next((w for w in words if "nabu" in w), words[0] if words else None)

    listing = await ha.call({"type": "assist_pipeline/pipeline/list"})
    engine = agent or "conversation.home_assistant"
    existing = next((p for p in listing["pipelines"] if p["conversation_engine"] == engine
                     and engine != "conversation.home_assistant"), None)
    existing = existing or next((p for p in listing["pipelines"] if p["name"] == "Voice"), None)
    wanted = {
        "name": existing["name"] if existing else "Voice",
        "language": "en", "conversation_engine": engine, "conversation_language": "en",
        "stt_engine": stt, "stt_language": "en",
        "tts_engine": tts, "tts_language": "en_US", "tts_voice": VOICE if tts else None,
        "wake_word_entity": wake, "wake_word_id": wake_id, "prefer_local_intents": False,
    }
    if existing:
        # Keep what's already chosen (e.g. a different voice); fill in what's missing.
        update = {k: existing.get(k) if existing.get(k) not in (None, "") else v for k, v in wanted.items()}
        update["conversation_engine"] = engine
        await ha.call({"type": "assist_pipeline/pipeline/update", "pipeline_id": existing["id"], **update})
        pipeline_id, verb = existing["id"], "updated"
    else:
        created = await ha.call({"type": "assist_pipeline/pipeline/create", **wanted})
        pipeline_id, verb = created["id"], "created"
    await ha.call({"type": "assist_pipeline/pipeline/set_preferred", "pipeline_id": pipeline_id})
    say(f"  {verb} and preferred: \"{wanted['name']}\" ({engine}, {stt}, {tts}, wake word {wake_id})")

    scenes = [s["entity_id"] for s in states if s["entity_id"].startswith("scene.")]
    if scenes:
        await ha.call({"type": "homeassistant/expose_entity", "assistants": ["conversation"],
                       "entity_ids": scenes, "should_expose": False})
        say(f"  hid {len(scenes)} scene(s) from voice, so \"turn off the fan\" means the fan")


async def two_factor(ha):
    step("Two-factor login")
    user = await ha.call({"type": "auth/current_user"})
    if any(m["id"] == "totp" and m["enabled"] for m in user.get("mfa_modules", [])):
        say("  already on")
        return
    if not yes("  Turn on two-factor login now? You need it before exposing HA to the internet."):
        say("  skipped: turn it on later under your profile → Security")
        return
    form = await ha.call({"type": "auth/setup_mfa", "mfa_module_id": "totp"})
    secret = form["description_placeholders"]["code"]
    url = form["description_placeholders"].get("url") or pyotp.TOTP(secret).provisioning_uri(
        name=user["name"], issuer_name="Home Assistant")
    say("  Scan this with your authenticator app:")
    say(pyqrcode.create(url).terminal(quiet_zone=1))
    say(f"  Or enter this key by hand: {secret}")
    while True:
        code = ask("  6-digit code it shows now: ")
        result = await ha.call({"type": "auth/setup_mfa", "flow_id": form["flow_id"], "user_input": {"code": code}})
        if result.get("type") == "create_entry":
            say("  done: two-factor login is on")
            return
        form = result
        say("  That code didn't work. Try the current one.")


async def main():
    async with aiohttp.ClientSession() as http:
        await wait_for_ha(http)
        ha = HA(http)
        await sign_in(ha)
        await two_factor(ha)
        await speech_services(ha)
        await tuya(ha)
        fallback = await fallback_agent(ha)
        agent = await jev(ha, fallback)
        await assistant(ha, agent or fallback)
        if ha.ws is not None:
            await ha.ws.close()
    say("\nHome Assistant is configured.")


if __name__ == "__main__":
    asyncio.run(main())
