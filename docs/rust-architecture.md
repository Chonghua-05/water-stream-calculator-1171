# Rust Architecture

## Goal

This document summarizes the runtime architecture of the calculator.

Its runtime responsibilities are:

- serve the static viewer
- accept simulation requests
- store viewer runs
- run reachable-candidate search
- expose the same local HTTP API used by the existing viewer

## Top-Level Layout

- `viewer/`
  Static front-end files.
- `rust-backend/`
  Rust service and search implementation.
- `data/viewer_data/`
  Viewer run store.
- `data/reachability-candidate-generator/`
  Search artifacts written by the Rust generator.
- `assets/minecraft/textures/block/`
  Minimal block textures used by the viewer.
- `model/config/`
  Published structure-parts JSON snapshot.

## Runtime Boundaries

### 1. Static viewer

The browser UI is plain static HTML, JS, and CSS from `viewer/`.

The Rust service serves it directly through `serve-web`.

### 2. Viewer run store

Viewer-visible runs are stored under:

```text
data/viewer_data/runs/
```

The active format is split-run storage:

- `data/viewer_data/runs/index.json`
- `data/viewer_data/runs/run-<id>.json`

The sample dataset in this repo contains only:

- `run-910002.json` (`游戏实测2`)

### 3. Search artifact store

Reachable-candidate search writes diagnostic artifacts under:

```text
data/reachability-candidate-generator/
```

Only viewer-promoted runs are written back into the viewer run store.

## Service Endpoints

The Rust service owns these viewer-facing routes:

- `GET /api/status`
- `GET /api/runs`
- `POST /api/model/simulate`
- `POST /api/model/search`
- `GET /api/model/search/<task_id>`
- `POST /api/model/search/<task_id>/cancel`
- `POST /api/litematic/import`
- `POST /api/litematic/export`

`/api/model/compare` is currently reserved and returns unavailable.

## Search Flow

1. Viewer sends a search request.
2. Rust service derives launch/search execution parameters.
3. Rust reachable-candidate generator scans target windows and expands candidate prefixes.
4. Rust solver verifies candidate behavior and calculates hit-rate metrics.
5. Passing candidates are promoted into viewer runs.
6. Search diagnostics remain in `data/reachability-candidate-generator/`.

The final proof source is still the solver simulation used by the Rust backend. Search beam ranking is only a heuristic stage.

## Configuration Boundary

This repo includes:

```text
model/config/waterway-structure-parts.json
```

as the published config snapshot and format reference.

Current limitation:

- the active Rust search catalog is still compiled into `rust-backend/src/lib.rs`
- the packaged JSON is not yet the single runtime source of truth for the Rust searcher

At the moment, the JSON serves as the external format reference, while the active search catalog remains compiled into the Rust backend.

## Start Script Contract

`start-windows.ps1` sets these runtime roots before launching `serve-web`:

- `MC_VIEWER_STATIC_DIR = <repo>/viewer`
- `MC_VIEWER_DATA_DIR = <repo>/data/viewer_data`
- `WATERWAY_DATA_DIR = <repo>/data`
- `WATERWAY_ASSET_DIR = <repo>/assets/minecraft/textures/block`
- `WATERWAY_PARTS_CONFIG = <repo>/model/config/waterway-structure-parts.json`

The script prefers the bundled Windows solver binary in `bin/windows/`, and rebuilds from `rust-backend/` when the source tree is newer and `cargo` is available.
