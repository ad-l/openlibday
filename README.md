# openlibday

An open-source reimplementation of the engine for **Liberation Day** (1998),
the turn-based isometric tactics game. openlibday aims to play the original
game using data from a legally owned installation, on Windows, Linux and mobile.

openlibday contains **no original game assets or executable code**. You need an
existing Liberation Day installation; the engine reads its databases, artwork,
animation, video and sound files at runtime.

## Status

Milestone M2 (map viewer) complete: every skirmish map renders natively
(terrain, roads/bridges, walls, mines, buildings, units, reinforcements and
deployment zones) with exact draw-order parity against the Python map editor.
The battle interface is in place: right-hand sidebar (minimap, unit
selector, action buttons) and bottom bar, drawn from the original
`INTERF\1024X768.SMK` art and resizable with the window. The minimap, MAP
overview (units in player colours, per-player unit lists that blink on the
map) and GOAL dialog work; the other buttons are not wired up yet.
Units move: a click previews the cheapest path in action points (dots,
crossed beyond the unit's AP), a second click walks it with the original
walk animations; overwatch keeps a shot's AP in reserve.
Battles save and load as versioned JSON (`F5`/`F9` quick save/load), and
battle saves of the original game (`SAVEGAME\savegame.NN`) can be opened or
converted.
DBI sprites, Jet 3 databases, ZIP archives and PE string tables decode natively. The first goal is playable
stand-alone skirmishes; campaign (story mode) and multiplayer will follow.

See [docs/PLAN.md](docs/PLAN.md) for the architecture and roadmap.
Implementation: Rust + SDL3. License: MIT (code only; no game data included).
The bundled Liberation Sans font (`crates/libday-view/assets/fonts`) is under
the SIL Open Font License 1.1.

## Building and running

Requires stable Rust (1.88+), CMake and a C compiler (MSVC Build Tools on
Windows); SDL3 is built from source and linked statically.

```powershell
$env:LIBDAY_ROOT = 'F:\libday'      # your Liberation Day install directory
cargo test --workspace              # data-gated tests skip without LIBDAY_ROOT
cargo run -p openlibday             # map viewer (first skirmish)
cargo run -p openlibday -- s3-4-1   # a specific skirmish (file name, with or without .mdb)
cargo run -p openlibday -- --load 1          # original save SAVEGAME\savegame.01 (or a .json/dir path)
cargo run -p libday-tools -- skirmishes
cargo run -p libday-tools -- save-list       # original savegame slots (battle or campaign-only)
cargo run -p libday-tools -- save-info 1     # summary of a slot number, directory or .json save
cargo run -p libday-tools -- save-import 1 battle.json
cargo run -p libday-tools -- map-render s1-2-1 -o map.png --grid
cargo run -p libday-tools -- map-scene s1-2-1 --deployment   # draw list as JSON
cargo run -p libday-tools -- dbi-list TERRAINS.DBI
cargo run -p libday-tools -- dbi-export UB.DBI out\ub
```

Map viewer controls: mouse wheel or `+`/`-` zoom (anchored on the cursor),
drag with any button or arrow keys to pan, `Home` recentre, `PgUp`/`PgDn`
previous/next skirmish, `G` grid, `D` deployment zones, `U` units, `T` terrain
only, `[`/`]` show reinforcements up to an arrival turn, `M` MAP overview,
`O` GOAL dialog, `I` cycle interface scale, `F5` quick save, `F9` quick
load, `Esc` back to battle or quit.
Click a unit to select it; with a unit selected, click a tile to preview
its path and click it again (or double click) to move. The OVERWATCH box
toggles the reserve. Click the minimap to move the view. The title
bar shows the hovered tile's terrain, building and unit. Set
`OPENLIBDAY_TRACE_EVENTS=1` to log SDL events to stderr.

Options (interface scale, default 2x; initial window size; fullscreen) are
read from `openlibday.ini` in the user preferences directory
(`%APPDATA%\openlibday\` on Windows, `~/.local/share/openlibday/` on Linux),
or the file given with `--options FILE`. A commented default is created on
first run; `openlibday --help` prints its location. The interface scale is
lowered automatically when the window is too small for it. Quick saves go to
`saves\quick.json` in the same directory.

A save stores the battle's source scenario (a stock `.mdb`, or the original
save's own `SCENARIO.MDB` when it is not a stock map), the turn clock,
difficulty, per-player deploy points and kill/loss tallies, every unit,
building, wall, road, mine, ruin and surface with its current state, and the
terrain cells that differ from the source (destroyed tiles). See
"Savegames" in [tools/modding/README.md](tools/modding/README.md).

| Crate | Purpose |
|---|---|
| `libday-formats` | Decoders: DBI sprites, INI, Jet 3 (Access 97), ZIP, PE string tables, CP1252, FF containers, Smacker video; DBI/FF/Smacker encoders for modding |
| `libday-data` | Locates an installation (`LIBDAY_ROOT`, `LIBDAY.INI`), case-insensitive lookup, databases, scenarios inside `SCENS\*.ZIP`, UI strings; typed GLOBAL.MDB definitions, scenario loader, skirmish catalog |
| `libday-view` | Renderer-independent map scene (isometric projection, draw order, sprite choice) and a software compositor |
| `libday-sim` | Deterministic headless simulation: movement rules and AP pathfinding (`path`) |
| `libday-tools` | `libday` CLI for inspecting/exporting data |
| `openlibday` | SDL3 application |
| `tools/modding` | Python/Tk map and scenario editor and unit/building asset editor (see its README) |

## Modding tools

`tools/modding` holds the map/scenario editor and the unit/building asset editor
(game data per tech level, sprites, Smacker animations). They need Python 3.10+
with Tk (standard Windows installer) and a built `libday` CLI (`cargo build`);
point them at your install with `LIBDAY_ROOT`. Launch `Map Editor.cmd` or
`Asset Editor.cmd`, and see [tools/modding/README.md](tools/modding/README.md).
The map editor writes scenario databases through 32-bit Windows Jet, so it is
Windows-only for now.

## Knowledge base

Reverse-engineering notes, file formats and observed native behavior are in
[tools/modding/README.md](tools/modding/README.md) ("Engine reconstruction
knowledge base"). Findings that the engine relies on are summarized in
`docs/` as they are implemented.
