完全vibecoding的产物，请见谅😭

# Water Stream Calculator 1.17.1

English | [简体中文](README.zh-CN.md)

Water Stream Calculator 1.17.1 is a local toolkit for modeling Minecraft 1.17.1 item movement in water-stream channels. It provides a browser-based structure editor, Rust simulation backend, reachable-candidate search, run inspection, and one sample game-captured dataset.

## Features

- Edit and inspect water-stream structures in the local viewer.
- Simulate item movement with the Rust backend.
- Search reachable structure candidates and promote verified results into viewer runs.
- Store runs under the local split-run data format.
- Open the included sample dataset `游戏实测2` directly in the viewer.

## Model and Search Overview

### Item Movement Model

The Rust backend builds the item movement model from Minecraft 1.17.1 source code, especially the `ItemEntity` tick path. The model covers water pushing, gravity, collision response, ground friction, ice and slime floor effects, and tick-phase details.

### Candidate Search

The searcher expands structure prefixes, simulates candidate behavior, and scores promising states with speed and cadence metrics. After each expansion layer, it prunes the frontier before continuing the search.

### Parallel Evaluation

During reachable-candidate search, prefix evaluation is distributed across worker threads controlled by the search thread setting. Candidate ranking is heuristic; accepted results are still judged by Rust simulation and hit-rate metrics.

### Data Inspection

The local viewer organizes run data, steady-state metrics, CSV export, and interactive speed charts so movement behavior can be inspected visually against game-captured data.

## Quick Start

### Windows

Start the local service:

```powershell
.\start-windows.ps1
```

Then open:

```text
http://127.0.0.1:8766
```

If PowerShell blocks script execution, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\start-windows.ps1
```

Stop the service:

```powershell
.\stop-windows.ps1
```

### Linux

Start the local service from the Linux release package:

```bash
chmod +x ./start-linux.sh ./stop-linux.sh ./bin/linux/item-waterway-solver
./start-linux.sh --no-browser
```

Then open:

```text
http://127.0.0.1:8766
```

Stop the service:

```bash
./stop-linux.sh
```

## Build From Source

Release packages include the platform binary for their target system. To rebuild manually:

```powershell
cargo build --release --manifest-path .\rust-backend\Cargo.toml
```

The startup scripts use the bundled binary by default. If the Rust source is newer and `cargo` is available, they rebuild and start the fresh binary.

## Project Layout

- `viewer/`
  Static web UI for structure editing, simulation, search, and run inspection.
- `rust-backend/`
  Rust backend source for the local service, simulation, run storage, and reachable-candidate search.
- `model/config/waterway-structure-parts.json`
  Structure-parts config snapshot and format reference.
- `assets/minecraft/textures/block/`
  Minimal block textures used by the viewer.
- `data/viewer_data/runs/`
  Viewer run store with the included `游戏实测2` sample.
- `docs/`
  Configuration format, model notes, and Rust architecture documentation.

## Runtime Data

- Viewer-visible runs are stored in `data/viewer_data/runs/`.
- Search diagnostics are written to `data/reachability-candidate-generator/`.
- Generated data is local to this project folder.

## Configuration

`model/config/waterway-structure-parts.json` documents the structure-parts format used by the project. The current Rust search catalog is still compiled into `rust-backend/src/lib.rs`; the JSON file is kept as the external format reference.

## Documentation

- [Structure parts config](docs/waterway-structure-parts-config.md)
- [Minecraft 1.17.1 item waterway model](docs/item-waterway-model-1.17.1.md)
- [Rust architecture](docs/rust-architecture.md)
- [Structure parts JSON](model/config/waterway-structure-parts.json)
