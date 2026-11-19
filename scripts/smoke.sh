#!/bin/sh
# Smoke checks against a running REAPER with the mort bridge loaded
# (ADR 0011 §5). Run it before closing tickets that touch the bridge's REAPER
# session or the probe, and record the result in the close-out.
#
# Usage: scripts/smoke.sh [socket]
#
# For the full set, start REAPER on the smoke project first:
#   open -a REAPER --args -new "$PWD/scripts/smoke-project.lua"
# The edit checks change that project, so run them once per start. The
# project is throwaway and never saved under its own name: quit that REAPER
# with `kill -TERM $(pgrep -x REAPER)`, which does not ask to save.
#
# Not covered here: *mort: hold edits* is an action only a person can run.
# Check it by hand: run it from REAPER's action list, then `mort status`
# shows `held yes` and `mort track add x` fails with `busy`.
set -eu

dir="${TMPDIR:-/tmp}"
dir="${dir%/}/mort"
sock="${1:-}"
if [ -z "$sock" ]; then
    if [ ! -f "$dir/current" ]; then
        echo "smoke: no bridge running ($dir/current is missing); start REAPER with reaper_mort.dylib installed" >&2
        exit 3
    fi
    sock="$(cat "$dir/current")"
fi

python3 -I - "$sock" <<'PY'
import json, socket, sys

sock = sys.argv[1]
s = socket.socket(socket.AF_UNIX)
s.settimeout(5)
try:
    s.connect(sock)
except OSError as e:
    print(f"smoke: cannot connect to {sock}: {e}", file=sys.stderr)
    sys.exit(3)
f = s.makefile("rw")
next_id = 0

def call(method, params=None):
    global next_id
    next_id += 1
    f.write(json.dumps({"jsonrpc": "2.0", "id": next_id, "method": method, "params": params}) + "\n")
    f.flush()
    return json.loads(f.readline())

failures = 0

def check(name, ok, detail):
    global failures
    print(("ok   " if ok else "FAIL ") + name + ": " + detail)
    failures += 0 if ok else 1

r = call("bridge.hello", {"protocol": [0, 1]})
check("hello", r.get("result", {}).get("protocol") == [0, 1], json.dumps(r.get("result", r)))

r = call("bridge.hello", {"protocol": [99, 0]})
check("version refused", "error" in r and "99.0" in r["error"]["message"], r.get("error", {}).get("message", "no error"))

r = call("bridge.status")
res = r.get("result", {})
check("status", isinstance(res.get("state"), int), json.dumps(res))

# The rest needs the project built by scripts/smoke-project.lua.
project = call("project.get").get("result", {})
lst = call("tracks.list")["result"]
by_name = {t["name"]: t for t in lst["tracks"]}
if list(by_name) != ["Drums", "Kick", "Bass", "Verb"]:
    print("skip reads: start REAPER on scripts/smoke-project.lua to check them")
    sys.exit(1 if failures else 0)
check("tracks", list(by_name) == ["Drums", "Kick", "Bass", "Verb"], ", ".join(by_name))
kick, bass, verb = by_name.get("Kick", {}), by_name.get("Bass", {}), by_name.get("Verb", {})
check("folder", kick.get("parent") == by_name.get("Drums", {}).get("id"), str(kick.get("parent")))
check("send", [s["dest"] for s in kick.get("sends", [])] == [verb.get("id")], json.dumps(kick.get("sends")))
check("mix controls",
      abs(bass.get("volume", 0) - 0.5) < 1e-9 and abs(bass.get("pan", 0) + 0.2) < 1e-9 and bass.get("mute") is True,
      json.dumps({k: bass.get(k) for k in ("volume", "pan", "mute")}))
fx = bass.get("fx", [])
check("fx chain",
      len(fx) == 2 and "ReaEQ" in fx[0]["name"] and "ReaComp" in fx[1]["name"]
      and fx[0]["enabled"] and not fx[1]["enabled"],
      json.dumps([(f["name"], f["plugin"], f["enabled"]) for f in fx]))
check("master", lst["master"]["name"] == "MASTER" and lst["master"]["index"] is None, lst["master"]["id"])

if fx:
    p = call("fx.params", {"fx": fx[0]["id"]}).get("result", {})
    params = p.get("params", [])
    check("params", len(params) > 3 and all(x["display"] for x in params[:3]),
          f"{len(params)} params; " + "; ".join(f'{x["name"]}={x["display"]}' for x in params[:3]))
    check("params' track", p.get("track") == bass.get("id"), str(p.get("track")))

regions = project.get("regions", [])
check("regions",
      [(r["name"], r["start"], r["end"]) for r in regions] == [("verse", 0.0, 16.0), ("drop", 8.0, None)],
      json.dumps(regions))
print("info path: " + str(project["project"].get("path")) + " (the smoke project is never saved under its own name)")

items = call("items.list", {"track": bass.get("id")}).get("result", {}).get("items", [])
check("items", len(items) == 1 and items[0]["midi"] and abs(items[0]["length"] - 4) < 1e-9,
      json.dumps(items))

t = call("time.convert", {"to_seconds": ["9.1", "+4b"], "to_musical": [16.5]}).get("result", {})
check("time", t.get("seconds") == [16.0, 18.0] and t.get("musical") == ["9.2"], json.dumps(t))

# --- Edits (ticket 0006). These change the smoke project. ---------------
def edit(method, params, summary):
    params = dict(params, summary=summary)
    r = call(method, params)
    return r.get("result"), r.get("error")

def tracks():
    l = call("tracks.list")["result"]
    return l, {t["name"]: t for t in l["tracks"]}

r, e = edit("track.add", {"name": "Lead", "index": 0}, "smoke add lead")
_, by = tracks()
check("track.add at 0", e is None and list(by)[0] == "Lead" and by["Lead"]["parent"] is None, str(e or list(by)))

r, e = edit("track.add", {"name": "Snare", "parent": by["Drums"]["id"]}, "smoke add snare")
_, by = tracks()
order = list(by)
check("track.add in folder",
      e is None and by["Snare"]["parent"] == by["Drums"]["id"] and by["Kick"]["parent"] == by["Drums"]["id"]
      and by["Bass"]["parent"] is None and order.index("Snare") == order.index("Kick") + 1,
      str(e or [(n, t["parent"] is not None) for n, t in by.items()]))

r, e = edit("track.set", {"track": by["Lead"]["id"], "volume": 0.25, "pan": 0.3, "solo": True, "name": "Lead 1"}, "smoke set lead")
_, by = tracks()
lead = by.get("Lead 1", {})
check("track.set", e is None and abs(lead.get("volume", 0) - 0.25) < 1e-9 and abs(lead.get("pan", 0) - 0.3) < 1e-9 and lead.get("solo"),
      str(e or {k: lead.get(k) for k in ("volume", "pan", "solo")}))

r, e = edit("send.add", {"from": by["Bass"]["id"], "to": by["Verb"]["id"]}, "smoke send")
i = (r or {}).get("send")
r, e2 = edit("send.set", {"from": by["Bass"]["id"], "send": i, "volume": 0.5}, "smoke send level")
_, by = tracks()
check("sends", e is None and e2 is None and [(s["dest"], s["volume"]) for s in by["Bass"]["sends"]] == [(by["Verb"]["id"], 0.5)],
      str(e or e2 or by["Bass"]["sends"]))

r, e = edit("fx.add", {"track": by["Verb"]["id"], "plugin": "ReaEQ"}, "smoke eq")
eq = (r or {}).get("fx", {}).get("id")
r, e2 = edit("fx.add", {"track": by["Verb"]["id"], "plugin": "ReaComp", "index": 0}, "smoke comp")
comp = (r or {}).get("fx", {}).get("id")
_, by = tracks()
check("fx.add at position", [f["id"] for f in by["Verb"]["fx"]] == [comp, eq], str(e or e2 or by["Verb"]["fx"]))
edit("fx.move", {"fx": eq, "index": 0}, "smoke move")
edit("fx.enable", {"fx": comp, "enabled": False}, "smoke bypass")
_, by = tracks()
check("fx.move and bypass", [(f["id"], f["enabled"]) for f in by["Verb"]["fx"]] == [(eq, True), (comp, False)],
      json.dumps([(f["name"], f["enabled"]) for f in by["Verb"]["fx"]]))
r, e = edit("fx.add", {"track": by["Verb"]["id"], "plugin": "No Such Plugin"}, "smoke missing")
check("fx.add missing plugin", e is not None and e["data"]["kind"] == "not_found", str(e))
r, e = edit("fx.preset", {"fx": eq, "preset": "No such preset"}, "smoke preset")
check("fx.preset missing", e is not None and e["data"]["kind"] == "not_found", str(e))

params = call("fx.params", {"fx": eq})["result"]["params"]
gain = next(p["index"] for p in params if p["name"].startswith("Gain-Band 2"))
freq = next(p["index"] for p in params if p["name"].startswith("Freq-Band 2"))
r, e = edit("param.set", {"fx": eq, "index": gain, "text": "-3"}, "smoke gain")
check("param.set by text (gain)", e is None and r["display"].startswith("-3.0"), str(e or r))
r, e = edit("param.set", {"fx": eq, "index": freq, "text": "2400"}, "smoke freq")
check("param.set by text (freq)", e is None and r["display"].startswith("2400"), str(e or r))
r, e = edit("param.set", {"fx": eq, "index": gain, "value": 0.5}, "smoke gain value")
check("param.set by value", e is None and abs(r["value"] - 0.5) < 1e-9, str(e or r))

def gain_value():
    return call("fx.params", {"fx": eq})["result"]["params"][gain]["value"]

st = call("fx.state", {"fx": eq}).get("result", {}).get("chunk", "")
saved = gain_value()
edit("param.set", {"fx": eq, "index": gain, "value": 0.9}, "smoke gain up")
r, e = edit("fx.set_state", {"fx": eq, "chunk": st}, "smoke state")
check("fx.state restores", st.startswith("<VST") and e is None and abs(gain_value() - saved) < 1e-6,
      f"{e} gain {saved} -> {gain_value()}")
r, e = edit("fx.set_state", {"fx": eq, "chunk": "<CLAP x\\n>"}, "smoke bad state")
check("fx.set_state refuses another kind", e is not None and e["data"]["kind"] == "invalid", str(e))

state = call("bridge.status")["result"]["state"]
r, e = edit("track.set", {"track": by["Bass"]["id"], "mute": False, "expect_state": state - 1}, "smoke stale")
check("stale", e is not None and e["data"]["kind"] == "stale", str(e))

r = call("undo")
check("undo", r.get("result", {}).get("label") == "mort: smoke state" and abs(gain_value() - 0.9) < 1e-6,
      f"{r.get('result') or r.get('error')} gain {gain_value()}")
r = call("redo")
check("redo", r.get("result", {}).get("label") == "mort: smoke state" and abs(gain_value() - saved) < 1e-6,
      f"{r.get('result') or r.get('error')} gain {gain_value()}")

_, by = tracks()
edit("track.rm", {"track": by["Lead 1"]["id"]}, "smoke rm lead")
edit("track.rm", {"track": by["Snare"]["id"]}, "smoke rm snare")
_, by = tracks()
check("track.rm keeps folders", list(by) == ["Drums", "Kick", "Bass", "Verb"] and by["Kick"]["parent"] == by["Drums"]["id"]
      and by["Bass"]["parent"] is None, str([(n, t["parent"] is not None) for n, t in by.items()]))

# Save the smoke project's copy so REAPER can quit without asking.
import os
r = call("project.save", {"path": os.path.join(os.path.dirname(sock), "smoke.rpp")})
check("project.save", "result" in r, str(r.get("error", "saved")))

sys.exit(1 if failures else 0)
PY
