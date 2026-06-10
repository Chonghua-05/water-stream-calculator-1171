#!/usr/bin/env bash
set -euo pipefail

PORT="8766"
OPEN_BROWSER="1"

while [[ $# -gt 0 ]]; do
  case "$1" in
    -p|--port)
      if [[ $# -lt 2 ]]; then
        echo "Missing value for $1" >&2
        exit 2
      fi
      PORT="$2"
      shift 2
      ;;
    --no-browser)
      OPEN_BROWSER="0"
      shift
      ;;
    -h|--help)
      echo "Usage: ./start-linux.sh [--port PORT] [--no-browser]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ARTIFACTS_DIR="$ROOT/artifacts"
LOGS_DIR="$ROOT/logs"
PID_FILE="$ARTIFACTS_DIR/viewer_pid.txt"
STDOUT_LOG="$LOGS_DIR/viewer_stdout.log"
STDERR_LOG="$LOGS_DIR/viewer_stderr.log"
BUNDLED_SOLVER="$ROOT/bin/linux/item-waterway-solver"
CARGO_MANIFEST="$ROOT/rust-backend/Cargo.toml"
BUILT_SOLVER="$ROOT/rust-backend/target/release/item-waterway-solver"

mkdir -p "$ARTIFACTS_DIR" "$LOGS_DIR"

latest_source_tick() {
  local source_root="$1"
  if [[ ! -d "$source_root" ]]; then
    echo 0
    return
  fi
  find "$source_root" -type f -printf '%T@\n' 2>/dev/null | sort -nr | head -n 1
}

build_rust_solver() {
  if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo is unavailable, and no usable bundled solver was found." >&2
    exit 1
  fi
  cargo build --release --bin item-waterway-solver --manifest-path "$CARGO_MANIFEST"
  if [[ ! -x "$BUILT_SOLVER" ]]; then
    echo "Built solver was not produced at $BUILT_SOLVER" >&2
    exit 1
  fi
  echo "$BUILT_SOLVER"
}

resolve_solver_path() {
  if [[ -x "$BUNDLED_SOLVER" ]]; then
    if [[ -f "$CARGO_MANIFEST" ]]; then
      local source_tick bundled_tick
      source_tick="$(latest_source_tick "$ROOT/rust-backend/src")"
      bundled_tick="$(stat -c '%Y' "$BUNDLED_SOLVER")"
      if awk "BEGIN { exit !($source_tick > $bundled_tick) }"; then
        if command -v cargo >/dev/null 2>&1; then
          build_rust_solver
          return
        fi
        echo "Bundled solver is older than rust-backend/src, but cargo is unavailable. Falling back to bundled solver." >&2
      fi
    fi
    echo "$BUNDLED_SOLVER"
    return
  fi
  if [[ -x "$BUILT_SOLVER" ]]; then
    echo "$BUILT_SOLVER"
    return
  fi
  if [[ -f "$CARGO_MANIFEST" ]]; then
    build_rust_solver
    return
  fi
  echo "Rust solver is unavailable." >&2
  exit 1
}

if [[ -f "$PID_FILE" ]]; then
  EXISTING_PID="$(head -n 1 "$PID_FILE" || true)"
  if [[ -n "$EXISTING_PID" ]] && kill -0 "$EXISTING_PID" >/dev/null 2>&1; then
    echo "Viewer already running at http://127.0.0.1:$PORT (PID $EXISTING_PID)"
    exit 0
  fi
  rm -f "$PID_FILE"
fi

SOLVER="$(resolve_solver_path)"

export WATERWAY_HOME="$ROOT"
export WATERWAY_APP_DIR="$ROOT"
export WATERWAY_DATA_DIR="$ROOT/data"
export MC_VIEWER_DATA_DIR="$ROOT/data/viewer_data"
export MC_VIEWER_STATIC_DIR="$ROOT/viewer"
export WATERWAY_ASSET_DIR="$ROOT/assets/minecraft/textures/block"
export WATERWAY_PARTS_CONFIG="$ROOT/model/config/waterway-structure-parts.json"
export WATERWAY_SOLVER="$SOLVER"
export MC_VIEWER_HOST="127.0.0.1"
export MC_VIEWER_PORT="$PORT"

"$SOLVER" serve-web >"$STDOUT_LOG" 2>"$STDERR_LOG" &
VIEWER_PID="$!"

sleep 0.8
if ! kill -0 "$VIEWER_PID" >/dev/null 2>&1; then
  echo "Viewer failed to start." >&2
  if [[ -f "$STDERR_LOG" ]]; then
    tail -n 40 "$STDERR_LOG" >&2 || true
  fi
  exit 1
fi

echo "$VIEWER_PID" > "$PID_FILE"
echo "Started viewer PID $VIEWER_PID"
echo "Viewer URL http://127.0.0.1:$PORT"

if [[ "$OPEN_BROWSER" == "1" ]]; then
  if command -v xdg-open >/dev/null 2>&1; then
    xdg-open "http://127.0.0.1:$PORT" >/dev/null 2>&1 || true
  elif command -v sensible-browser >/dev/null 2>&1; then
    sensible-browser "http://127.0.0.1:$PORT" >/dev/null 2>&1 || true
  fi
fi
