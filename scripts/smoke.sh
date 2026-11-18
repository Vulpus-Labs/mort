#!/bin/sh
# Smoke checks against a running REAPER with the mort bridge loaded
# (ADR 0011 §5). Run it before closing tickets that touch the bridge's REAPER
# session or the probe, and record the result in the close-out.
#
# Usage: scripts/smoke.sh [socket]
#
# For the full set, start REAPER on the smoke project first:
#   open -a REAPER --args -new "$PWD/scripts/smoke-project.lua"
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

sys.exit(1 if failures else 0)
PY
