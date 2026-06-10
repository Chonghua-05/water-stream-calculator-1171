# Water Stream Calculator 1.17.1

Standalone Rust-based toolkit for Minecraft 1.17.1 item water-stream simulation, reachable-candidate search, and local run inspection.

This repository is the stripped public package for the water-stream calculator only. It does not include Minecraft source, Fabric bridge code, or historical local runtime clutter.

## Included

- `viewer/`
  Static web UI for structure editing, simulation, run inspection, and search.
- `rust-backend/`
  Rust backend source for `serve-web`, simulation, run storage, and reachable-candidate search.
- `model/config/waterway-structure-parts.json`
  Published structure-parts config snapshot and grammar reference.
- `assets/minecraft/textures/block/`
  Minimal block textures used by the viewer.
- `data/viewer_data/runs/`
  Sample viewer dataset containing only `游戏实测2`.
- `docs/`
  README, config format doc, model doc, and Rust architecture doc.

## Quick Start (Windows)

1. Optional rebuild from source:

```powershell
cargo build --release --manifest-path .\rust-backend\Cargo.toml
```

2. Start the local service:

```powershell
.\start-windows.ps1
```

3. Open:

```text
http://127.0.0.1:8766
```

The viewer starts with one sample run already available:

- `游戏实测2`

## Runtime Layout

- Viewer static files are served from `viewer/`
- Sample and generated runs live under `data/viewer_data/`
- Search artifacts are written under `data/reachability-candidate-generator/`
- Minimal textures are served from `assets/minecraft/textures/block/`

## Notes

- The runtime stack in this repo is Rust + static web assets only. There is no Node or Python backend.
- A bundled Windows solver binary is included under `bin/windows/`. If the Rust source is newer and `cargo` is available, `start-windows.ps1` will rebuild and use the fresh binary.
- The published structure-parts JSON is included as the external config reference. The current Rust search catalog is still compiled into the backend and is documented in the architecture note.

## Key Files

- [`docs/waterway-structure-parts-config.md`](docs/waterway-structure-parts-config.md)
- [`docs/item-waterway-model-1.17.1.md`](docs/item-waterway-model-1.17.1.md)
- [`docs/rust-architecture.md`](docs/rust-architecture.md)
- [`model/config/waterway-structure-parts.json`](model/config/waterway-structure-parts.json)

