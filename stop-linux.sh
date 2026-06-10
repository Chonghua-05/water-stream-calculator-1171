#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PID_FILE="$ROOT/artifacts/viewer_pid.txt"

if [[ ! -f "$PID_FILE" ]]; then
  echo "No viewer PID file found."
  exit 0
fi

VIEWER_PID="$(head -n 1 "$PID_FILE" || true)"
if [[ -n "$VIEWER_PID" ]] && kill -0 "$VIEWER_PID" >/dev/null 2>&1; then
  kill "$VIEWER_PID" >/dev/null 2>&1 || true
fi

rm -f "$PID_FILE"
echo "Stopped viewer."
