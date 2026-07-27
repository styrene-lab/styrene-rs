#!/usr/bin/env bash
# Shared helpers for .upstream-tracking.json.
# Requires TRACKING_FILE to be set by the caller.

_tracking_field() {
    local key="$1" field="$2"
    python3 - "$TRACKING_FILE" "$key" "$field" <<'PY'
import json, sys
path, key, field = sys.argv[1:]
try:
    value = json.load(open(path, encoding="utf-8")).get(key, {}).get(field, "")
except (OSError, json.JSONDecodeError):
    value = ""
print(value if value is not None else "")
PY
}

read_tracking() { _tracking_field "$1" last_reviewed; }
read_remote() { _tracking_field "$1" remote; }
read_branch() { _tracking_field "$1" branch; }

write_tracking() {
    local key="$1" revision="$2"
    python3 - "$TRACKING_FILE" "$key" "$revision" <<'PY'
import json, sys
from pathlib import Path
path, key, revision = Path(sys.argv[1]), sys.argv[2], sys.argv[3]
data = json.loads(path.read_text(encoding="utf-8"))
if key not in data:
    raise SystemExit(f"unknown lineage key: {key}")
data[key]["last_reviewed"] = revision
path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
PY
}
