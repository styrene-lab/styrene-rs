#!/usr/bin/env bash
# Screenshot the styrene-dx window for visual feedback.
# Usage: ./scripts/screenshot-dx.sh [output.png]
#
# Launches styrene-dx, waits for render, captures the screen, saves screenshot.
# Requires screen recording permission for the calling terminal.

set -euo pipefail

OUT="${1:-/tmp/styrene-dx-screenshot.png}"
BINARY="$(cd "$(dirname "$0")/.." && pwd)/target/debug/styrene-dx"

if [ ! -f "$BINARY" ]; then
    echo "Build first: cargo build -p styrene-dx"
    exit 1
fi

# Kill any existing instance
pkill -f styrene-dx 2>/dev/null || true
sleep 1

# Launch and wait for render
"$BINARY" &
DX_PID=$!
sleep 4

# Full-screen capture (window-targeted capture is unreliable with webview)
screencapture -x "$OUT"
echo "Screenshot saved: $OUT"

kill $DX_PID 2>/dev/null
wait $DX_PID 2>/dev/null
