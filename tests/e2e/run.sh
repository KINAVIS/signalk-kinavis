#!/usr/bin/env bash
# End-to-end check: a Signal K server with this plugin, fed recorded AIS
# traffic, must raise the collision alarm the recording holds.
#
# Needs plugin.wasm built. SIGNALK_SERVER names an installed signalk-server
# package directory; without it, one is installed into a temporary directory.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
plugin="$(cd "$here/../.." && pwd)"
work="$(mktemp -d)"
port=3123
trap 'kill "${server_pid:-0}" 2>/dev/null || true; rm -rf "$work"' EXIT

test -f "$plugin/plugin.wasm" || { echo "plugin.wasm not built"; exit 1; }

server="${SIGNALK_SERVER:-}"
if [ -z "$server" ]; then
  (cd "$work" && npm init -y >/dev/null && npm install --no-audit --no-fund --silent signalk-server@2.33.0)
  server="$work/node_modules/signalk-server"
fi

# A configuration directory of its own: the server never touches ~/.signalk.
config="$work/config"
mkdir -p "$config/node_modules" "$config/plugin-config-data"
ln -s "$plugin" "$config/node_modules/signalk-kinavis"
python3 "$here/replay.py" "$here/data/harlingen.nmea" "$work/replay.nmea"
cat > "$config/package.json" <<JSON
{ "name": "e2e", "version": "0.0.0", "description": "e2e", "repository": "-", "license": "Apache-2.0",
  "dependencies": { "signalk-kinavis": "file:node_modules/signalk-kinavis" } }
JSON
cat > "$config/settings.json" <<JSON
{ "mdns": false, "interfaces": {},
  "pipedProviders": [ { "id": "harlingen", "enabled": true, "pipeElements": [ { "type": "providers/simple",
    "options": { "logging": false, "type": "FileStream",
      "subOptions": { "dataType": "NMEA0183", "filename": "$work/replay.nmea", "throttleRate": 4000 } } } ] } ] }
JSON
# The policy of the recording's encounter: two miles, half an hour.
cat > "$config/plugin-config-data/signalk-kinavis.json" <<JSON
{ "enabled": true, "configuration": { "cpaLimitNm": 2.0, "tcpaLimitMin": 30, "warnWithinMin": 120, "staleAfterS": 600, "assessEveryS": 2 } }
JSON

PORT=$port node "$server/bin/signalk-server" -c "$config" > "$work/server.log" 2>&1 &
server_pid=$!

url="http://localhost:$port/signalk/v1/api/vessels/self/notifications/navigation/closestApproach"
for _ in $(seq 1 90); do
  sleep 1
  if curl -sf "$url" > "$work/notifications.json" 2>/dev/null &&
     python3 - "$work/notifications.json" <<'PY'
import json, sys
notifications = json.load(open(sys.argv[1]))
alarms = [n["value"]["message"] for n in notifications.values()
          if n.get("value", {}).get("state") == "alarm" and "(Rule 15)" in n["value"]["message"]]
for message in alarms:
    print("alarm:", message)
sys.exit(0 if alarms else 1)
PY
  then
    echo "The plugin raised the alarm."
    exit 0
  fi
done

echo "No Rule 15 alarm within 90 s. Server log:"
tail -50 "$work/server.log"
exit 1
