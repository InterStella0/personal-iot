"""Can Jev route voice transcripts to device actions? Measures accuracy, confidence and latency.

Each utterance is one request with one question per device (answered in parallel by Jev),
plus a routing question. Every case runs twice to see how stable the answers are.
"""
import json
import statistics
import sys
import time

from typesafe_sdk import Choice, TypeSafeClient

MODEL = "jev-1.13.0"
RUNS = 2

HOME = {
    "time": "Sunday 14:00",
    "devices": {
        "aircond": {"power": "off", "mode": "cool", "target_c": 24, "fan": "auto"},
        "fan": {"power": "on", "speed": 1, "max_speed": 3},
        "room_switch": {"power": "on", "note": "powers the bedroom lights"},
        "camera_light": {"level": "off"},
        "front_door_sensor": {"state": "closed", "note": "read-only sensor"},
    },
}

TEMPS = {f"set_{t}": f"Set the aircond target to exactly {t}°C" for t in range(16, 31)}
# no_change is listed first on purpose: Choice favours the first option, so the bias
# pushes towards doing nothing.
QUESTIONS = {
    "route": Choice(
        instructions="The `utterance` is something a person said to their smart-home voice assistant. "
        "What kind of request is it?",
        criteria={
            "device_command": "Asks to change one or more devices (switch, set, adjust, turn on/off)",
            "home_question": "Asks about the current state of the home or a device, without changing anything",
            "other": "Anything else: general questions, weather, chit-chat, thanks, jokes, cancelling",
        },
    ),
    "aircond_power": Choice(
        instructions="Based on the `utterance` and the aircond's current state in `devices.aircond`, "
        "what should happen to the aircond's power?",
        criteria={"no_change": "The utterance does not ask to change the aircond's power, or it is already in the requested state",
                  "turn_on": "Turn the aircond on", "turn_off": "Turn the aircond off"},
    ),
    "aircond_mode": Choice(
        instructions="Based on the `utterance`, what should happen to the aircond's operating mode?",
        criteria={"no_change": "The utterance does not ask for a different aircond mode",
                  "cool": "Cooling mode", "dry": "Dehumidify / dry mode", "fan_only": "Fan-only mode",
                  "auto": "Automatic mode"},
    ),
    "aircond_temp": Choice(
        instructions="Based on the `utterance` and `devices.aircond.target_c`, what should happen to the "
        "aircond's target temperature?",
        criteria={"no_change": "The utterance does not ask to change the aircond temperature",
                  "warmer": "Raise the target a little, without naming a number",
                  "cooler": "Lower the target a little, without naming a number", **TEMPS},
    ),
    "aircond_fan": Choice(
        instructions="Based on the `utterance`, what should happen to the aircond's own fan speed setting "
        "(not the standalone fan)?",
        criteria={"no_change": "The utterance does not ask to change the aircond's fan speed",
                  "auto": "Aircond fan on auto", "low": "Aircond fan low", "medium": "Aircond fan medium",
                  "high": "Aircond fan high / max"},
    ),
    "fan": Choice(
        instructions="Based on the `utterance` and `devices.fan`, what should happen to the standalone "
        "fan (kipas), not the aircond?",
        criteria={"no_change": "The utterance does not ask to change the standalone fan, or it is already in the requested state",
                  "off": "Turn the fan off", "speed_1": "Fan on at speed 1 (lowest)",
                  "speed_2": "Fan on at speed 2", "speed_3": "Fan on at speed 3 (highest)"},
    ),
    "room_switch": Choice(
        instructions="Based on the `utterance` and `devices.room_switch` (it powers the bedroom lights), "
        "what should happen to the room switch?",
        criteria={"no_change": "The utterance does not ask to change the room switch or lights, or it is already in the requested state",
                  "on": "Turn the room switch / lights on", "off": "Turn the room switch / lights off"},
    ),
    "camera_light": Choice(
        instructions="Based on the `utterance`, what should happen to the camera's light?",
        criteria={"no_change": "The utterance does not ask to change the camera light",
                  "off": "Camera light off", "half": "Camera light at half brightness",
                  "full": "Camera light fully on"},
    ),
}
DEVICE_QS = [q for q in QUESTIONS if q != "route"]

# (utterance, route, {question: acceptable answers}); questions not listed must be no_change.
CMD = "device_command"
CASES = [
    ("turn on the aircond", CMD, {"aircond_power": {"turn_on"}}),
    ("set the AC to 22", CMD, {"aircond_temp": {"set_22"}, "aircond_power": {"turn_on", "no_change"}}),
    ("it's so stuffy and humid in here, put the aircond on dry", CMD,
     {"aircond_mode": {"dry"}, "aircond_power": {"turn_on", "no_change"}}),
    ("turn off the fan and set the aircond to 26", CMD,
     {"fan": {"off"}, "aircond_temp": {"set_26"}, "aircond_power": {"turn_on", "no_change"}}),
    ("fan speed two please", CMD, {"fan": {"speed_2"}}),
    ("make the fan faster", CMD, {"fan": {"speed_2", "speed_3"}}),
    ("kill everything, I'm heading out", CMD,
     {"fan": {"off"}, "room_switch": {"off"}, "aircond_power": {"no_change", "turn_off"},
      "camera_light": {"no_change", "off"}}),
    ("aircond fan to max", CMD, {"aircond_fan": {"high"}, "aircond_power": {"turn_on", "no_change"}}),
    ("switch the aircond to auto mode", CMD, {"aircond_mode": {"auto"}, "aircond_power": {"turn_on", "no_change"}}),
    ("lights off", CMD, {"room_switch": {"off"}}),
    ("turn the camera light on halfway", CMD, {"camera_light": {"half"}}),
    ("set fan speed 3 and aircond fan high", CMD,
     {"fan": {"speed_3"}, "aircond_fan": {"high"}, "aircond_power": {"turn_on", "no_change"}}),
    ("can you on the aircond and off the fan ah", CMD, {"aircond_power": {"turn_on"}, "fan": {"off"}}),
    ("panas gila, buka aircond", CMD, {"aircond_power": {"turn_on"}}),
    ("tutup kipas", CMD, {"fan": {"off"}}),
    ("kipas laju sikit", CMD, {"fan": {"speed_2", "speed_3"}}),
    ("tolong set aircond 25 darjah", CMD, {"aircond_temp": {"set_25"}, "aircond_power": {"turn_on", "no_change"}}),
    ("bilik ni lembap la, on dry mode", CMD, {"aircond_mode": {"dry"}, "aircond_power": {"turn_on", "no_change"}}),
    ("off semua, I nak keluar dah", CMD,
     {"fan": {"off"}, "room_switch": {"off"}, "aircond_power": {"no_change", "turn_off"},
      "camera_light": {"no_change", "off"}}),
    ("aircond sejuk sangat, naikkan sikit", CMD,
     {"aircond_temp": {"warmer", "set_25", "set_26"}, "aircond_power": {"turn_on", "no_change"}}),
    ("lampu kamera separuh", CMD, {"camera_light": {"half"}}),
    ("is the front door open?", "home_question", {}),
    ("pintu depan dah tutup ke?", "home_question", {}),
    ("is the aircond on right now", "home_question", {}),
    ("what's the weather tomorrow", "other", {}),
    ("esok hujan tak?", "other", {}),
    ("tell me a joke", "other", {}),
    ("eh nevermind", "other", {}),
    ("thanks!", "other", {}),
    ("what time is it in Tokyo", "other", {}),
]


def main() -> None:
    client = TypeSafeClient(model=MODEL, timeout=30.0)
    rows = []
    for utterance, route, expect in CASES:
        for run in range(RUNS):
            t0 = time.perf_counter()
            r = client.system_one(state={"utterance": utterance, **HOME}, questions=QUESTIONS, model=MODEL)
            ms = (time.perf_counter() - t0) * 1000
            a = r.answers
            got = {q: a[q].choice for q in QUESTIONS}
            conf = {q: round(max(a[q].probabilities.values()), 3) for q in QUESTIONS}
            want = {q: expect.get(q, {"no_change"}) for q in DEVICE_QS}
            wrong = [q for q in DEVICE_QS if got[q] not in want[q]]
            rows.append({"utterance": utterance, "run": run, "ms": round(ms), "model": r.model,
                         "tokens": r.usage.input_tokens, "route_ok": got["route"] == route,
                         "route": got["route"], "want_route": route, "wrong": wrong, "got": got, "p": conf})
            mark = "ok " if not wrong and got["route"] == route else "BAD"
            changes = {q: f"{got[q]}@{conf[q]}" for q in DEVICE_QS if got[q] != "no_change"}
            print(f"{mark} {ms:5.0f}ms {utterance!r:48} route={got['route']}@{conf['route']} {changes}"
                  + (f"  WRONG={[(q, got[q]) for q in wrong]}" if wrong else ""), flush=True)
    json.dump(rows, open(sys.argv[1], "w"), indent=1)

    lat = [r["ms"] for r in rows]
    print(f"\nmodel={rows[0]['model']}  requests={len(rows)}  input tokens/request≈{statistics.mean(r['tokens'] for r in rows):.0f}")
    print(f"latency ms: median={statistics.median(lat):.0f} p90={sorted(lat)[int(len(lat) * .9)]} min={min(lat)} max={max(lat)}")
    full = sum(r["route_ok"] and not r["wrong"] for r in rows)
    print(f"fully correct: {full}/{len(rows)}   route correct: {sum(r['route_ok'] for r in rows)}/{len(rows)}")
    by_utt = {}
    for r in rows:
        by_utt.setdefault(r["utterance"], []).append(json.dumps(r["got"], sort_keys=True))
    print(f"identical answers across runs: {sum(len(set(v)) == 1 for v in by_utt.values())}/{len(by_utt)}")


if __name__ == "__main__":
    main()
