"""Garbled and nonsense transcripts: does Jev ever execute the wrong thing?

Uses the same questions and house state as jev_voice_test.py, and the policy from that test:
Jev only executes when route=device_command (p>=0.7) and every change has p>=CUTOFF.
Otherwise the transcript would go to Claude, which counts as safe here.
"""
import json
import sys
from pathlib import Path
import time

sys.path.insert(0, str(Path(__file__).parent))
from jev_voice_test import DEVICE_QS, HOME, MODEL, QUESTIONS  # noqa: E402
from typesafe_sdk import TypeSafeClient  # noqa: E402

CUTOFF = 0.6
RUNS = 2

# Mis-heard commands: (transcript, what was actually said, acceptable answers if Jev does act).
GARBLED = [
    ("turn on the air con", "turn on the aircon", {"aircond_power": {"turn_on"}}),
    ("turn on the icon", "turn on the aircon", {"aircond_power": {"turn_on"}}),
    ("on the hair cone please", "on the aircond please", {"aircond_power": {"turn_on"}}),
    ("tow top key pass", "tutup kipas", {"fan": {"off"}}),
    ("too tub keep ass", "tutup kipas", {"fan": {"off"}}),
    ("key pass lad you sick it", "kipas laju sikit", {"fan": {"speed_2", "speed_3"}}),
    ("set the air con to twenty fife", "set the aircon to 25",
     {"aircond_temp": {"set_25"}, "aircond_power": {"turn_on", "no_change"}}),
    ("said a econ two twenty two", "set aircon to 22",
     {"aircond_temp": {"set_22"}, "aircond_power": {"turn_on", "no_change"}}),
    ("lice off", "lights off", {"room_switch": {"off"}}),
    ("off say more eye knack Clyde ah", "off semua, I nak keluar dah",
     {"fan": {"off"}, "room_switch": {"off"}, "aircond_power": {"no_change", "turn_off"},
      "camera_light": {"no_change", "off"}}),
    ("the fun speed to", "fan speed two", {"fan": {"speed_2"}}),
    ("panas killer bukak air con", "panas gila, buka aircond", {"aircond_power": {"turn_on"}}),
    ("camera lamp separate", "lampu kamera separuh", {"camera_light": {"half"}}),
    ("turn off the fence", "turn off the fan", {"fan": {"off"}}),
]

# Must never change a device. (transcript, why)
MUST_NOT_ACT = [
    ("blue harmonica sideways potato", "nonsense"),
    ("uh um hmm yeah", "filler"),
    ("and in other news the prime minister announced a new budget", "TV in the background"),
    ("Thank you for watching.", "classic Whisper hallucination on silence"),
    ("Subscribe to my channel", "classic Whisper hallucination on silence"),
    ("lah lah lah walao eh", "exclamation"),
    ("turn off the", "cut off mid-sentence"),
    ("off", "no device named"),
    ("on on on on", "no device named"),
    ("kipas kipas kipas", "device named, no action"),
    ("the cat is sleeping next to the fan", "mentions a device, not a command"),
    ("I hate this aircond", "complaint, not a command"),
    ("jangan tutup kipas", "negation: don't turn off the fan"),
    ("don't turn on the aircond", "negation"),
    ("turn on the microwave", "device that doesn't exist"),
    ("open the garage door", "device that doesn't exist"),
    ("set the aircond to 99", "impossible temperature"),
]


def main() -> None:
    client = TypeSafeClient(model=MODEL, timeout=30.0)
    rows = []

    def ask(text):
        t0 = time.perf_counter()
        r = client.system_one(state={"utterance": text, **HOME}, questions=QUESTIONS, model=MODEL)
        ms = (time.perf_counter() - t0) * 1000
        got = {q: r.answers[q].choice for q in QUESTIONS}
        p = {q: round(max(r.answers[q].probabilities.values()), 3) for q in QUESTIONS}
        changes = {q: got[q] for q in DEVICE_QS if got[q] != "no_change"}
        executes = (got["route"] == "device_command" and p["route"] >= 0.7 and bool(changes)
                    and all(p[q] >= CUTOFF for q in changes))
        return got, p, changes, executes, ms

    print("== Mis-heard commands (recover or hand to Claude; never do the wrong thing)")
    for text, meant, expect in GARBLED:
        for run in range(RUNS):
            got, p, changes, executes, ms = ask(text)
            want = {q: expect.get(q, {"no_change"}) for q in DEVICE_QS}
            correct = all(got[q] in want[q] for q in DEVICE_QS)
            outcome = ("RECOVERED" if correct else "WRONG ACTION") if executes else "to Claude"
            rows.append({"kind": "garbled", "text": text, "meant": meant, "run": run,
                         "outcome": outcome, "got": got, "p": p, "ms": round(ms)})
            shown = {q: f"{v}@{p[q]}" for q, v in changes.items()}
            print(f"  {outcome:12} {text!r:40} (meant {meant!r}) route={got['route']}@{p['route']} {shown}",
                  flush=True)

    print("\n== Must not act")
    for text, why in MUST_NOT_ACT:
        for run in range(RUNS):
            got, p, changes, executes, ms = ask(text)
            outcome = "ACTED (BAD)" if executes else "safe"
            rows.append({"kind": "must_not_act", "text": text, "why": why, "run": run,
                         "outcome": outcome, "got": got, "p": p, "ms": round(ms)})
            shown = {q: f"{v}@{p[q]}" for q, v in changes.items()}
            print(f"  {outcome:12} {text!r:62} [{why}] route={got['route']}@{p['route']} {shown}", flush=True)

    json.dump(rows, open(sys.argv[1], "w"), indent=1)
    g = [r for r in rows if r["kind"] == "garbled"]
    n = [r for r in rows if r["kind"] == "must_not_act"]
    count = lambda rs, o: sum(r["outcome"] == o for r in rs)  # noqa: E731
    print(f"\nGarbled ({len(g)}): recovered {count(g, 'RECOVERED')}, to Claude {count(g, 'to Claude')}, "
          f"WRONG ACTION {count(g, 'WRONG ACTION')}")
    print(f"Must-not-act ({len(n)}): safe {count(n, 'safe')}, ACTED {count(n, 'ACTED (BAD)')}")


if __name__ == "__main__":
    main()
