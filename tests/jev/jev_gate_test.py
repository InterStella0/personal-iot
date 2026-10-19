"""Adds a 'clear_command' gate question and reruns clean, garbled and must-not-act cases.

Saves every answer, then compares execution policies offline so thresholds can be tuned
without paying for more requests.
"""
import json
import sys
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, str(Path(__file__).parent))
from jev_garbled_test import GARBLED, MUST_NOT_ACT  # noqa: E402
from jev_voice_test import CASES, DEVICE_QS, HOME, MODEL, QUESTIONS  # noqa: E402
from typesafe_sdk import Noul, TypeSafeClient  # noqa: E402

RUNS = 2
GATED = {
    **QUESTIONS,
    "clear_command": Noul(
        instructions="The `utterance` is a speech-to-text transcript of something said to a smart-home "
        "voice assistant, so it may contain recognition errors. Is it a complete, clearly worded request "
        "that names (or unmistakably refers to) a specific device in `devices` and says what to do with it?",
        criteria={
            "true": "Complete and unambiguous: both the device and the action are clear, with no missing, "
            "garbled or nonsensical words.",
            "false": "Cut off, garbled, repeated, nonsensical, a negation, or the device or the action is "
            "unclear and would have to be guessed.",
        },
    ),
}

cases = []  # (kind, text, expect or None)
for text, route, expect in CASES:
    cases.append(("clean_cmd" if route == "device_command" else "clean_other", text, expect))
for text, _meant, expect in GARBLED:
    cases.append(("garbled", text, expect))
for text, _why in MUST_NOT_ACT:
    cases.append(("must_not_act", text, None))


def run(job):
    kind, text, expect = job
    client = TypeSafeClient(model=MODEL, timeout=30.0)
    r = client.system_one(state={"utterance": text, **HOME}, questions=GATED, model=MODEL)
    got = {q: r.answers[q].choice for q in QUESTIONS}
    p = {q: max(r.answers[q].probabilities.values()) for q in QUESTIONS}
    correct = None
    if expect is not None:
        correct = all(got[q] in expect.get(q, {"no_change"}) for q in DEVICE_QS)
    return {"kind": kind, "text": text, "got": got, "p": p, "clear": r.answers["clear_command"].noul,
            "correct": correct}


jobs = [c for c in cases for _ in range(RUNS)]
with ThreadPoolExecutor(8) as pool:
    rows = list(pool.map(run, jobs))
json.dump(rows, open(sys.argv[1], "w"), indent=1)


def evaluate(gate):
    tally = {}
    for r in rows:
        changes = [q for q in DEVICE_QS if r["got"][q] != "no_change"]
        acts = (r["got"]["route"] == "device_command" and r["p"]["route"] >= 0.7 and bool(changes)
                and all(r["p"][q] >= 0.6 for q in changes) and (gate is None or r["clear"] >= gate))
        if not acts:
            outcome = "to_claude"
        elif r["kind"] == "must_not_act" or r["kind"] == "clean_other" or not r["correct"]:
            outcome = "WRONG"
        else:
            outcome = "right"
        t = tally.setdefault(r["kind"], {"right": 0, "WRONG": 0, "to_claude": 0})
        t[outcome] += 1
    return tally


print(f"{'policy':16} {'clean cmds (right/WRONG/claude)':34} {'garbled':22} {'must-not-act':16}")
for gate in (None, 0.5, 0.7, 0.8, 0.9):
    t = evaluate(gate)
    fmt = lambda k: f"{t[k]['right']}/{t[k]['WRONG']}/{t[k]['to_claude']}"  # noqa: E731
    name = "no gate" if gate is None else f"clear >= {gate}"
    print(f"{name:16} {fmt('clean_cmd'):34} {fmt('garbled'):22} "
          f"{t['must_not_act']['WRONG']} acted / {len([r for r in rows if r['kind'] == 'must_not_act'])}")

print("\nclear_command scores (mean of runs):")
seen = {}
for r in rows:
    seen.setdefault((r["kind"], r["text"]), []).append(r["clear"])
for (kind, text), v in seen.items():
    if kind != "clean_other":
        print(f"  {sum(v) / len(v):.2f}  {kind:13} {text}")
