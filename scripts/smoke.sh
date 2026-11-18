#!/bin/sh
# Smoke checks against a running REAPER with the mort bridge loaded
# (ADR 0011 §5). Run it before closing tickets that touch the bridge's REAPER
# session or the probe, and record the result in the close-out.
#
# Usage: scripts/smoke.sh [socket]
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

sys.exit(1 if failures else 0)
PY
