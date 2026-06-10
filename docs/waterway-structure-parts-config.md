# Waterway Structure Parts Config

The published structure-parts config snapshot lives at:

```text
model/config/waterway-structure-parts.json
```

It documents the searchable waterway grammar used by the project family:

- prefix atoms
- reachability modules
- cycle layouts

You can point tools at another file with:

```powershell
$env:WATERWAY_PARTS_CONFIG = "D:\path\to\my-waterway-parts.json"
```

## Segment Syntax

Each part is built from `segments`.

```json
{ "kind": "dry", "length": 2, "floors": ["blue_ice"] }
{ "kind": "still", "length": 1, "floors": ["packed_ice"], "amount": 8 }
{ "kind": "flow", "length": 3, "direction": 1, "floors": ["packed_ice"] }
```

Meanings:

- `dry`: no water
- `still`: source/still water
- `flow`: flowing water gradient
- `length`: number of cells
- `floors`: floor pattern, repeating if more than one value is given

Supported floors in the current model family:

- `normal`
- `packed_ice`
- `blue_ice`
- `slime`

## Prefix Atom Example

```json
{
  "name": "F3B",
  "segments": [
    { "kind": "flow", "length": 3, "direction": 1, "floors": ["blue_ice"] }
  ]
}
```

## Reachability Module Example

```json
{
  "name": "D4B",
  "role": "phaseAdjust",
  "cost": 4.5,
  "stage": 2,
  "segments": [
    { "kind": "dry", "length": 4, "floors": ["blue_ice"] }
  ]
}
```

Roles in the historical grammar:

- `accelerator`
- `brake`
- `phaseAdjust`
- `stabilizer`

`stage` is the old search-order constraint used by the original generator grammar.

## Cycle Example

```json
{
  "name": "W3-I_D2-B",
  "note": "Custom compact dry-gap cycle.",
  "segments": [
    { "kind": "flow", "length": 3, "direction": 1, "floors": ["packed_ice"] },
    { "kind": "dry", "length": 2, "floors": ["blue_ice"] }
  ]
}
```

## Important Boundary

This repository ships the JSON config and its format documentation as the published grammar reference.

Today, the Rust backend still carries its active search catalog in code for compatibility with the current search pipeline. That means this JSON is the external config reference, but not yet the single runtime source of truth for the Rust searcher.

