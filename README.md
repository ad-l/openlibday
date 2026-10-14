# openlibday

An open-source reimplementation of the engine for **Liberation Day** (1998),
the turn-based isometric tactics game. openlibday aims to play the original
game using data from a legally owned installation, on Windows, Linux and mobile.

openlibday contains **no original game assets or executable code**. You need an
existing Liberation Day installation; the engine reads its databases, artwork,
animation, video and sound files at runtime.

## Status

Milestone M1 (asset layer) complete: DBI sprites, Jet 3 databases, ZIP archives
and PE string tables decode natively. The first goal is playable
stand-alone skirmishes; campaign (story mode) and multiplayer will follow.

See [docs/PLAN.md](docs/PLAN.md) for the architecture and roadmap.
Implementation: Rust + SDL3. License: MIT (code only; no game data included).

## Building and running

Requires stable Rust (1.88+), CMake and a C compiler (MSVC Build Tools on
Windows); SDL3 is built from source and linked statically.

```powershell
$env:LIBDAY_ROOT = 'F:\libday'      # your Liberation Day install directory
cargo test --workspace              # data-gated tests skip without LIBDAY_ROOT
cargo run -p openlibday             # M0 demo: terrain tile mosaic (drag/arrows to pan)
cargo run -p libday-tools -- dbi-list TERRAINS.DBI
cargo run -p libday-tools -- dbi-export UB.DBI out\ub
```

| Crate | Purpose |
|---|---|
| `libday-formats` | Pure decoders: DBI sprites, INI, Jet 3 (Access 97), ZIP, PE string tables, CP1252 |
| `libday-data` | Locates an installation (`LIBDAY_ROOT`, `LIBDAY.INI`), case-insensitive lookup, databases, scenarios inside `SCENS\*.ZIP`, UI strings |
| `libday-sim` | Deterministic headless simulation (placeholder) |
| `libday-tools` | `libday` CLI for inspecting/exporting data |
| `openlibday` | SDL3 application |

## Knowledge base

Reverse-engineering notes, file formats and observed native behavior are in the
companion modding project (`libday/modding/README.md`, "Engine reconstruction
knowledge base"). Findings that the engine relies on are summarized in
`docs/` as they are implemented.
