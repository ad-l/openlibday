# Liberation Day modding tools

This README also serves as the **engine reconstruction knowledge archive**.
The consolidated reference below records discoveries through **2026-10-13**.
Start with [Engine reconstruction knowledge base](#engine-reconstruction-knowledge-base)
for file formats and behavior, [Disassembly anchors](#disassembly-anchors)
for native code locations, and the editor sections for current tooling.
Older experiments are retained as historical evidence, not current blockers.

These tools live in the openlibday repository under `tools\modding`. They
contain **no game assets**: every registry (`*-types.csv`), artwork database
and test fixture is generated locally from an original Liberation Day install.
Point the tools at that install with `LIBDAY_ROOT` (or `--game-root`, or the
folder picker shown on first launch). Decoding and encoding of game formats
(Jet MDB, ZIP, DBI, FF, Smacker) is done by the Rust `libday` CLI, which the
tools locate under `target\{debug,release}`; build it once with `cargo build`.

The tools were developed against a working install at `F:\libday`; paths such
as `modding\archive` or `modding\green-skirmish` in the historical sections
below refer to that original install's `modding` folder, which was not moved.

## Directory layout

| Location | Contents |
| --- | --- |
| `map_editor.py`, `map_editor_core.py`, `Map Editor.cmd` | Graphical map/scenario editor, shared model/artwork decoder and launcher |
| `asset_editor.py`, `asset_editor_core.py`, `Asset Editor.cmd` | Unit/building data viewer and sprite/animation editor (see [Asset editor](#asset-editor-units-and-buildings)) |
| `scenario_scripts.py`, `scenario_script_editor.py` | Lossless native script model, diagnostics/timeline and map-picking dialog |
| `map_tool.ps1`, `libday_mod.py` | Jet database write bridge (32-bit PowerShell) and shared archive/staging helpers |
| `registries.py` | Generates the `*-types.csv` registries from `GLOBALS\GLOBAL.MDB` via the CLI and runs CLI commands |
| `testsupport.py`, `test_*.py` | Regression tests; skipped unless `LIBDAY_ROOT` is set. Fixtures are extracted into `.fixtures\` |

Run the tests from this directory with `python -m unittest` or
`python run_tests.py` (about 5 s). The slow Jet round-trip and Tk GUI tests are
skipped unless `LIBDAY_SLOW_TESTS=1` is set; run them before committing editor
changes. `python run_tests.py --slow` runs each slow test in its own process
(default `--jobs` is half the CPU count) and takes about 2 minutes instead of
10 for the sequential `LIBDAY_SLOW_TESTS=1 python -m unittest`. Extra arguments
filter test ids by substring, e.g. `python run_tests.py --slow test_map_resize`.

`test_engine_parity.py` (slow, about 6 s) checks the Rust engine renderer
against the editor: it compares every image item on the Tk map canvas (sprite
and position, in stacking order) with `libday map-scene` for three skirmishes,
with and without deployment zones. It uses the newest of
`target\{release,debug}\libday.exe` (or `LIBDAY_CLI`) and skips if none is built.

## Current status

### Graphical map editor: terrain and object layers

Launch **`Map Editor.cmd`** by double-clicking it, or run:

```powershell
python .\tools\modding\map_editor.py
```

To open a scenario directly, or a specific scenario inside an archive:

```powershell
python .\tools\modding\map_editor.py --open F:\libday\modding\green-skirmish\s1-99-1.mdb
python .\tools\modding\map_editor.py --open F:\libday\SCENS\SCEN1.ZIP --member s1-3-1.mdb
```

The Windows/Tkinter application uses Python's standard library only, including
Tk supplied by the normal Windows Python installation. Pillow is not required.
It reads the installed `IMGS\TERRAINS.DBI`, `UB.DBI`, `ROADS.DBI`, `WALLS.DBI`,
`RUINS.DBI` and `SYMBOLS.DBI` palettes and compressed sprites to show actual terrain,
connected roads/walls, deployment zones, debris and composed, multi-tile buildings on an isometric
canvas. The shared decoder supports the installed v3 54x56 artwork databases
and validates their fixed records and name indexes, including the shortened
final transparent run. Building definitions are exported locally from the
game's global database into `building-types.csv`. Road, wall and debris definitions
are exported into `road-types.csv`, `wall-types.csv` and `decor-types.csv`.
`unit-types.csv` contains 217 native type/technology definitions for 62 unit types.

**Open scenario** accepts a standalone scenario MDB or a ZIP archive. For ZIPs,
a searchable member picker selects which map to edit. Ground textures can be
searched by ID and filtered by terrain family. Choose **Ground**, **Trees**,
**Scenery**, **Buildings**, **Roads**, **Walls**, **Decors**, **Units** or **Deployment** in the sidebar.
There are 355 clear-ground textures, 175 forest textures, 354 scenery tiles,
81 building types, two road types, four wall types and 136 registered debris
types. The extra, unregistered ruin artwork is not offered for placement.

**Alternate map** switches between the two complete campaign-map variants:
unchecked opens `sC-P-1.mdb`, checked opens `sC-P-2.mdb`. Each keeps its own
pending edits and undo/redo history while you switch. The title's unsaved
marker covers both; the heading warns when the other variant has unsaved
edits. **Save Copy** and **Export skirmish** save only the currently displayed
variant. Opening another mission or closing asks about either variant's
unsaved edits. This checkbox is a document selector, not a saved scenario
flag; it does not change installed campaign or savegame state.

The editor first looks for the paired member in the opened ZIP, then in the
neighboring `SCEN1.ZIP` or `SCEN2.ZIP`. Standalone MDBs require the
native-named paired MDB beside them. No partner is silently taken from a
different installed-game directory. Missing partners disable the checkbox;
renamed standalone copies such as `s1-2-1-edited.mdb` and registered `edit1.mdb`
no longer follow the native pairing convention. Base/city and demo maps
normally have no alternate. Invalid peer archives are reported without
preventing the selected map from opening.

**Deployment** paints the four authored starting-zone slots. Select **Player
1..4** in the native-sprite palette or the player selector, then left-drag
with the existing **1x1 / 3x3 / 5x5** brushes. **Paint** assigns tiles to that
player, transferring them from another player's starting zone if necessary;
**Erase** removes only the selected player's cells. Right-click a starting
zone to pick its player. Tile counts are shown for all four slots.

Zones are **hidden by default**. Entering Deployment mode reveals them;
leaving it hides them. The toolbar's **Deployment zones** checkbox can
independently show or hide them in any mode. Markers use the game's four
`DEPL1xx` through `DEPL4xx` sprite families, with borders connected only to
the same player's neighboring cells. All zoom levels and shared undo/redo
work for this layer. Save Copy and registered skirmish export include it.

Only human slots 1..4 at **Turn 0** are editable. AI, unknown-player and
nonzero-turn rows remain untouched and prevent assigning a new zone over
their coordinates. Existing native counters survive transfers; Jet assigns
counters for new cells. Painting is not a unit-placement check: zones can
cover trees, water or objects without changing them. Other placement tools
retain their conservative deployment reservations, updated as zones are
painted/erased. The game can merge slots for fewer active players, so these
are **authored slots**, not a preview of the merged solo/multiplayer result.
Empty base/city zones can be runtime-generated; the editor does not fill
them automatically or change campaign generation or deployment points.

**Scenario...** opens the scenario-settings dialog. Edit the English name,
turn limit (1..100), English briefing/objective text, and the skirmish's
deployment points budget (0..32767 native points). The **Available army units**
tab offers the 39 native recruitable military types: click a row to toggle it,
and search by name or type ID. Selection survives filtering. Availability
controls the player's army-setup choices, independently of units placed on
the map. Automated/support-unit availability, existing technology rules and
upgrade costs are preserved; the player still allocates the points in-game.

The native skirmish selector derives its difficulty label from
`One Off Deployment`: **Easy** up to 300, **Average** 301..500,
**Hard** 501..1000, and **Very hard** above 1000. There is no independent
per-map difficulty field. Choosing a preset sets the budget to 150, 400, 750
or 1500 respectively; entering a custom budget updates the label. The game
derives a separate technology allocation from this deployment budget;
they are not one shared pool. A live exported demo with a budget of 420
showed 2100 in technology setup and 420 for deployment.
The campaign-wide Easy/Hard settings are unrelated and are not changed.

Briefing line breaks are displayed normally and saved using the game's `##`
markers. Editing the text does **not** rewrite actual victory conditions,
objective IDs, scripted objective deadlines, AI orders, reinforcements or
deployment zones. A different turn limit does not automatically retime those
scripts. Other languages and aftermath text remain untouched.
**Apply** commits one shared undo/redo operation; **Cancel** discards the
dialog's changes. Settings participate in the same dirty state and save
snapshot as every map layer.

**Scripts...** opens a nonmodal native scenario-script editor with four tabs:
**Timeline / diagnostics**, **Scheduled support**, **AI orders**, and **Objective**.
The inspector shows the six native script tables, starting-unit AI fields and
ordinary reinforcement rows without rewriting their enumeration or legacy
duplicate/orphan records. The timeline distinguishes initial placements, exact-turn support
requests, reinforcement arrivals, inclusive order expiry and the raw Hold
threshold. Diagnostics warn about missing/future targets, unused orders,
unverified modes, nominal landing obstruction and events beyond the turn limit.

Scheduled support offers stock-used bombs, gas/nuclear strikes, aircraft drops
and transports. Choose a native type and arrival turn **1..99**, then pick the
effect/source tile on the main map; transports additionally have a destination.
Non-transports synchronize their destination to the effect tile. Payload and
damage properties come from the read-only **`support-types.csv`** registry, a
cached factual subset of all 27 GLOBAL definitions. Drop previews show the
nominal offsets and military footprints, not guaranteed survivors or technology.
These requests belong to native AI owner 0; they are not ordinary reinforcements.
Targets may overlap objects because scheduled effects are not static obstacles.

For initial units, use the **AI orders** tab or **Edit unit AI order...**
in unit properties. Pick a starting unit and choose No Order, Goto, Goto Fast,
Escort, Defend, Patrol or Attack, its **Active through turn**, retention value
and preferred enemy military type. Pick unit/building IDs for Escort/Defend/Attack,
or paint tile collections for Goto/Goto Fast/Patrol using the map's brush size.
Tiles are not an ordered waypoint route. Unused existing collections remain
until explicitly cleared. Clear an active order and its references before
converting its unit to a later arrival. Reinforcements instead expose
**Use objective-derived AI order**, whose behavior follows the current objective.
Both initial and reinforcement IDs appear in the AI orders selector, labeled
with their initial owner or reinforcement arrival turn. Selecting one switches
the map's **Units at** view to its arrival and highlights its source in green.
For a reinforcement, only **Use objective-derived AI order** is editable:
the order, targets, expiry 99 and retention 100 are read-only previews of the
native spawn helper. Apply saves the native Boolean in `[Reinforcement]`,
not invented OrderID/target rows. Target overlays use the applied flag; changing
the checkbox previews the form until Apply. Disabling it requests no generated
order. The engine has no per-reinforcement custom-order fields; those would
need an engine change, not just an editor control. Scripted aircraft drops
remain separate support events.

The **Objective** tab edits the single verified campaign preset (kinds 0..6),
its relevant target IDs/tiles and Hold Position's raw threshold. Other deadlines
use the global turn limit in Scenario settings; briefing text remains separate.
Internal counterattack/deathmatch objectives, unverified AI orders and unsupported
support type/turn combinations are preserve-only. There is no arbitrary trigger
language, conditional graph or multiple-objective authoring.

**Script targets** are hidden by default. When checked, native red crosshairs
(`SYMBOLS.DBI:OBJT001`) mark objective-listed unit/building IDs, centered on
their complete footprints, and the native red tile overlay (`OBJT000`) marks
objective positions such as escort destinations and capture regions. Unit
crosshairs follow the selected **Units at** arrival view. Selected AI-order
targets keep their separate outline/ID annotations. These overlays use applied
script records and refresh with edits, undo/redo and zoom; they do not alter the
scenario or simulate objective completion.
While the script window is open, its active tab filters the overlay:
**Objective** shows only objective crosshairs/red regions; **Scheduled support**
shows only support source/effect/destination markers and the selected event's
nominal drop footprint; **AI orders** shows only target IDs/tiles belonging to
the selected unit (including applied objective-derived reinforcement targets),
with that source unit's footprint outlined in green
to distinguish it from its targets. **Timeline / diagnostics** retains the combined
view, as does closing the window. Switching tabs stops map picking so an old
tab's draft cannot be edited accidentally. The **Script targets** checkbox
still controls applied overlays; active picking temporarily shows draft markers.
**Done picking** or Escape stops picking, and the erase checkbox removes draft
region tiles. Normal pan/zoom still work. Picking/form changes do not alter the
document until their **Apply** button is used. Apply commits one atomic shared
undo/redo entry; closing or refreshing discards unapplied form values. Changing
map variants closes the dialog, retaining each variant's applied scripts/history.
Remove target references explicitly before deleting units/buildings. Applied
scripts participate in Save Copy and registered skirmish export, including the
immutable background-save snapshot. Live-visible turn boundaries, landing
interception and complete objective outcome handling remain unverified; the
timeline and overlays do not simulate them.

**Save copy...** includes settings in the edited MDB or ZIP but does not change
the installed game catalog. For a new selectable skirmish, use **Apply and
export skirmish...** and name a **new, nonexistent folder**. Export creates
`SCENS\SCEN1.ZIP` with a uniquely named added scenario and a matching
`SCENS\SKIRMISH.MDB` catalog copy. The new entry takes its names, descriptions,
points and actual map size from the edited scenario. All existing archive
members and catalog entries are preserved. These files can be used in an
isolated installation; export never installs them or overwrites existing
folders. The two files must be used together for the game selector to show
the edited name, briefing and difficulty.

Live verification of **Green Thaw - Settings Demo** reached technology setup,
deployment and the playable battle. Its four enabled military types
(Trooper, Grenadier, LAV and Buggy) appeared in both setup and deployment;
placing two Troopers reduced the deployment budget from 420 to 410.
The battle displayed the edited 12-turn limit. Actual win-condition rules
were preserved, not retimed to that limit.

Trees are terrain tiles, not independent objects: planting a tree replaces the
whole tile, including its background. In Trees mode, enable **Remove trees**
and choose the explicit **Replacement ground** texture to erase them. Ground
mode still protects trees and scenery. **Scenery** paints rock/mountain/crack
tiles and overlapping terrain props. It also has an explicit erase-to-ground
tool. These are terrain tiles, not movable objects; matching mountain/crack
pieces must be arranged manually. Water and coast tiles remain protected
in ordinary terrain modes; **Bridges** provides the restricted bank-editing exception.

In **Ground**, **Terrain mode (hide objects and vegetation)** temporarily hides
buildings, units, roads, walls, decors, mines, deployment zones and script markers.
Trees/scenery are displayed as plain-ground placeholders from their terrain
family, falling back to the selected ground brush. Native vegetation tiles contain
their own background, so this is not a recovered underlying terrain layer.
The checkbox never changes saved data or painting protections: erase trees/scenery
explicitly before painting those tiles. Leaving Ground restores normal rendering.
Entering terrain-only view stops script picking; starting script picking restores
normal rendering so its targets are visible.

**Resize map...** in Ground changes the square side length (1..256 tiles).
Coordinates stay anchored at (0, 0): growth adds rows/columns at increasing X/Y,
filled with the chosen clear-ground texture; shrinking removes those edges.
Resizing is one undoable operation and applies only to the active map variant.
Shrinking is blocked when it would cut building/unit footprints, roads, walls,
decors, mines, deployment records, script positions, support source/destination
tiles or preserved spatial records. Move/remove affected editable records first.
Maps containing unverified `Taurans On Map` records conservatively refuse shrinking.
Untouched legacy records already outside the existing map, including off-map HQ
coordinates found in stock scenarios, are preserved rather than moved or deleted.
Native IDs for retained cells remain unchanged; new cells receive fresh IDs above
the original/current maximum. Regrowing original coordinates reuses their original
IDs. Save Copy writes the resized grid transactionally; registered skirmish export
also updates the exported catalog's `Size`. Stored overview artwork is not
regenerated. Arbitrary map dimensions have not been verified in the running game.

**Roads**, **Walls** and **Decors** are independent per-tile object layers.
Choose a palette type and **Paint**, then drag; **Erase** removes that layer
without changing the terrain beneath it. One drag is one undo operation.
Road and wall sprites reconnect immediately after painting, moving, deleting
or undoing. Connections match the game's four-direction lookup and join only
the same type; ownership does not affect joining. Road/wall drags insert
orthogonal steps instead of disconnected diagonal tiles. Paved roads also support
straight bridges on existing deep-water and riverbank tiles. Unsupported
bridge junctions are rejected rather than shown with invented artwork.

**Mines** supports normal and grav mines, with the same **Paint / Erase /
Select / Move** workflow and shared undo/redo. The inspector saves owner,
**Armed**, and separate **Hidden from AI / P1 / P2 / P3 / P4** flags.
Initial brush flags match stock scenarios: armed, visible to AI, hidden
from all four human slots. Changing owner does not silently change visibility.
All mines remain visible in the editor using native `MINE000` / `GMINE000`
static artwork; this is not an arming animation or fog-of-war preview.
New/moved mines require clear plain land and cannot overlap units,
reinforcements, buildings, walls, debris or deployment reservations.
Roads and mines may share a tile. Property-only edits preserve legacy
placements; erased/moved mines stop reserving their old tile immediately.

**Bridges** shows terrain-generated construction spans using native bridge
markers. The overlay is hidden outside this mode. Select riverbank artwork
and **Paint banks** on existing deep-water/riverbank tiles; **Remove banks**
replaces riverbank tiles with the explicitly selected deep-water texture.
These are real terrain changes, not an independent saved permission mask.
Plain land must lie behind both banks; intervening cells must be deep water.
The native generator also accepts immediately adjacent banks, prefers the
positive X axis, and prunes nearby spans, so removing one bank can expose a
neighboring candidate. Brush drags, right-click texture picking and shared
undo/redo work here. Existing objects/deployment reservations are protected.
Already-constructed bridges remain editable in **Roads**.

Markers represent terrain-derived candidates, not a guarantee that every
marked tile is currently constructible. Runtime occupancy, adjacent roads
and the available construction support still affect gameplay. Save Copy,
ZIP saves, alternate-map documents and registered skirmish export retain
both mine records and bank terrain without changing the installed files.

**Units** edits regular starting units and scheduled reinforcements. **Units at**
in the toolbar selects **Start** or **Turn 1..100**: the map shows the placements
scheduled for that arrival, and new palette placements use that turn. This is
an arrival-layer view, not a simulation of previous turns, movement or casualties.
Use Turn 2 or later for arrivals after the first turn. Original reinforcement
records with native turn 0 are retained and shown at Start.

Choose a unit type in the palette, then **Place** and click a compatible tile.
**Select** or right-click inspects a unit; repeated clicks cycle original units
sharing a footprint. Edit its type, arrival or position and **Apply properties**;
**Move** uses a destination click. Starting units also expose owner (0 = AI,
1..4 = players), a supported technology level, facing (0..7), health and action
points. **Delete unit** and shared undo/redo work on both arrival categories.
The inspector uses a compact layout to leave the palette accessible at high DPI.

Starting units are stored in `Units On Map`. Its existing `Turn` field is
**inclusive AI-order expiry, not arrival**: normal property edits preserve it;
the script dialog edits it together with the other verified AI/order fields.
Later arrivals are stored separately in
`Reinforcement`, with stable `UnitID`, type, position, `turn` and `order`.
The arrival-order checkbox exposes that table's objective-derived AI-order flag.
The game always creates these reinforcements for **owner 0 (AI)** and assigns
their technology, facing, health and AP at runtime; those inspector properties
are disabled rather than saved to nonexistent fields. Player-owned units must
start on the map. Converting a starting AI unit to a reinforcement or back
retains its game ID; Jet assigns a new counter when changing tables.

Placement checks include complete multi-tile unit footprints, map bounds,
compatible movement/terrain and same-arrival occupancy. Land vehicles need clear
land, infantry can also use forest, boats/submarines/platforms need deep water,
grav units can use clear land/water/riverbanks, and flying units ignore terrain
obstacles. These are conservative editor checks: buildings, blocking debris,
walls, mines and deployment reservations are still avoided.
Roads and passable debris can share a unit tile. Different arrival layers may
share a location, but actual arrivals can be delayed if it remains occupied
in-game; the editor does not guarantee playability or extend the mission turn limit.
Deleting or changing the starting/reinforcement category of units referenced by
objectives or orders is blocked. Existing AI-order links are never silently deleted.

Right-click an object in its active layer, or choose **Select** and click it.
Edit its type, coordinates and, for roads/walls, owner; click **Apply properties**.
**Move** uses a destination click, and **Delete object** removes the selection.
Walls also expose armor/health and the original `isDamaged` flag as **Broken
concrete section**. The game selects damaged artwork using a **33% health threshold**,
not that flag, but the editor's **low-HP damage preview is disabled**.
Broken concrete sections still use the game's alternate end-cap shapes.
The flag cannot be enabled on other wall types whose artwork lacks those shapes.
The inspector shows the stored MDB health out of the type's maximum and explains
that the map preview uses intact HP artwork. The original Green Thaw concrete
walls store 10 health out of 60, but the game resets their health during fresh-battle
initialization, as explained below. Walls and buildings therefore show intact
artwork regardless of stored HP, without changing that HP or the `isDamaged` flag.
**Full health** still fills the wall inspector; **Apply properties** explicitly
updates the stored health. Neither opening nor rendering repairs scenario records.

**Stored health versus battle-start health.** The editor's property controls display and
edit the MDB's `currentAR`, preserving the source data. Its artwork intentionally
ignores low HP to avoid confusing stored health with fresh-battle appearance.
The native MDB constructors initially read that value,
but a later battle/deployment initialization pass resets buildings and walls to
their type's maximum armor. A human Headquarters can therefore store 100 while
starting a fresh battle at 500 HP; concrete walls can store 10 while starting at
60 HP. The installed global definitions specify concrete 60, laser fence 150,
force field 300 and ruined walls 5. This is a full-health reset, not a universal
five-times conversion or a health percentage.

Consequently, editing these stored health values alone does not establish
damaged starting buildings/walls in a fresh battle. The separate concrete `isDamaged` geometry
flag is not cleared by the health-reset routines. The loader also has a
conditional reset pass covering military-unit health/AP; these stored properties
must not be assumed to override every game-start path. Saved-state loading must
be distinguished from fresh-scenario initialization rather than normalizing
all MDB records on open. No health values in the source scenarios have been
changed to compensate for this behavior.

**Decors** contains building rubble and unit wreckage from `Ruins On Map`;
its `id` is a textual artwork/type name, **not a numeric game object ID**.
The inspector identifies whether a decor blocks movement. Original counters
remain stable on edits, and new road/wall/decor counters are assigned by Jet.

In Buildings mode, choose a palette type and click a valid **Place** footprint.
Use **Select** to click any tile of an existing building; change type, owner
(0 = AI, 1..4 = players), armor/health, position or AI-target status and click
**Apply properties**. To move a selected building, choose **Move** and click its
new origin tile. **Delete building** or the Delete key removes the selection.
The origin is the minimum X/Y corner, and the footprint extends toward increasing
coordinates. Existing game IDs and database counters are retained on edits;
new game IDs are allocated above the scenario's counter and occupied/referenced
IDs. Jet assigns new AutoNumber counters.

Placement is conservative: complete footprints must fit inside the map, use
clear land (or deep water for water buildings), and avoid other buildings and
reserved unit, road, wall, mine, ruin, deployment and reinforcement tiles.
Scheduled-support source/effect tiles do not reserve object placement. Types marked
`UniqueOnMap` cannot be duplicated. Existing placements are preserved, so
owner/health edits do not unexpectedly reject a pre-existing placement.
Deleting a building referenced by mission objectives or unit orders is blocked
in both the editor and database bridge.
Object occupancy is updated immediately when roads/walls/decors move or disappear.
New walls and decors avoid reserved unit/deployment cells and other objects.
Roads may run beneath reserved unit/deployment positions, but not buildings,
walls or decors. Property-only edits preserve pre-existing placements.

| Control | Action |
| --- | --- |
| Left click/drag (terrain or object Paint/Erase) | Paint or erase, with a 1x1, 3x3 or 5x5 brush |
| Left click (Buildings) | Select, place or move, according to the active tool |
| Left click (Units) | Select, place or move at the selected arrival turn |
| Left click (object Select/Move) | Select an object or move the selection |
| Right click | Pick terrain, or inspect an object in the active layer |
| Delete (map focused) | Delete the active layer's selected object, with confirmation |
| Middle drag or Shift + left drag | Pan the map |
| Wheel / Shift + wheel | Vertical / horizontal scrolling |
| Ctrl + wheel or Zoom selector | Change between 50%, 100%, 200% and 300% |
| Ctrl + Z / Ctrl + Y | Undo / redo all layers in the same history |
| Ctrl + O / Ctrl + S | Open a scenario / save an edited copy |

**Save copy** never overwrites an existing file, including the original input.
Terrain, building, road, wall, decor, unit and reinforcement edits are committed together in a
transaction on a copy; only their requested rows in `Map`, `Buildings On Map`,
`Roads On Map`, `Walls On Map`, `Ruins On Map`, `Units On Map`, `Reinforcement`, `Deployment` and,
when necessary, `NextID_Buildings` / `NextID_Units` in
`Scenario Info` change. Other rows, fields and tables are preserved.
When a ZIP was opened, choosing
a `.zip` destination repacks a copy of that archive with only the selected MDB
replaced; all other members are preserved. The loaded source is snapshotted
into temporary storage so external source changes cannot alter a later save.
Unsaved edits are indicated with `*` and confirmed before opening another map
or closing. Database operations run off the GUI thread, with explicit error
dialogs. Temporary working files are removed when the document closes.

The database bridge reuses `map_tool.ps1` via 32-bit Windows PowerShell and
the installed Jet OLE DB provider. It uses a **process-only**
`-ExecutionPolicy Bypass` for these local scripts, not a system policy change.
There is a 120-second timeout for each database operation.

Scheduled support (`Automated Units`) and verified objectives are editable in
the script dialog. The legacy `Taurans On Map` table and unexplained
army/internal scripting fields are preserved, but are not editable.
AI/unknown-player/nonzero-turn deployment rows are also preserved, not editable.
Unit previews use the lowest complete static artwork frame with the saved facing,
including composed 3x3 outposts/platforms; runtime animation, damage effects,
technology appearance and player-color remapping are not simulated.
Building previews use the lowest
complete artwork level, except Deploy Mk1..4, which use matching levels.
Scenario technology and player-color remapping are not simulated. HP-based damaged
wall/building artwork is not displayed; previews always use intact HP variants.
Existing ruin building types and the concrete `isDamaged` geometry flag retain
their own artwork. Lab Ruin (1005) and Tracks Shop (1017) also lack complete
damaged variants at their preview level, but this no longer requires an inspector
warning because damaged HP variants are not selected.

There is no automatic installation or stored
overview-artwork regeneration. The scenario dialog can register a new skirmish
in an exported catalog copy; it does not install that catalog.
To install an edited copy, close the game, use the existing packaging/catalog
tools where needed, preserve backups, and clear only that scenario's cached MDB.
Opening a ZIP and saving another ZIP edits the same scenario slot in the
output copy; it does not create a new selectable skirmish by itself.

The focused suite covers terrain artwork and PNG encoding, isometric hit
testing, ground/tree protection, mixed undo/redo, building IDs, footprints,
properties, collisions, reference protection, empty building lists, and real
Jet MDB/ZIP save-and-reopen operations. Building palette composition and
intact artwork blocks are checked against the installed assets.
The suite also checks all 16 connection masks, mixed-type boundaries,
ownership-independent joins, intact low-HP previews, broken concrete caps, bridge shapes/rejection,
dynamic occupancy, stable counters, duplicate original decor preservation,
scenery erasure, malformed object inputs, empty layers, GUI stroke/inspector
handlers and transactional MDB/ZIP roundtrips.
Mine/bridge coverage includes both mine types, every visibility flag,
stable counters, no-op preservation of complete native databases, atomic
mine removal plus unit placement, malformed-save rollback, bridge-axis/border
rules, native pruning and crossing exclusion, bank editing/occupancy,
native artwork, hidden-by-default overlays and captured save snapshots.
Unit coverage includes zero-based type IDs, technology-specific stats, all static
facing/footprint artwork, turn-layer occupancy, reference protection, conversion
between starting/reinforcement tables, ID exhaustion, malformed-save rollback,
minimum-window/high-DPI layout, inspector handlers and asynchronous saves.
The later campaign fixture is **World Cup** (`SCEN1.ZIP`, `s4-20-1.mdb`), which
has native reinforcement waves. Save-and-reopen checks preserve their existing
timing, IDs, database counters, order flags and unrelated archive members.

An earlier object-editing demonstration added and removed trees, added a building,
moved and changed another building's type/owner/health, and deleted an existing
building. Full database readback confirmed only the intended two terrain cells,
building rows and next-building counter changed; all 19 other tables and
overview blobs were preserved. Demonstration outputs are session artifacts,
not installed game modifications.

A separate road/wall/decor demonstration retains the original building layout.
It adds connected paved roads, laser-fence sections, building rubble, a unit
wreck and one scenery tile. Full-table readback confirms only `Map`, `Roads On Map`,
`Walls On Map` and `Ruins On Map` change; the other 18 tables, building positions
and overview blobs remain unchanged.

The `trees-buildings-demo.mdb` and `trees-buildings-ui.mdb` test copies do **not**
retain the original building layout: the original tech centre was deleted, and
the living pod was moved to `(5, 3)` and changed into a tech centre. For the
original layout, open `modding\green-skirmish\s1-99-1.mdb`: its tech centre
(ID 1000000) is at `(10, 14)`, beside the lower-left woods, and its living pod
(ID 1000001) is at `(8, 10)`. These are game coordinates, not screen pixels.

### Windowed mode in the main installation

`F:\libday\EXE\LIBDAY.EXE` now launches in a bordered 2048x1536 client
window using the locally available cnc-ddraw 7.1 wrapper. The executable
bytes are unchanged. `EXE\ddraw.ini` selects `windowed=true`,
`fullscreen=false`, and `border=true`; the game's `InWindow=0` is retained
because its native `InWindow=1` setting exited during startup on this desktop.
The wrapper handles the game's original 8-bit fullscreen requests without
changing the desktop resolution. This is windowed presentation/scaling, not
an expanded native viewport.

The output size is configured through `width=2048` and `height=1536` in
`EXE\ddraw.ini`. The game's `Resolution=1` still renders tactical gameplay
at 1024x768, scaled up 2x in each direction. Menus retain their original
640x480 rendering and are scaled to the same window. Mouse coordinates
are adjusted by the wrapper; no display-table patch is enabled.
After relaunching with these settings, the client area measured exactly
2048x1536 and the scaled main menu and skirmish selection responded to
MCP mouse input.

On the Windows desktop, the main menu, skirmish selection, unit upgrade
screen, scenario overview, and tactical deployment map rendered in that
window and accepted MCP mouse input at the initial 1024x768 output size.
That initial client area measured 1024x768
with a title bar and borders; the desktop remained at 3840x2160. This
does not establish a completed mission or resolve the separate native
widescreen investigation.

The original GOG wrapper is backed up as `EXE\ddraw.dll.pre-windowed`, and
the original game settings as `EXE\LIBDAY.INI.pre-windowed`. To undo the
change, close the game, restore those two backups to `ddraw.dll` and
`LIBDAY.INI`, and remove only `EXE\ddraw.ini`. The archived
`archive\playtest` configuration is byte-unchanged but retains its old paths.

### Installed skirmish: Testing Ground

Catalog Counter 55 (`s1-99-1.mdb` in `SCENS\SCEN1.ZIP`), first installed as
the 24x24 **Green Thaw** example (a grass-painted clone of Crime and
Punishment), is now **Testing Ground**: a 96x96 sandbox for scripted tests,
produced reproducibly by `make_testing_ground.py`:

```powershell
python tools\modding\make_testing_ground.py --root F:\libday --install
```

The generator keeps Green Thaw's database tables, stages the terrain of
s3-1-1 (rivers, lakes, banks, forests, mountains) into `[Map]`, then builds
everything else through the map editor model and saves with the editor's
validation (`Apply -ResizeMap square -BridgeTerrain banks`). Layout (x, y):

| Feature | Where |
| --- | --- |
| Player 1 | 3 of every unit type (186): land units x 1..33, y 24..95; water units in the north-west sea x 0..33, y 0..41; starting zone x 11..22, y 45..52 |
| AI | 1 of every non-human unit type (35): land units near (76, 20), water units in the south-east sea near (84, 74); every building type (81), land at x 56..95, y 0..41, water at x 72..95, y 52..95. At least 20 tiles (40 as generated) from any player unit |
| Built bridges | (35,33)-(35,41) and (67,46)-(67,51), paved, joined by the paved trunk road y 43 from x 4 and by paved approaches |
| Unbuilt bridges | (19,22)-(24,22) and (58,61)-(64,61): bank/water spans with dirt approaches ending at the banks |
| Dirt roads | (39,16)-(80,16), (21,13)-(39,13)-(39,15), (10,60)-(30,60)-(30,75)-(44,75)-(44,92) |
| AI mines, hidden from humans | normal x 37..42, y 64..68; grav x 37..42, y 80..84; on the paved road (46,43) and (48,43); x 48..50, y 18..20 |
| Player 1 mines | normal x 37..40, grav x 41..42, y 88..91 |
| Walls (AI) | concrete y 86, laser fence y 88, force field y 90, ruined y 92 (x 46..52); concrete x 56, y 20..26 |
| Props | up to three ruins/wrecks of every artwork family at x 20..54, y 1..9 (step 2); BORE scenery along y 11; a fresh forest patch x 46..51, y 33..37 |

The scenario has 100 turns, 0 deployment points (the original lists
skirmishes `ORDER BY [One Off Deployment]`, so this puts it first; the
stock minimum is 150) and every skirmish unit
type available. It also gets an **openlibday-only** `TechPoints` column
in `Scenario Info` (LONG, 0): the original game ignores it, but openlibday
uses it in place of the technology points derived from `One Off
Deployment`, and skips the technology screen when it is 0. The four
obsolete campaign `HQ on Map` rows are removed. The catalog row gets the
new title (all languages), description and size 96.

`--install` backs up `SCENS\SCEN1.ZIP` and `SCENS\SKIRMISH.MDB` once as
`*.pre-testing-ground` (these contain Green Thaw, and the golden-digest
tests read them in preference to the modified files), repacks the archive
member, updates the catalog and removes `CACHE\SCEN1\s1-99-1.mdb`. Re-running
regenerates from the backups. To undo, restore the two backups.
`scripts\testing_ground.txt` checks the result headless (run by `cargo test`
when the backups exist). The older Green Thaw files remain in
`modding\green-skirmish`.

| Feature | What is established | Limit |
| --- | --- | --- |
| Disassembly | `archive\libday.disasm.txt` contains the x86 executable disassembly; addresses below identify the display and map code | No source recovery or decompiler engine |
| 1920x1080 scaling | The original executable's main menu rendered at 1920x1080 with cnc-ddraw in the isolated copy | Tactical gameplay and reliable navigation are not established |
| Native 1920x1080 | The experimental table patch requests and recognizes the larger display mode; its loading screen was observed | No verified expanded map viewport; a later capture was black |
| Custom terrain | Export, validate, edit and repack square MDB terrain grids | Template dimensions, units, objectives and preview blobs remain unchanged |
| Custom map catalog | Testing Ground (formerly Green Thaw) is in the main skirmish catalog and plays in openlibday; a separate cloned deathmatch entry remains in the copied catalog | Full mission playthrough and the custom deathmatch remain unverified |

Early native-resolution experiments included loading screens, black/incomplete drawing, and
inconsistent navigation with synthetic input. They do not isolate a single
failure cause. In particular, a responsive process or a 1920x1080 window is not
proof of functioning tactical gameplay.

### Asset editor: units and buildings

Launch **`Asset Editor.cmd`**, or run `python .\tools\modding\asset_editor.py`
(`--game-root` overrides `LIBDAY_ROOT`). Pick a unit or building type on the left:

- **Game data** shows every `Unit Types` column with one column per tech level
  (0-10; most types define only some levels), grouped into general, combat,
  movement/detection, artwork and encyclopedia fields. Rows that change across
  tech levels are highlighted; lookup values are labelled (movement, race,
  category, weapon/damage type). Selecting a row shows its untruncated values in
  the status bar. A capability summary (movement, direct/indirect fire, area
  damage, AP drain, anti-air, overwatch, mine detection, stealth/low signature,
  convoy) heads the table. Buildings show their `Building Types` row.
- **Sprites** shows the static `IMGS\UB.DBI` sprites as composed footprints: units
  as frame x facing (8 facings), buildings as level x state. Click a cell to select
  it. Sprites export/import as 54x56 PNGs named after their DBI record; RGBA
  pixels are mapped to the nearest palette colour (lossless for exported files) and
  alpha < 128 becomes transparent. "Import folder" replaces every sprite of the type
  that has a matching `<name>.png`.
- **Animations** plays `ANMSUNIT\ANMSUNIT.FF` Smacker animations per player
  colour, action and facing (buildings: their destruction animation). Frames
  export as indexed PNGs plus `meta.json`; edit them, keeping the frame count and
  size, and import the folder to re-encode the animation. Indexed PNGs keep their
  palette indices; other PNGs are mapped to the animation's palette. Each
  animation paints its background in its own key colour (it varies per file);
  the editor shows it transparent over a selectable preview background (dark,
  grass, light or the original key colour). Exported PNGs mark the key colour
  transparent (tRNS) and `meta.json` records it as `key_colour`; in RGBA
  imports, pixels with alpha < 128 become the key colour.

Edits are staged in a temporary work folder. **Save copies** writes the edited
`UB.DBI`/`ANMSUNIT.FF` into a folder of your choice; **Install into game**
overwrites the installed files, first keeping a one-time `UB.DBI.orig` /
`ANMSUNIT.FF.orig` backup. Restore by copying the `.orig` file back. Game data
is read-only in this version.

The equivalent CLI commands are `dbi-export`, `dbi-replace`, `ff-list`,
`ff-export`, `ff-replace`, `smk-info`, `smk-export` and `smk-import` (run
`libday <command> --help`). An `.ff` member is addressed as `FILE.FF!member`.

## Engine reconstruction knowledge base

Reference sections: [containers and loading](#installation-containers-and-loader-lifecycle),
[database schemas](#tactical-database-schema-and-identities),
[geometry](#terrain-geometry-and-logical-layers),
[DBI decoding](#dbi-binary-artwork-format),
[sprite composition](#building-and-unit-sprite-composition),
[surfaces](#roads-walls-bridges-and-wreckage),
[units and arrivals](#military-state-reinforcement-scheduling-and-occupancy),
[scenario rules](#scenario-text-recruitment-budgets-and-objectives),
[state initialization](#fresh-battle-initialization-versus-saved-state),
[runtime debugging](#runtime-control-debugging-and-message-pump),
[fixtures](#persistence-tooling-and-reconstruction-fixtures),
and [open questions](#what-remains-unknown-for-a-replacement-engine).

### Evidence, scope and compatibility target

The supported executable identity, display modes and imports are recorded in
the historical engine summary and native-address table below. The practical
compatibility target is this exact installed build, its local databases and
artwork, and observed fresh-skirmish behavior. No original source has been
recovered and no replacement engine has been implemented.

Keep three kinds of evidence distinct:

| Evidence | What it establishes | What it does not establish |
| --- | --- | --- |
| Native disassembly and data constants | A particular loader, branch, field, getter or reset exists in this executable | Every execution path or mode uses that behavior |
| MDB/ZIP readback and artwork decoding | Schema, stored values, identities, encoded pixels and preservation of other data | The game accepts an arbitrary combination of edited objects |
| Visible runtime/HUD observation | The named fixture reached a particular screen or displayed a value | Completion of its mission or correctness of untested campaign/network modes |

Editor validation is deliberately conservative. Its placement restrictions,
input limits, preview frame selection and undo rules are not a complete
specification of the original simulation. Where a rule below is an editor
policy rather than proven native behavior, it is identified as such.
`archive\evidence.json` describes the earlier WSL/display experiment; it is a historical
snapshot and does not include the later successful windowed/editor playtests.

### Installation, containers and loader lifecycle

The installation paths are taken from `[Installation]` in `EXE\LIBDAY.INI`.
Known keys are `anmsunit`, `cache`, `colanms`, `colonies`, `encycl`, `exe`,
`globals`, `imgs`, `interf`, `savegame`, `scenarios`, `sounds` and `video`.
An isolated copy rewrites the writable `exe`, `globals`, `scenarios`,
`savegame` and `cache` paths; the large read-only assets can remain shared.
Launch with the executable's `EXE` directory as the working directory.

| Container | Role and known contents |
| --- | --- |
| `GLOBALS\GLOBAL.MDB` | Global terrain, building, military-unit, road, wall and decor definitions |
| `GLOBALS\GAMEINFO.MDB` | Mutable player/game state; isolate it between playtests or instances |
| `SCENS\SCEN1.ZIP`, `SCENS\SCEN2.ZIP` | Embedded tactical MDBs; baseline inventories were 55 and 51 members before our additions |
| `SCENS\SKIRMISH.MDB` | Skirmish selector catalog, separate from each embedded scenario's metadata |
| `SCENS\DEATHM.ZIP`, `SCENS\DEATHM.MDB` | Ten baseline deathmatch maps and their selector catalog |
| `COLONIES\COLONIES.ZIP` | Colony databases C1 through C4; base/counterattack behavior is not fully mapped |
| `IMGS\TERRAINS.DBI` | Terrain, including forest and scenery tiles |
| `IMGS\UB.DBI` | Military-unit and building sprite parts |
| `IMGS\ROADS.DBI`, `WALLS.DBI`, `RUINS.DBI` | Connected roads, walls, rubble and wreckage |
| `INTERF\640X480.SMK`, `INTERF\1024X768.SMK` | Resolution-specific interface assets |
| `EXE\LIBDAYEN.DLL` | English UI string resources, including skirmish difficulty names |

Observed skirmish progression is selector -> map loading -> technology
allocation -> overview/deployment -> tactical battle. Screenshots of the
loading card alone are not proof that an MDB has been loaded successfully.
The stored overview and tactical minimap are different rendering paths:
the overview can retain old artwork while the tactical minimap reflects
new terrain.

**Extraction directories are a native precondition.** With a configured
cache root, archive-specific subdirectories such as `CACHE\SCEN1`,
`CACHE\SCEN2` and `CACHE\DEATHM` must already exist. Our staging helper
initially created only `CACHE`. Both a stock map and the edited demo then
remained at `LOADING SKIRMISH`; their extracted MDBs appeared in `EXE`,
not the expected cache subdirectory. Creating the empty subdirectories
fixed both stock loading and the edited playable battle.
`stage_game()` now creates one empty cache directory per staged scenario ZIP.

Do not copy old cached maps into a new installation. Before replacing a
previously loaded map, close the game and remove only that map's cached MDB
in the installation being changed. A byte-correct ZIP is insufficient if
the loader reuses an older extracted database.

The short-path stock experiment also failed before the cache folders were
created. Copying the original `EXE\C1.mdb` into that experiment did not fix it;
the successful baseline did not need that extra file. Likewise, applying
`HIGHDPIAWARE` alone did not resolve the stall. These observations reject
those proposed fixes for this particular failure, not every possible
colony, path-length or compatibility problem.

### Tactical database schema and identities

The researched tactical fixtures have **22 user tables**. Names and field
spelling are native identifiers, including historical typos. Access treats
field casing case-insensitively; an importer in another language must not
accidentally distinguish `PosX` from `posX`.

| Table | Confirmed fields or role |
| --- | --- |
| `Map` | `Id, PosX, PosY, TerrainImg` |
| `Buildings On Map` | `Counter, id, buildingType, playerOwner, currentAR, PosX, PosY, IsAiTarget` |
| `Roads On Map` | `Counter, roadtype, playerOwner, PosX, PosY` |
| `Walls On Map` | `Counter, roadtype, playerOwner, currentAR, posX, posY, isDamaged` |
| `Ruins On Map` | `Counter, id, PosX, PosY`; `id` is text, not an entity number |
| `Units On Map` | Initial military units; full field list below |
| `Taurans On Map` | Same observed field layout as initial units; preserved-only legacy table, empty in the scanned campaign fixtures |
| `Reinforcement` | `Counter, UnitID, type, posX, posY, turn, order` |
| `Attack Army` | `Counter, UnitID, type, posX, posY, turn` |
| `Counter Attack` | Same observed fields as Attack Army, plus Boolean `order` |
| `Automated Units` | `Counter, AutomatedType, StartX, StartY, DestX, DestY, TurnArrival` |
| `Deployment` | `Counter, player, PosX, PosY, Turn` |
| `HQ on Map` | `Counter, playerOwner, PosX, PosY`; not the same schema as a placed HQ building |
| `Mines On Map` | `Counter, typeID, playerOwner, PosX, PosY, armed, hidden_AI, hidden_1, hidden_2, hidden_3, hidden_4` |
| `Scenario Info` | One embedded metadata row; fields described below |
| `One Off Units Available` | `Counter, Enum, Available` |
| `One Off Automated Available` | Separate support/automated recruitment availability |
| `Objective` | `Counter, ObjectifId, TurnNumber` |
| `Objectives ID List` | `Counter, Id` |
| `Objectives Pos List` | `Counter, PosX, PosY`; scenario-wide tiles interpreted by the selected objective preset |
| `Unit Order ID List` | `Counter, Unit ID, Id` |
| `Unit Order Pos List` | `Counter, Unit ID, PosX, PosY` |

`Counter` is a database row identity, often a Jet AutoNumber; it is not the
same thing as an in-game object ID, type enum or list position. Existing
Counters must remain paired with their original entities. New records can
receive fresh AutoNumbers even when an entity's game ID stays unchanged.
Terrain `Id` values are also stored identities: do not replace them with
a coordinate-derived row index.

Military-unit game IDs are 0..999999; IDs from 1000000 upward belong to
buildings. `Scenario Info` has `NextID_Units` and `NextID_Buildings`.
Allocate above both the stored next counter and occupied/referenced IDs,
not merely above the number of rows. Zero is a valid unit ID/type value.
The editor allows unit ID 999999; next ID 1000000 means unit IDs are exhausted.
Its building allocator also guards the signed 32-bit ID boundary.

Objective/order ID tables, Attack Army and Counter Attack can reference
entities. A rebuild needs a reference graph, not independent lists which
silently leave dangling IDs on deletion. A reinforcement's own `UnitID`
is its identity, not a link to an initial-unit row: 2617 scanned
reinforcement records had no matches to initial-unit IDs.

Native Jet Booleans can appear as -1 for true. The bridge uses ADODB numeric
type 3 for long integers, 11 for Booleans, 202 for Unicode text and 203 for
Unicode memo fields. Do not confuse a serialized Boolean with an integer
property just because Python permits `bool` as a subclass of `int`.

### Terrain geometry and logical layers

`Map.Id` is a long integer; `PosX` and `PosY` are byte coordinates;
`TerrainImg` is a text field of length 9. The native loader takes the square
root of the terrain row count and assigns equal width and height.
Known fixtures include 24x24 Crime and Punishment/Green Thaw, 48x48
Enclosure, and 96x96 World Cup. Complete-square validation and the
editor's 1..256 dimension limit follow the stored coordinate representation;
they do not establish that every such size is playable.

Terrain registry identifiers are case-insensitive. Some exported names have
mixed case; normalize lookup keys, but retain unchanged source spelling.
`Terrain Types` includes `counter, id, burned_id, burned_min,
can_fire_through, type, Color_Red, Color_Green, Color_Blue, Overlapping`.
The burn/fire fields exist, but their simulation is not reconstructed.
NORD017 has blank color fields; this is not evidence that a default color
should be synthesized.

| Terrain kind | Observed meaning | Editor classification |
| --- | --- | --- |
| 0 | Shallow water | Preserved/protected by ordinary painting |
| 1 | Deep water | Water-building/unit and bridge checks |
| 2 | Riverside/riverbank | Road bridge transition checks |
| 3 | Plain | Non-overlapping entries are Ground; overlapping entries are Scenery |
| 4 | Mountains | Scenery |
| 5 | Forest | Trees, regardless of overlapping flag |
| 6 | Crack | Scenery |

The registry has 355 editable clear-ground textures, 175 forest textures,
and 354 editor-classified scenery textures. **A tree is a terrain tile**,
not an independently stored object over an underlying ground tile. Removing
forest or scenery requires a replacement terrain identifier. Water/coast
connectivity and automatic terrain-family transitions are not implemented.

The editor's established projection for a logical tile `(x,y)` is:

```text
center_x = origin_x + (x - y) * 27 * scale
center_y = origin_y + (x + y) * 14 * scale
sprite_left = center_x - 27 * scale
sprite_top  = center_y - 42 * scale

dx = (pixel_x - origin_x) / (27 * scale)
dy = (pixel_y - origin_y) / (14 * scale)
tile_x = floor((dx + dy) / 2 + 0.5)
tile_y = floor((dy - dx) / 2 + 0.5)
```

The ground diamond has half-extents 27 horizontally and 14 vertically.
The 54x56 sprite image extends above the ground diamond. Hit testing must
use logical diamonds, not transparent sprite rectangles. Draw terrain and
multi-tile object parts in `(x+y, x)` order so foreground terrain can
occlude rear objects. This is the verified editor reproduction, not a
complete recovery of all native animation/effect sorting.

Building/unit origins are the minimum logical X/Y corner. A footprint
contains `(x+dx,y+dy)` for `dx=0..SizeX-1`, `dy=0..SizeY-1`.
Building loading reads PosX/PosY directly; no swap, screen-pixel conversion,
flip or center-to-origin correction was needed.
The apparent misplaced Tech Centre was a deliberately altered demo, not
a coordinate bug.

### DBI binary artwork format

The shared `TerrainArt` decoder handles all five researched DBIs despite
its historical class name. Everything below is little-endian.

| Structure | Layout |
| --- | --- |
| File header | 32 bytes, `<4sIIIIIII`; magic `dbi\0`, version 3, count, width 54, height 56, palette count 1; remaining header fields observed zero |
| Palette | 1024 bytes at offset 32: 256 BGRA entries |
| Record area | Starts at offset 1056 (`0x420`); each record is exactly 8192 bytes |
| Tail index | 12 bytes per record: 8-byte ASCII name and uint32 record number |
| Exact file size | `1056 + count * (8192 + 12)` |

Validated record counts are terrain 1119, UB 4225, roads 55, walls 140
and ruins 137. The decor registry has only 136 entries; the remaining
ruin artwork is not automatically a valid placeable object.

An individual record has:

```text
+0   8 bytes   ASCII name, optionally NUL-terminated; all 8 bytes may be used
+8   uint16    width
+10  uint16    height
+12  uint16    compressed payload size
+14  uint16    encoding, observed 1
+16  4 bytes   not interpreted by the current decoder
+20            scanline payload
```

Each scanline starts with a uint32 nominal length, including that length
field. Runs contain uint16 pixel count and uint16 kind: kind 1 skips
transparent pixels; kind 2 is followed by that many literal palette-index
bytes. Opaque palette entries use alpha 255; the palette's reserved alpha
byte is not the transparency rule.

**The final run has a nonstandard truncation convention.** For the final
scanline, a trailing transparent run can store only the first byte of its
skip count, omitting the other three run-header bytes. The decoder accepts
this only when the nominal end is payload end +3, exactly one payload byte
remains, and that byte accounts for all remaining pixels. Fully represented
payloads can instead finish at payload end -1. Do not replace these checks
with a generic "ignore truncation" rule.

Record padding contains stale bytes and apparent sprite names. Never scan
padding to discover sprites or decode beyond the validated payload.
Cross-check every fixed record against the tail index, reject duplicate
names/invalid record numbers, and validate that every row covers 54 pixels.
The local decoder successfully decoded every record of all five DBIs.
The standard-library PNG writer uses RGBA, filter-zero scanlines, zlib,
and CRC32; Pillow is unnecessary.

### Building and unit sprite composition

Building definitions use `TypeID`, not their global database `Counter`.
The reduced registry retains `NameEnglish, SizeX, SizeY, Position, Armor,
UniqueOnMap, TransformIntoUnit, isAItarget, ImageID, BuildingCat,
DamagedImages`. There are 81 registered types, including non-square
footprints; observed dimensions reach 5x5. Position 3 means land and
position 1 means deep water. Observed BuildingCat values are 0 standard,
1 ruin and 2 tech.
The editor enforces `UniqueOnMap` globally by building type on new/relocated
placements; its exact scope across native game modes still needs verification.

Building sprite names are:

```text
ImageID + level_digit + state_digit + dx_digit + dy_digit
```

Choose a complete set of footprint parts, not a single available sprite.
The editor picks the lowest complete level; many human buildings have
levels 1..4 rather than level 0. Deploy Mk1..4 (types 131,173,174,175)
use matching levels 1,2,3,4. The editor uses damaged building art below
half armor when the definition and complete sprites support it.
The full native building damage/technology/animation rules remain a
separate reconstruction task.

Guard/Laser/AA/Rocket towers (types 100/102/103/101) transform to military
types 37/39/40/38. Their state digits are directions, not ordinary
building damage states; editor previews use direction 0.
Lab Ruin 1005 and Tracks Shop 1017 lack complete damaged variants at the
selected preview level. The editor displays intact art with a warning,
not invented damage sprites.

Examples: Tech Centre type 237 is 2x2, armor 50, prefix TSTC; Living Pod
480 is 2x2, armor 40, prefix BGEA; human HQ 130 is 4x4, armor 500,
prefix HHQ. Original Green Thaw has Tech Centre ID 1000000 at `(10,14)`
and Living Pod ID 1000001 at `(8,10)`.

`Unit Types` is keyed by zero-based `(typeID, unitTechLevel)`: 217
definitions across 62 base types. Human types often have levels 0..10.
All 6193 scanned initial-unit records matched these definitions.
Examples are Commando 0, Trooper 2, Footman 4, Claptrap 28, Flyer 30,
Platform 36 and OutPost 47. Platform and OutPost have 3x3 footprints;
most types are 1x1. Do not increment type IDs.

Military sprite names are:

```text
facingsImageID + frame_digit + facing_digit + dx_digit + dy_digit
```

Facing values are 0..7; definitions with `Facings=false` use sprite
facing 0. Some tower sprites start at frame 1. The editor chooses the
lowest frame 0..9 with a complete set for every facing and footprint.
All 62 base types have such a frame. Static previews do not simulate
animation, technology-dependent appearance, health effects or player-color
palette remapping. Those omissions must not become replacement-engine rules.

### FF containers and Smacker unit animations

`ANMSUNIT\ANMSUNIT.FF` is a flat container: magic `FF\0\0`, u32 entry count,
then 64-byte entries (56-byte NUL-padded name, u32 absolute offset, u32 zero)
and the member data. Entries are not sorted by offset; a member's size is the
distance to the next offset in file order. All 5146 members are Smacker 2
(`SMK2`) videos, most 216x224 at 15 fps:

- `PREFIX\P<c>\PREFIX<c><A><f>.SMK`: unit animations, where `PREFIX` is the unit's
  `facingsImageID`, `c` the player colour 1-4, `f` the facing 0-7, and `A` the
  action. The letters match the `Unit Types` `*Animation_Begin` fields (-1 = no
  animation): `M` move, `L` move start, `N` move end (L/N order confirmed visually),
  `S` shoot begin, `T` shoot (muzzle flash, carries the shot sound), `U` shoot
  end, `P` projectile, `I` impact and `E` explosion. P/I/E exist for facing 0
  only. Two types ship projectile files although their field is -1.
- `BUILDING\<ImageID>.SMK`: building destruction (`ExplosionAnimation_Qty` 25).
- `AUTOUNIT\<id><nn>.SMK`: air support (e.g. bombers `HBMB01-03`, helicopters
  `HHTR11-13`); `MINE\MINE.SMK`, `MINE\GMINE.SMK`: mine explosions.

Smacker has no transparency field and no database table names a key, so each
animation paints its background in a solid key colour that differs per file
(of 5146 members: (28,0,0) at index 198 in 3402, magenta (255,0,174) at 245 in
1265, grey (93,93,93) in 250, orange (174,77,0) in 124, then a few blues and
reds). The key is the most frequent colour of the first frame
(`Frame::dominant_colour`); this matches the background of every member but
`MPAR\P3\MPAR3M3.SMK`, whose background is dithered noise in the original data.
The top-left pixel is a weaker rule: the HTAA3T1 projectile covers it, and
HHTR03 has a 1-pixel black edge line. Match the key by RGB, not index, because
palettes can change between frames. That the engine uses exactly this rule is
not yet confirmed from the disassembly. The `libday-formats` Smacker codec decodes all members and its
encoder (Huffman trees rebuilt per file, original palette/audio chunks kept)
re-encodes every member pixel- and palette-identically; the re-encoded archive
is slightly smaller (108.5 MB versus 108.8 MB). The real-data test
`every_unit_animation_re_encodes_losslessly` checks this (`cargo test --release
-p libday-formats -- --ignored` with `LIBDAY_ROOT` set).

`Unit Types` has no explicit stealth column. `Jammer` is 0 for the Chopper,
Commando, Flyer, Mind Warrior, Pack Freak, Shocktrooper and Submarine, 10 for
grav units and 100 for everything else. Jammer 0 is stealth (confirmed from
play): enemy units do not react when the unit enters their range. The editor
labels values below 100 as a low signature, which is still an inference.
Other capabilities: `weaponType` 1 = indirect fire, `DamageArea` 1/3/5 (area
side), `damageType` 1 = drains action points, `IsAA`/`DamageAA`,
`canOverwatch`, `MineDetectionRange`.

### Roads, walls, bridges and wreckage

Roads, walls and ruins are separate per-tile object records, not terrain
identifiers. Surface Counters are stable row identities; a ruin's textual
`id` is its artwork/type identifier and must not enter the numeric object-ID
allocator. Original duplicate decor records are preserved.

Road/wall connections use same-type cardinal neighbors, independent of
owner. X- sets bit 8, X+ bit 4, Y- bit 2, Y+ bit 1.
The mask is **not** the sprite number:

```text
mask  0  1  2  3  4  5  6  7  8  9 10 11 12 13 14 15
shape 0  9 10  7  8 13 12  2 11 15 14  5  6  3  4  1
```

Road artwork uses `img_id + mode + three_digit_shape`. Mode G is ground,
B is deep-water bridge, and R is riverbank transition. The riverbank
orientation tests, in priority order, are:

```text
0: X- neighbor is deep water/riverbank and X+ neighbor is plain
1: X+ neighbor is deep water/riverbank and X- neighbor is plain
2: Y- neighbor is deep water/riverbank and Y+ neighbor is plain
3: remaining orientation

connected shape -> riverbank variants for orientations 0,1,2,3
0  -> 16,20,24,28
6  -> 17,21,32,32
8  -> 18,22,32,32
11 -> 19,23,32,32
7  -> 32,32,25,29
9  -> 32,32,26,30
10 -> 32,32,27,31
other shapes -> 32,32,32,32
```

The editor permits supported straight paved bridges on existing water/
riverbank terrain and rejects unsupported bridge junctions. Its drag
interpolator inserts cardinal steps at diagonal transitions so painted
roads/walls actually connect.

Bridge construction permissions are **derived from terrain**, not stored in a
tactical MDB table or `[Map]` flag. `[Map]` contains only
`Id, PosX, PosY, TerrainImg`. The generator at `0x42FB2D` runs:

1. `0x42FBB1`: inspect riverbank type 2 in row-major order, excluding the
   final row/column as starting positions. Prefer positive X when the next
   tile is type 1 or 2 and the previous tile is plain type 3; otherwise try
   positive Y with the same rules. An eligible X start does not fall back
   to Y when its far endpoint later fails.
2. `0x42FE1D` / `0x42FEE4`: advance through deep water type 1 to the first
   riverbank type 2; require plain type 3 immediately beyond that bank.
   Shallow water, other terrain, missing endpoints and out-of-map cells
   fail. No separate maximum length was found.
3. `0x42FF6E`: retain the earlier candidate when either start-coordinate
   difference is exactly 1 **and** either end-coordinate difference is
   exactly 1. These are axis-wise OR comparisons, not Manhattan distance
   or a same-orientation check. `0x430546` appends candidates at the tail.
4. `0x43002D`: expand accepted spans, including both banks; skip an entire
   later span if any position overlaps an earlier accepted span. This
   lookup searches the generated-position list, **not existing road records**.

The resulting road-manager `+0x24` list feeds the bridge-placement interface
(`0x44614B`, `0x4461B1`) and construction (`0x4722B3`). Runtime eligibility
at `0x42F5DD` separately checks terrain, buildings, units, walls and roads;
its helper `0x42F690` checks the four neighboring roads. Consequently the
editor displays generated spans without claiming to simulate every runtime
construction condition. Its native marker is `SYMBOLS.DBI:BRIDG001`.
The Jet bridge accepts deep-water/bank transitions only with explicit
`-BridgeTerrain banks` plus building/surface validation inputs; ordinary
object-editor terrain restrictions are unchanged.
It validates final unit/building footprints and road geometry, including
compatible roads/grav units placed after a bank edit in the same save.

Mines are native `[Mines On Map]` records rather than terrain or unit records.
`GLOBAL.MDB` defines type 0 (normal, `ImageID=Mine`) and type 1 (grav,
`ImageID=Gmine`). Both have `deployTerrain=3`, damage type 0, base 25 and
native random parameter 10. Normal mines affect ground movement; grav mines
also affect grav and water categories. Purchase/recycle costs are 10/5 and
20/10 respectively. The observed 115-map inventory contains 1,704 normal
and 688 grav mines, all armed and owned by AI slot 0. Minesweeper
(`s1-5-1.mdb`) contains 16 normal mines, visible to AI and hidden from all
human slots; `s3-10-1.mdb` contains 233 grav mines.

`armed` and `hidden_AI, hidden_1..4` are independent native Boolean fields;
the native reader/writer around `0x42B850` / `0x42BD75` handles them separately.
`UB.DBI` contains `MINE000/001` and `GMINE000/001`; the editor deliberately
uses static frame 000 without claiming to reproduce the second frame's
runtime meaning. Cached `mine-types.csv` provides the two factual definitions
without modifying GLOBAL. Mine insertion uses Jet AutoNumber counters;
untouched rows, flags, original duplicate placements and unrelated tables
are preserved on no-op saves. Mine/bridge live-game behavior after editing
has not yet been independently exercised.

Wall artwork uses `img_id + damage_digit + two_digit_shape`. Health damage
and `isDamaged` have different meanings:

```text
damage_threshold = floor(maximum_armor * 33 / 100)
damage_digit = 1 when currentAR <= damage_threshold, otherwise 0
broken concrete end-cap remapping: 8->19, 9->18, 10->16, 11->17
```

Native disassembly corroborates the 0.33 comparison and shape selection.
Maximums/thresholds are concrete 60/19, laser fence 150/49,
force field 300/99 and ruined wall 5/1. Values 20,50,100,2 respectively
are above the damaged threshold. `isDamaged` selects the broken concrete
end-cap variants independently; it is not the general damage-texture flag.
The editor rejects that flag on wall types without those variants.

Decor definitions include `can_fire_through` and `can_move_through`.
Building rubble and military wreckage have independent passability.
Scenery terrain and ruin records must not be collapsed into one storage
layer just because both are decorative.

### Military state, reinforcement scheduling and occupancy

Initial-unit rows contain:

```text
Counter, ID, unitType, unitTL, playerOwner, facing, currentAR, currentAP,
negAP, PosX, PosY, -------------, OrderID, Turn, Probability, EnemyUnitType
```

All numeric fields inspected here are ADODB longs; the separator named
`-------------` is Boolean. `currentAR` is current armor/health and
`currentAP` is current action points. Owner 0 represents AI; initial
owners 0,1,2 were observed, and the editor supports 0..4.
`Turn` is **AI-order expiry**, not a delayed arrival: Green Thaw unit
101 has Turn 2 and still starts on the map. The AI planner clears an
order when its Turn is less than the current native turn. `negAP` holds action
points drained below 0 (player gas bomb, Tauran paralyzer); at the next turn
the unit gets its maximum AP minus `negAP`, and `negAP` returns to 0. It is
believed to be capped at the maximum AP, so paralysis lasts at most one turn
(gameplay report, not yet traced in the executable). Preserve `negAP`, separator,
OrderID, Turn, Probability and EnemyUnitType on ordinary property edits.
New editor-created initial rows use defaults 0, true, 0, 0, 100 and 65535
for those fields; these are editor creation defaults, not a full AI design.

Reinforcements have their own table and distinct IDs. They do not store
owner, technology, facing, health or AP. Native arrival constructs an
AI-owned unit (owner 0), resolves technology through player information,
and assigns the other runtime properties. The Boolean `order` derives an
AI order and its targets from the scenario objective, with expiry 99 and
Probability 100. It does not store a separate customizable reinforcement
order; the mapping is documented in the scripting section below.

Native reinforcement turns are 0..100; campaign scans observed 1..12.
Turn 0 has a separate startup-processing path and remains a reinforcement
record. It must not silently become an initial-unit row just because an
editor groups it into the Start view. Due rows are attempted when their
turn is at most the current turn. Successful spawns leave the pending list;
failed/occupied spawns remain pending and can be attempted later.
Native UI formats the current turn directly; no evidence was found for
an additional editor-side +1 offset.

World Cup (`SCEN1.ZIP`, `s4-20-1.mdb`) is the useful reinforcement fixture:
96x96, turn limit 15, NextID_Units 858, NextID_Buildings 1000345, and
114 reinforcement rows in the preserved snapshot. Example reinforcement
836/type 31 arrives at `(45,0)` on turn 2 with order=false.
Initial unit 452/type 47 is at `(10,89)` with owner 0 and armor 300.
These are fixture facts, not defaults for every mission.

The editor's conservative movement compatibility is:

| Movement value | Placement accepted by editor |
| --- | --- |
| 4 boat, 6 underwater, 8 static-water | Non-overlapping deep water |
| 5 air | Any terrain |
| 3 grav, 9 grav-fly | Non-overlapping deep water, riverbank or plain |
| 2 infantry | Clear ground or forest |
| Other land/static/teleport categories | Clear ground |

A real engine still needs movement costs, fire-through/LOS, transitions,
pathfinding and mode-specific rules. Current placement checks alone do not
specify those systems. New/relocated units avoid buildings, walls, mines,
blocking decors and deployment reservations; roads and passable
decors may coexist with units. Same-arrival footprints cannot newly overlap;
different arrival turns can share a spawn location, but the game may
delay a blocked reinforcement.

Building/tree/wall/decor placement reserves complete unit footprints across
arrival layers. Property-only edits preserve existing placements without
rejecting unrelated pre-existing geometry. Deployment cells and preserved-only
army rows remain conservative reservations. Scheduled-support Start/Dest
positions are effects, not static placement obstacles, and no longer reserve tiles.

### Scenario text, recruitment, budgets and objectives

The embedded `Scenario Info` row includes localized Title, provincename,
description and Aftermath fields for English/German/French, plus
`turn_qty`, `necessary_unit`, `NextID_Units`, `NextID_Buildings`,
`One Off Deployment`, `CounterAttack`, `Credits`, `ReinfRes`,
`mapScreen640` and `mapScreen1024`.
Names/province names are Unicode text of size 50; descriptions/aftermath
are Unicode memo fields. `Credits` is distinct from deployment and
technology setup budgets. `necessary_unit`, CounterAttack and ReinfRes
are preserved fields, not fully reconstructed mechanics.

The external skirmish catalog also has a `Scenario Info` table, but its
rows have `Counter, FileName`, localized names/province names/descriptions,
`One Off Deployment` and `Size`. It has no independent difficulty or turn
limit column. **Catalog data and embedded map data are separate sources**:
editing one does not inherently update the other, and their stored budgets
must not be assumed equal in original scenarios. Our exporter intentionally
synchronizes new catalog entries with edited map metadata.

The selector orders maps by budget rather than simply by catalog Counter/
insertion order: the 420-point demo appears among Average maps, not at
the end of the catalog. Exact secondary sort rules remain unrecorded.
Do not use catalog row position as a permanent scenario identity.

Military availability is `One Off Units Available`: Enum is a base military
type ID and Available is Boolean. The native candidate set is existing
level-0 definitions with ID below 35, ID 49, or IDs 54..59: 39 types.
Towers and other map-placeable variants are not automatically recruitable.
Original Crime and Punishment/Green Thaw enables Trooper 2, Pyro Maniac 3,
Footman 4, LAV 8, Beast Wagon 10 and Grenadier 56.
Support availability is separate; the sample enables automated enums 0
and 7. Preserve existing availability Counters; disable rows rather than
silently dropping existing kinds.

Difficulty comes from the catalog's deployment value: Easy <=300,
Average 301..500, Hard 501..1000, Very hard >1000. Native constants are
300,500,1000,-1 and English resource IDs 912..915 supply the labels.
Campaign/global Easy/Hard settings are another mechanism.
Presets 150,400,750,1500 are **editor choices**, not native difficulty
constants.

Technology and deployment are separate allocations. The edited 420-point
demo displayed 2100 in technology setup and 420 in deployment; two
Troopers cost 5 each and left 410 deployment points. This demonstrates
a factor of five for that setup, not a universal conversion for every
credit/health/cost field. Full research-budget computation, costs,
technology caps and campaign economy remain open. `unit-types-full.csv`
retains Cost, CostToNextTL and unitTechLevel for future research.

Native text uses `##` separators. The demo which began directly with
`Objective: (12 turns)##Find...` omitted that first segment in the selector
but displayed subsequent Find/Army setup segments. This suggests the
first segment is treated as story/briefing and later segments as objective
text. Keep raw text unchanged on a no-op; do not assume `##` is merely
an arbitrary visible newline in every screen.

Text is not executable victory logic. `Objective.ObjectifId`, TurnNumber,
ID targets and position lists supply actual rules. The Green Thaw fixture
has kind 0, TurnNumber 10 and target ID 100. A shared native timeout helper
consults the global turn limit, while particular objective rules also use
TurnNumber (including holding/cumulative-turn logic). Do not rewrite all
objective TurnNumbers when changing `turn_qty`.
The 12-turn demo reached battle and displayed 12 while its objective
tables remained unchanged. The scripting section below records the decoded
campaign predicates and separates remaining runtime uncertainties.

Current editor limits are nonblank title 1..50, turns 1..100, deployment
points 0..32767, and no NUL characters in text. Only native turn/field
representation evidence should be promoted into engine limits;
application validation is not an exhaustive proof of every native bound.

### Scenario scripting: native orders, scheduled support and objectives

This records the reverse-engineering evidence behind the implemented native
script editor. Research compared all 115 tactical MDB copies
byte-for-byte with the installed SCEN1/SCEN2/DEATHM archive members, inspected
their script tables and GLOBAL definitions, and traced native handlers in
the unmodified executable. No live scripting probes have yet confirmed the
player-visible turn boundaries or every action's completion behavior.

There is no general-purpose script language or arbitrary trigger/action
graph in the inspected scenario data. Three distinct native mechanisms
cover the observed scripting: one AI order per starting unit, turn-scheduled
automated/support requests, and one objective preset with shared targets.
Ordinary reinforcements are a fourth, already editable arrival mechanism.
Do not manufacture new MDB fields and imply the original engine will run
them.

The stock inventory contains 6588 initial units, 1556 unit-order ID targets,
2667 unit-order tile targets, 312 automated events across 67 maps, and
2617 ordinary reinforcements across 95 maps. Each tactical map has exactly
one Objective row. There are 270 objective ID rows and 3077 objective tile
rows. These counts include normal and alternate variants separately.

#### Starting-unit AI orders

GLOBAL.MDB's `Unit Order Id Lookup` and the AI factory agree on:

| OrderID | Native order | Target storage |
| --- | --- | --- |
| 0 | No Order | No authored targets required; ordinary AI can still act |
| 1 | Goto | `Unit Order Pos List` |
| 2 | Goto Fast | `Unit Order Pos List` |
| 3 | Escort | `Unit Order ID List` |
| 4 | Defend | `Unit Order ID List` |
| 5 | Patrol | `Unit Order Pos List` |
| 6 | Attack | `Unit Order ID List` |

`Unit Order ID List` rows are `Counter, Unit ID, Id`; `Unit ID` identifies
the ordered unit and `Id` identifies a target. Position rows are
`Counter, Unit ID, PosX, PosY`. Targets are loaded with a per-unit WHERE
clause and no ORDER BY. Escort, Defend and Attack can reference buildings
as well as military units. Their names alone do not establish all movement,
combat or target-selection details.

Native order setters also accept 7 and an internal overwatch action exists,
but 7 has no authored lookup entry or stock use. Do not expose it as a
verified scenario order.

`Turn` is an inclusive expiry: the AI clears OrderID when
`unit.Turn < currentTurn`. It is not an activation turn and does not delay
the unit's spawn. A commonly used value of 99 is a long expiry, not evidence
of a special unlimited sentinel. The format cannot directly express
"Goto on turn 3, then Attack on turn 5" as two independent orders on the
same unit.

`Probability` is an order-retention roll, not spawn probability. At the
start of an AI planning cycle the engine computes `rand() % 100` and clears
OrderID if that roll is **greater than** Probability. The inclusive
comparison means value 0 can retain an order on roll 0; value 50 permits
rolls 0..50; value 100 always passes. Failure changes the stored runtime
order rather than just suppressing one shot. The AI state machine resets
to this stage for subsequent planning cycles; exact cadence relative to
visible player turns remains a live-check item. Native setters accept 0..100.

`EnemyUnitType` is a target preference, not a second object-ID target.
Native scoring compares the candidate's military type with this resolved
type and doubles a score component on a match. It is not a strict
only-attack-this-type filter. 65535 is the stock default/no-specific-type
value; 153 initial rows use explicit types.

Patrol scans candidate tiles and selects using a path/action-cost
comparison. Its tile list must not be presented as a guaranteed sequential
waypoint route. Preserve native row identities/enumeration for all orders
until each action's traversal semantics are established.

Examples: Opportunity Knocks `s1-2-1.mdb` has unit 135 using Goto Fast until
turn 1 at `(37,34)`, unit 109 escorting building 1000000 until turn 7 with
Probability 33, and unit 107 patrolling a 20-tile region. Knowledge is Power
`s2-11-1.mdb` has unit 104 attacking building 1000015 until turn 20.

#### Scheduled support, bombing and air drops

Scenario `[Automated Units]` rows contain:

```text
Counter, AutomatedType, StartX, StartY, DestX, DestY, TurnArrival
```

These numeric columns are Long Integer. The native validator accepts types
0..26 and arrival turns 0..99. Start and destination **must be identical**
except for transports 0 and 7. For a bomber or air drop, these coordinates
describe the effect location, not a user-authored aircraft flight path.

The authored event list is loaded into the world at offset 0x84.
The AI scheduler compares arrival using **equality with the current turn**,
requests the selected support with owner 0, then removes the authored row.
It does not keep a past-due event for retry, nor wait for successful landing
before consuming it. Animation/effect execution follows the request; row
removal alone does not prove the effect succeeded.

This is separate from the AI's dynamic support purchasing/target-selection
routine. `One Off Automated Available` controls recruitment availability,
not the authored event timeline. Scheduled stock arrivals are turns 1..11;
native-valid turn 0 needs a startup-path live check before ordinary authoring
should recommend it.

GLOBAL.MDB supplies the support definitions, categories and effects:

| Types | Defined support |
| --- | --- |
| 0, 7 | Human / Tauran Transport |
| 1, 8 | Human / Tauran Bridge |
| 2, 9 | Human / Tauran Mine detector |
| 3, 10, 26 | Human Armor, Tauran Medic, Human Movement |
| 4 | Human Bomber: base AR damage 150, damageArea 5 |
| 5 | Human Gas: base AP damage 300, damageArea 3 |
| 6 | Human Nuke: base AR damage 999, damageArea 11 |
| 11 | Tauran Fighters: base AR damage 150, damageArea 7 |
| 12 | Tauran Gene Bomb: base AR damage 999, damageArea 11 |
| 13 | Shadamar Fighters: base AR damage 100, damageArea 3 |
| 14..25 | Fixed air-drop definitions, listed below |

Drop types 14..25 respectively select military types
`4, 2, 0, 58, 56, 3, 32, 14, 1, 34, 59, 30`:
Footmen, Troopers, Commandos, Elite Troopers, Grenadiers, Pyro-Maniacs,
Paralyzers, Mega-Tanks, Shocktroopers, Seekers, Worms and Flyers.
The inspected GLOBAL definitions have nominal `nbDrop=4` and relative
landing offsets `(-1,-2), (2,2), (-1,1), (2,-2)`, plus per-unit facing
properties. The drop handler adds those offsets to the effect location,
checks placement and generates units through the runtime creation path.
Payload units do not have preassigned IDs in the scenario event row.

Actual outcomes are not a simple four-unit stamp: the handler resolves
technology through the owner's player information and halves the nominal
count when the aircraft has taken damage. Bomb damage is likewise scaled
by remaining aircraft armor. GLOBAL's `unitTLDrop` exists, but its stored
value alone is not proof of the runtime technology used.
Obstruction, aircraft interception and animation timing need live probes.
The ordinary reinforcement `order` helper is not called in the traced drop
handler.

The event row has no editable owner, payload composition/count, damage,
technology or custom trigger. Those are runtime/GLOBAL concerns. A scenario
editor should show the selected type's nominal payload/effect read-only,
not silently modify GLOBAL.MDB. Treat damageArea as a native area parameter
until its exact tile footprint has been traced; do not label it a radius
or pretend a preview simulates actual damage.

Useful fixtures: Opportunity Knocks drops Footmen at `(33,13)` on turn 5
in variant 1 and `(31,37)` on turn 2 in variant 2. Controlled Chaos
`s3-7-1.mdb` has both transport types on turn 1. Light in the Tunnel
`s4-18-1.mdb` has several Human bomb/gas/nuke requests on turn 1.

#### Native objective presets

`Objective` stores `Counter, ObjectifId, TurnNumber`.
`Objectives ID List` stores `Counter, Id`, and
`Objectives Pos List` stores `Counter, PosX, PosY`.
Neither target table has an Objective foreign key: they are shared lists
for the scenario's single objective, not independent sets for multiple
clauses.

The decoded campaign handlers are:

| ID | Native label | Predicate evidence |
| --- | --- | --- |
| 0 | Destroy | Listed military/building IDs must disappear. Listed tiles must have no road or mine object; this branch does not check the wall list. |
| 1 | Hold Position | A non-AI military unit on any target tile contributes at most one count per native turn. Wins when accumulated count is strictly greater than TurnNumber; absence/gap reset and early timeout logic also apply. |
| 2 | Escort | A surviving listed military unit reaching any target tile satisfies the predicate. Fails if none of the listed military units remain. |
| 3 | Protect | At the global scenario turn limit, fails if an AI military unit occupies any target tile; survives past that limit if not failed. |
| 4 | Intercept | Fails if a surviving listed military unit reaches a target tile. Succeeds when none of those units remain. |
| 5 | Keep | Fails when any listed military/building ID disappears; succeeds after the global scenario turn limit. |
| 6 | Destroy All Except | Exception IDs must survive; all other AI military units must disappear. Does not require all enemy buildings to be destroyed. |

Empty exceptions for kind 6 supply the common eliminate-AI-units objective.
The position check for kind 0 is corroborated by native world loaders:
walls are at world+0x2C, roads at +0x30 and mines at +0x38; the predicate
checks the latter two. Minesweeper `s1-5-1.mdb` and Special Delivery
`s4-17-1.mdb` are the stock tile-target destruction fixtures.

These are predicate-level conclusions, not a complete specification of
all tactical outcome handling. In particular, the relationship of
`necessary_unit` to mission setup/outcomes still needs tracing. Do not
invent a minimum-escorted-unit setting from its name or treat Escort's
predicate as proof of every displayed mission requirement.

There is no universal meaning of Objective.TurnNumber. Keep and Protect
use the global `turn_qty` deadline rather than that field in their traced
predicates. The shared timeout helper fails unfinished objectives when
currentTurn exceeds the global limit. Hold Position's strict comparison,
gap behavior and visible duration need boundary-case live checks before
the editor translates the stored value into a friendly duration.
Do not rewrite all objective TurnNumbers when changing the global limit.

GLOBAL exposes deathmatch presets 8, 9 and 10, named Death match 2 cities,
Death match city + alien, and Death match slug fest. Their factories are
identified, but their full conditions remain undecoded. Internal kind 7
uses the campaign-counterattack class and has no public lookup entry.
Preserve these modes without offering speculative campaign replacements.
Briefing text and `##` formatting are presentation, not executable clauses.

Ordinary reinforcement `order=true` calls an objective-derived helper:

| Objective | Generated reinforcement order and targets |
| --- | --- |
| 0 Destroy | Escort with objective IDs if the ID list is nonempty; otherwise Patrol with objective tiles |
| 1 Hold Position, 3 Protect | Patrol with objective tiles |
| 2 Escort, 5 Keep | Attack with objective IDs |
| 4 Intercept | Escort with objective IDs |
| 6 and unhandled kinds | No Order |

The helper sets expiry 99 and Probability 100. Label this Boolean
"Use objective-derived AI order", not "Custom scripted order".
Changing an objective can consequently change later AI reinforcements'
behavior even when their own rows are untouched.

#### Remaining data and preservation requirements

`Counter Attack` has 2615 stock rows across 59 maps, all at `(0,0)` and all
with order=false; turns range 0..7. This strongly distinguishes it from
ordinary location-based reinforcements, but does not fully decode the
campaign roster subsystem. `Attack Army` and `Taurans On Map` are empty
in all 115 inspected tactical maps; emptiness is not proof their native
handlers are unused. HQ coordinates, armed/hidden mine state,
`CounterAttack`, `ReinfRes` and `necessary_unit` must not be repurposed as
unverified trigger parameters.

Stock data itself contains duplicates and abandoned records: 2 maps repeat
objective IDs, 19 repeat objective tiles, and `s4-15-2.mdb` has 8 order-tile
rows belonging to absent starting/reinforcement units. Some nonzero orders
have no targets. No-op loading/saving must preserve these records, not
silently sort, deduplicate, drop or repair them. Show diagnostics for
legacy anomalies and reject invalid new edits separately.

#### Implemented scenario script editor and remaining verification

The typed editor uses these native tables, not a new text language.
Its implemented controls are **AI orders**, **Scheduled support**, **Objective**
and a shared **Timeline/diagnostics** view. Unit/target picking, tile-set
painting and transport source/destination picking reuse the map.
Overlays are hidden by default, matching deployment zones.
The timeline distinguishes initial placement, reinforcement arrival,
exact-turn support requests and AI-order expiry.

The implementation follows this sequence; live engine checks below remain pending:

1. **Lossless inspection/model.** Load the script tables into the document,
   resolve object references, show native values and diagnostics, and prove
   no-op MDB/ZIP round trips including duplicate/orphan rows and preview
   blobs. Keep independent scripts/history for each alternate map.
2. **Scheduled support authoring.** Add/edit/remove events with native type,
   arrival and map targets. Start with stock-used bomb/drop/transport types;
   expose other GLOBAL types only after their scheduled execution is checked.
   Show nominal effects/landing footprints read-only. Enforce non-transport
   Start=Dest and verify event consumption and blocked landing in isolated
   game copies. Bomb targets may intentionally overlap units/buildings:
   these are effects, not static placement obstacles.
3. **AI order authoring.** Extend initial-unit properties with order,
   "Active through turn", retention value, preferred enemy type and native
   target collections. Preserve enumeration without promising ordered routes.
   Ordinary reinforcement orders stay objective-derived; arbitrary later
   per-unit action sequences are not supported by the native schema.
4. **Objective preset authoring.** Offer the verified campaign kinds with
   kind-specific ID/tile tools and timing labels, separate from briefing
   text. Initially preserve advanced/unresolved fields. Live-check Hold
   boundaries, Escort outcomes and deathmatch/counterattack modes before
   extending friendly controls. Do not invent arbitrary AND/OR, multiple
   objectives, conditional branching or victory-triggered bombing.

Every authoring stage needs the existing undo/redo, dirty handling, safe
Save Copy and registered skirmish export paths. Recompute reference guards
from the edited script model; a unit/building deletion must not silently
break orders or objectives. Distinguish objects available at startup from
later reinforcements and dynamically allocated drop units. Warn about
targets absent when an order/objective is first evaluated and about events
beyond the mission deadline. Treat nominal landing obstruction as a
warning, not a guarantee about future occupancy.

Regression coverage includes targeted native table changes, unchanged unrelated
tables/blobs, no-op preservation of duplicate/orphan/internal rows and native ID
allocator values, variant isolation, known type/turn bounds, reference guards,
failed-save rollback, GUI picking/history and captured save snapshots.
Small isolated live fixtures should
separate expiry, retention, event equality/consumption, drop obstruction and
objective boundaries. The timeline/overlays are an inspection aid, not a
claim to emulate AI, combat or victory resolution.

### Campaign alternate maps and failure/retry selection

Alternates are **separate complete tactical MDBs**, not unit-placement
overlays. Variant 1 is `SCENS\SCEN1.ZIP` member `sC-P-1.mdb`; variant 2 is
`SCENS\SCEN2.ZIP` member `sC-P-2.mdb`, where C is continent and P is province.
The installed archives contain 50 paired missions: continent 1 provinces
2..7, continent 2 provinces 2..12, continent 3 provinces 2..15, and continent
4 provinces 2..20. The four city/base maps `sC-1-1.mdb` and demo
`s1-99-1.mdb` have no matching second variant.

Selection belongs to the strategic colony database. `COLONIES\COLONIES.ZIP`
contains `C1.mdb` through `C4.mdb`; their `[Province List Captured]` table
has `Counter, NoProvince, Captured, RandomChosen`. `RandomChosen` stores the
province's selected variant (1 or 2). Presentation/reward data in
`[Province List]` and the tactical `Scenario Info.CounterAttack` field
are **not** this selector. Mutable campaign colony state is persisted as
`savegame.%02d\colony.mdb`; editing a tactical variant does not update it.

Native filename builder `0x47A999` uses the format string
`SCEN%d\s%d-%d-%d.mdb` at `0x4B8D38`. The selected variant determines both
the archive/cache folder and filename suffix. Getter `0x47A67F` reads the
colony object's province-indexed array at offset `0x20`; `RandomChosen`
is loaded at `0x47ABCF..0x47AC38` and saved at `0x47AE08..0x47AE2A`.
Setter `0x47AF00` updates that same array.

Campaign tactic result handling `0x47F2E5` calls cycle helper `0x47E960`
at `0x47F33C` on its false-result branch. The helper increments the selected
variant and wraps above 2 to 1: **1 -> 2 -> 1**, rather than an indefinitely
harder progression. RTTI identifies this phase as `CServerPhaseCampaignTactic`;
the result getter `0x4160C9` reads its byte at offset `0x41`.
Initialization at `0x47E269..0x47E28E` uses `MSVCRT.dll!rand` through IAT
`0x4AC5FC`, modulo 2 plus 1 for non-city provinces, and forces the player's
city to variant 1. Thus the first attempt need not be variant 1, even though
stock colony rows examined initially contain `RandomChosen=1`.
These are **database/native-code findings**, not a live failed-campaign
and retry observation.

The variants can change terrain, buildings, starting units, reinforcements,
deployment cells and scenario metadata independently:

| Mission | Starting units, 1 / 2 | Reinforcements, 1 / 2 | Deployment cells, 1 / 2 | Terrain differences |
| --- | --- | --- | --- | --- |
| Opportunity Knocks, `s1-2` | 23 / 25 | 4 / 6 | 211 / 193 | None; both 48x48 |
| Convoy, `s3-3` | 47 / 40 | 21 / 21 | 175 / 191 | 18 cells; both 48x48 |

Building counts stay at two for Opportunity Knocks and four for Convoy,
but building rows differ. Opportunity Knocks also
differs in `NextID_Units` and `CounterAttack`; Convoy differs in `NextID_Units`.
More enemy units is not a universal rule for variant 2. Compare terrain by
`(PosX, PosY) -> TerrainImg`: Convoy's native Map IDs differ between variants,
so comparing entire rows incorrectly suggests every tile changed.
The editor opens the complete paired database and preserves each variant's
own metadata, counters and unrelated native tables.

### Deployment zones: authored tiles, player merging and campaign generation

Deployment **area** and deployment **points** are separate mechanics.
The ordinary tactical MDB's `Deployment` table contains
`Counter, player, PosX, PosY, Turn`. Each row describes one allowed map
cell, not the origin of a rectangle or a radius. A zone can be irregular
or disconnected. Player IDs 1..4 are the human player slots; 0 is the AI
in `GLOBAL.MDB`'s `Players Lookup`, but the ordinary deployment loader
queries only slots 1..4. AI reinforcement coordinates are a separate
mechanism, not an implied fifth painted starting zone.

A read-only inventory of SCEN1, SCEN2 and DEATHM archives examined 115
tactical MDBs and 25,521 deployment rows. Every row had Turn 0; no
duplicate coordinate was found within a zone or across slots. The four
City Complex maps `s1-1-1.mdb` through `s4-1-1.mdb` had empty Deployment
tables and four owner-specific HQs. Colony `C1.mdb` through `C4.mdb` are
strategic databases and were not counted as tactical maps.

**The authored slot is not necessarily the effective runtime player
zone.** Native function `0x44049C` appends additional slots depending on
the active human-player count:

| Active players | Effective starting zones |
| --- | --- |
| 1 | Player 1 gets slots 1 + 2 + 3 + 4 |
| 2 | Player 1 gets slots 1 + 3; player 2 gets slots 2 + 4 |
| 3 | No merge; active players use their individual slots |
| 4 | No merge; each player uses its individual slot |

The 3-player branch does not redistribute slot 4. Append does not mean
that the source MDB is rewritten. The skirmish setup calls this helper
at `0x481EA7`; a campaign scenario-load path also calls it at `0x47E3D8`.
These are native-code findings, not a live multiplayer/network test.

Examples from the original archives, with effective counts calculated
from the native merge rule:

| Map | Authored P1/P2/P3/P4 cells | Solo cells | Two-player cells | Embedded deployment points |
| --- | --- | --- | --- | --- |
| Crime and Punishment, `s1-99-1.mdb` | 26 / 17 / 14 / 24 | 81 | 40 / 41 | 150 |
| Opportunity Knocks, `s1-2-1.mdb` | 46 / 53 / 52 / 60 | 211 | 98 / 113 | 150 |
| Opportunity Knocks alternate, `s1-2-2.mdb` | 62 / 45 / 38 / 48 | 193 | 100 / 93 | 150 |
| Jungle Ambush, `s2-6-1.mdb` | 16 / 16 / 21 / 20 | 73 | 37 / 36 | 240 |
| World Cup, `s4-20-1.mdb` | 57 / 61 / 54 / 71 | 243 | 111 / 132 | 2300 |

Crime and Punishment's slot-1 bounds are x=0..11, y=0..7, but only 26 of
those cells are in that zone. Replacing it with its bounding rectangle
would change the scenario. Green Thaw inherits those authored deployment
records. The area examples also show why `One Off Deployment` must not
be interpreted as a zone-size parameter.

#### Loading, duplicates, turns and placement eligibility

The Deployment SQL string is at `0x4B2D08`:
`SELECT * From [Deployment] WHERE ((Deployment.player)=%PLAYER%)`.
Loader `0x43F6DA` loads slots 1..4 and then removes coordinates duplicated
in later slots. The comparison is spatial, not an explicit Turn-aware
ownership rule; lower-numbered slots win in this code path. Original
maps do not exercise this overlap case, so intentional overlapping
zones should not be offered as a proven feature.

`CMapZone` record loading at `0x426515` reads PosX, PosY and Turn.
Validation at `0x4267B7` checks nonnegative coordinates and Turn 0..100.
Membership at `0x426833` requires **stored Turn == requested Turn**.
Both deployment clicking (`0x44C2C6`) and cursor validation
(`0x44C748`) explicitly request Turn 0. A nonzero Turn is therefore not
an ordinary initial-deployment cell, and is not evidence of the same
"arrive on or after this turn" behavior as the Reinforcements table.
No stock nonzero deployment-row behavior has been observed.

A cell being inside a zone is not a guarantee that a particular unit can
be placed there. Terrain, trees, occupied objects, the unit's movement/
footprint rules and available deployment points are separate constraints.
The manual describes land/water and forest-walking restrictions in
addition to the blue deployment-area display. Do not expand or shrink
stored zones merely because a chosen unit cannot use some of their cells.

#### Native deployment artwork

`IMGS\SYMBOLS.DBI` includes all 64 zone sprites, named `DEPL{player}{shape:02d}`
for players 1..4 and shapes 0..15. The editor uses their original palettes,
not a synthetic colored polygon or building artwork. The native selector
at `0x42690D` asks the zone for four neighbors at the requested Turn and
forms a mask with x-1=8, x+1=4, y-1=2, y+1=1. Its shape lookup, indexed by
mask 0..15, is:

```text
0, 13, 15, 14, 10, 1, 7, 4, 12, 3, 9, 6, 11, 2, 8, 5
```

This differs from the road/wall connection lookup. The authored-slot editor
connects only same-player Turn-0 cells; it does not connect separate player
slots even when the game would merge them for a solo battle.

#### Separate campaign/city spatial classifier

A native campaign-mode path contains a second zone builder at
`0x47F8E7`. Unlike the ordinary SQL loader, it classifies the runtime map
and replaces active player zone lists with generated Turn-0 records.
The classifier is constructed at `0x47C350`; extraction and replacement
are at `0x47C931` and `0x47C9F6`.
Campaign branches at `0x47E5D9/0x47E620` and
`0x47E692/0x47E6E9` invoke it with different AI-placement flags.
The campaign's nonzero `+0x34` branch resolves a saved `city.mdb`,
while its zero branch resolves the selected scenario.
**The exact reachable call timing and complete city/counterattack
lifecycle are not live-confirmed.** Do not claim that every campaign
deployment screen always uses only authored cells, or always uses only
generated cells, on this evidence.

Decoded spatial rules:

- The terrain pass (`0x47C3FD`) leaves terrain categories 0..3 and 5
  available to subsequent classification and marks other categories
  blocked with `0x100`.
- Ordinary owned building footprints receive markers 8, 10, 12 or 14
  for players 1..4. These numbers are **owner markers, not radii**.
  Building types with a resolved `transformIntoUnit` receive markers
  `0x40/0x50/0x60/0x70` instead. The test at `0x429550` checks the
  definition's resolved unit pointer at `+0x28`; it is not an
  `isDestructible` test. Field resolution is visible at `0x424A70`.
- The expansion pass (`0x47C6BB`) assigns still-unclassified cells within
  a square radius **3** of an ordinary owned building's marker to that
  owner's `0x40/0x50/0x60/0x70` zone. All four owners use the same 3;
  the code does not select a radius from Deploy Mk level or difficulty.
  Lower-numbered players classify contested empty cells first.
- Units and decors have blocking passes; owned wall cells receive owner
  zone markers. These do not turn all occupied cells into placeable cells.
  Player extraction includes its two exact marker values, with normal
  placement checks still needed afterward.
- The AI candidate marker is `0x80`, not a shared human-player zone.
  Its pass (`0x47C737`) uses proximity to the shared `0x40` bit in human
  zone markers, square distances 11 and 14, and a two-cell corner
  condition. Specifically, an empty cell must be near a human zone
  within 14 and either not near one within 11 or pass the edge predicate.
  That predicate requires both x and y in their outer two-cell bands,
  rather than accepting the entire map border.
  The optional generator branch uses these candidates when positioning
  AI reinforcements.
- Neighborhood helpers `0x47C7F3/0x47C892` use clipped rectangular
  searches. Their exclusive upper bound is clipped to width-1/height-1,
  so the last column/row is excluded from the neighborhood search.
  This boundary detail matters for an eventual exact engine recreation.

Thus an additional ordinary owned building can extend a generated area
because of **where it is built**, not because its deployment production
value is a radius. A Deploy Center participates as a building, but its
Mk level is primarily an economic distinction. The nominal three-cell
halo must not be substituted for authored skirmish zones.

#### Deployment-center economy is not zone geometry

Native global building definitions have the following `ResProduction`:

| Building | Type ID | Deployment-point production |
| --- | --- | --- |
| Human HQ | 130 | 50 |
| Deploy Mk1 | 131 | 100 |
| Deploy Mk2 | 173 | 110 |
| Deploy Mk3 | 174 | 120 |
| Deploy Mk4 | 175 | 130 |

The manual and encyclopedia associate these buildings with deployment
points and a maximum of ten Deployment Centers, not ten spatial radii.
The campaign points path at `0x47D4E7` counts the selected player's HQ
and the current continent's matching Mk type, sums Mk production, and
adds HQ production once if at least one HQ remains. It then applies a
deployment difficulty percentage and an active-player-count percentage.
The latter helper, `0x41EF17`, uses 100%, 75%, 65%, 55% for 1..4 players;
it is not a clamp or a zone-size calculation.

Difficulty interpolation at `0x47AA4D` uses the campaign continent's
DeployEasy and DeployHard endpoints, with integer division by 10.
`COLONIES.ZIP`'s `Continent Info` stores:

| Continent | DeployEasy | Stored DeployAvg | DeployHard |
| --- | --- | --- | --- |
| C1 | 130 | 70 | 70 |
| C2 | 125 | 100 | 75 |
| C3 | 130 | 100 | 90 |
| C4 | 140 | 100 | 90 |

The examined interpolation uses the endpoints, not the stored DeployAvg
column. Do not confuse these values with the three Deploy percentages in
`GLOBAL.MDB`'s Difficulty row, which are all 100 in this installation.
Integer conversion, initial mission allocation, per-turn accrual,
carryover and rising unit costs are distinct economy steps; the building
sum is not a universal formula for every displayed deployment balance.
None of those point calculations appears in the three-cell spatial
expansion pass.

#### Research evidence limits and editor scope

The deployment investigation above made no executable, installed asset or
editor behavior changes. It used read-only database copies and native
disassembly. A window-scoped live probe launched the isolated playtest
but did not reach a successfully observed deployment zone; therefore the
81-cell solo result and the campaign generator are not live HUD proofs.
The local research artifacts are `deployment_scan.ps1`,
`deployment-scan.json`, `deployment-sources`, `deployment_memory.py`
and `deployment-research-*.png` in the session's persistent `files`
directory. They were not added to the active modding directory.

The editor now paints/erases **authored slot 1..4 Turn-0 cells**, with a
default-hidden native-art overlay, shared history and transactional saving.
It preserves unknown-player/nonzero-Turn rows and unrelated tables. It does
not show an effective-player-count preview or regenerate city/campaign
zones. The points setting remains separate. Empty City Complex zones may
be runtime-generated and are not automatically filled or treated as broken.

### Fresh-battle initialization versus saved state

Loading an MDB and beginning a fresh battle are different operations.
Wall/building constructors initially read stored `currentAR`; building
loading caps it at type armor. Subsequent initialization explicitly
replaces building and wall health with full type armor.
There is also a conditional post-load reset of military health/AP.
Deployment completion performs another full-health building/wall pass.

Consequently, original concrete walls can store 10 but display **AR 60**
in the fresh battle. That concrete HUD value was observed live by holding
right mouse over the wall. Human HQ can store 100 and its type/reset code
specifies 500; a successful live HQ HUD was **not** obtained.
This is not an across-the-board multiplication of health by five.
The independent concrete broken-section flag survives those health resets.

A replacement engine should model stored state, fresh-scenario resets,
deployment transitions and savegame restoration separately. Do not repair
all records on import or promise pre-damaged starting walls/buildings
merely because a source MDB contains low health.

The user identifies **SX-1-1** as the player's base used for counterattacks
and says the ordinary human HQ belongs there rather than in ordinary
scenarios. This is a user-provided base-map distinction, not a complete
verified native placement prohibition. The earlier HQ-in-scenario loading
test was inconclusive; missing cache folders also caused stock-map hangs.
Its capture must not be cited as proof that an HQ necessarily crashes
the engine. Partial native deployment-generator research is recorded
above; full base/counterattack reconstruction remains deferred.

### Runtime control, debugging and message pump

cnc-ddraw supplies windowed presentation of the unchanged 8-bit renderer.
The active installation uses bordered 2048x1536 output, tactical 1024x768
scaled 2x, and original 640x480 menus. Native `InWindow=1` exited during
startup on this desktop; retained `InWindow=0` works with the wrapper.
Native viewport expansion is still unresolved, independent of this
successful scaling path.

Relevant active wrapper options are:

```ini
[ddraw]
width=2048
height=1536
fullscreen=false
windowed=true
border=true
maintas=true
boxing=false
renderer=auto
adjmouse=true
nonexclusive=true
singlecpu=true
minfps=5
savesettings=0
resizable=false
devmode=false
```

`devmode=true` is the wrapper's documented no-cursor-lock option, requiring
a restart; it was identified but not enabled. Ctrl+Tab temporarily
releases confinement under the discussed configuration. Hover alone
does not focus the game. These are wrapper behaviors, not reconstructed
native input rules.

A startup `FindWindowA` single-instance check was identified. No bypass
patch was applied and same-PC multiplayer was not tested. Two independent
copies would need separate mutable databases/caches/saves and matching
scenario content; bypassing window detection would not by itself prove
legacy network host/join works.

Prefer window/PID-scoped capture and input, not full-desktop coordinates.
`MainWindowHandle` can select a hidden cnc-ddraw CompatWindow. Enumerate
visible windows for the PID and choose the actual game client; the local
HUD helper chooses the largest visible client. Set thread DPI awareness
to physical pixels and scale input against measured client dimensions,
not the displayed/downsampled screenshot. Foreground capture works for
DirectDraw; PrintWindow can return black. Later targeted mouse messages
worked for menu/deployment actions, despite unreliable early probes.
Holding right mouse over an object displays its stats; single click
places a deployment unit, while double click can remove it.
Act promptly during timed deployment and inspect the bottom-left timer.
Respect desktop ownership/arbitration and stop input when focus changes.

For the loading stall, the process stayed alive/responding and used CPU;
no matching crash event was found. A local ctypes helper sampled the
WOW64 main-thread registers/frame chain three times and captured a full
hang dump before the isolated process was stopped. Each sample was in:

```text
win32u -> user32 -> cnc-ddraw hooked PeekMessageA
       -> LIBDAY return 0x485F62 -> caller 0x403F7F
```

The native pump starts at 0x485F38 and calls PeekMessage with PM_NOREMOVE.
In the sampled run, the main thread had consumed approximately 244.8 seconds
of CPU while the other roughly 34 threads were mostly waiting. The runtime
PeekMessage hook was 0x557D2AA0, with a sampled cnc-ddraw return at
0x557D2B06; these DLL addresses are run-specific, not stable code anchors.
The captured MSG had WM_TIMER (0x113), no window handle, wParam 0x660B
and timer callback 0x487914. That callback reads global pointer 0x4F45E0
and calls 0x487AC7; its 100 ms timer is created near 0x4878EF.
The application object was 0x4F1FC8, vtable 0x4ACD24; the MSG buffer
was app+0x34, not another application object.
Vtable offsets +0x64/+0x68/+0x70 pointed to 0x49BB7E/0x49BB78/0x403FBF.

These samples locate execution, not the root cause. The wrapper happened
to hook the message API; the successful cache-directory fix required no
wrapper/executable modification. Main-thread CPU use or a stack in
PeekMessage must not automatically be called a deadlock or wrapper bug.

Use a 32-bit debugger for richer tracing if available. At this investigation,
cdb, WinDbg, ProcDump and gdb were not found in the checked paths.
The custom sampler briefly suspends a specific thread, obtains a
716-byte WOW64 context, and resumes in a finally block. A 64-bit dbghelp
dump of this WOW64 process contained 3263-byte mixed/native contexts;
naively interpreting those blobs with 32-bit context offsets is invalid.
Keep full memory dumps local: they can contain unrelated runtime data.

### Persistence tooling and reconstruction fixtures

`map_editor_core.py` is the executable reference for DBI decoding,
projection, schema normalization, connection lookup and editing state;
`map_tool.ps1` is the independent database-validation/persistence reference.
`map_editor.py` defines preview composition and UI-only policies.
The reduced CSV registries are readable local exports, not substitutes
for all original global definitions.

Jet editing uses 32-bit Windows PowerShell 5.1 with ADODB/Jet OLEDB 4.0.
64-bit PowerShell cannot supply that provider. The protected GLOBAL.MDB
rejected Jet reads without a valid password; locally extracted MDB Tools
could read its tables and supplied the registries. A standalone DAO 3.5
COM licensing failure is not proof that the game's licensed MFC/DAO
startup fails. Do not apply global DAO installers as a speculative repair.

Each document snapshots its source. Save Copy writes to temporary copies,
validates independently in the bridge, commits requested map/object/
metadata changes together in a transaction, and publishes a new file
without overwriting. Failures roll back and remove incomplete output.
ZIP replacement/addition preserves other members' bytes and metadata.
Flat MDB names of at most 13 characters are a **packaging-tool rule**;
the successful exported `edit1.mdb` demonstrates that canonical
`sN-N-N.mdb` naming is not required for this tested skirmish path.
Native campaign path formatting still uses the canonical pattern.

Save snapshots include all layers and settings before the worker starts.
Starting/reinforcement conversion preserves the game ID but gets a new
destination-table Counter on save; an unsaved conversion back can restore
the original Counter and hidden fields. Scenario edits retain localized
metadata, objectives, preview blobs, support availability and unrelated
rows. Full-table snapshots encode binary fields for comparison;
checking only Map rows is not sufficient preservation evidence.

Useful durable fixtures in `modding\` are Green Thaw and its template/CSV/
archive/catalog copies, the Enclosure one-cell edit, global definition CSVs,
and `archive\libday.disasm.txt`. Additional research artifacts are currently under:

```text
C:\Users\maxg\.copilot\session-state\fbd15b89-14f5-4e9d-a4a7-21346dfc9acf\files
```

| Artifact | Use and evidence limit |
| --- | --- |
| `unit-types-full.csv` | Full global military definition export, including costs omitted from the editor's reduced CSV |
| `unit-scenario-scan.json`, `unit-scenarios\` | Campaign schema/arrival research, including World Cup |
| `snapshot_mdb.ps1`, preservation-before/after JSON files | Read-only full-table and binary-field comparison |
| `wall-held-hud.png` | Live concrete AR60 evidence |
| `scenario-settings-demo.mdb`, `scenario-settings-skirmish\` | Edited metadata/roster fixture and matching catalog/archive export |
| `scenario-settings-playtest\` | Repaired isolated installation; not an installed-main-game modification |
| `scenario-selector-demo-selected.png`, `scenario-settings-tech-loaded.png` | Successful selector and four-type/2100-technology setup |
| `scenario-settings-deployment-ready.png`, `scenario-settings-units-placed.png` | Deployment budget 420, then 410 after placing two Troopers |
| `scenario-settings-battle.png` | Playable edited battle with displayed 12-turn limit |
| `hang_probe.py`, `scenario-loading-stack.json`, `scenario-loading-hang.dmp` | Local native sampler, three stack samples and approximately 315 MB full hang dump |
| `hud_probe.ps1` | PID/visible-window-scoped HUD/input research helper |
| `hud-hq-stored-100.mdb`, HQ loading captures | Inconclusive base-HQ experiment, not a playable fixture or live HQ500 proof |

Artifact paths are a retrieval aid, not a promise of permanent availability.
The core findings are written here so loss of a conversation or session
artifact does not lose the format/behavior discoveries.

### Movement, action points and pathfinding

Recovered from LIBDAY.EXE (terrain/movement cost table at `0x4f35a0`) and
checked against gameplay video; implemented in `libday-sim::path`.

`UNITTYPES.movement` ids: 0 none_ground, 1 ground, 2 trooper, 3 grav,
4 boat, 5 underwater, 6 air, 7 teleport, 8 grav_fly, 9 none_water. Per
terrain kind, a type can *stop* (end a move there, cost 10 unless noted),
only *pass* over it, or not enter:

| Terrain | Stop | Pass only |
|---|---|---|
| Shallow water (0), riverbank | grav, teleport, grav_fly | air |
| Deep water | grav, boat, underwater, teleport, grav_fly | air |
| Plain | ground, trooper, grav, air, teleport, grav_fly | - |
| Forest | trooper (cost 13) | air, grav_fly |
| Mountain (4), crack (6) | nobody | air, grav_fly |

none_ground/none_water never move. Roads apply to ground and trooper
units only: road 7, paved road 4. A bridge (road on water) is a stop tile at
road cost for ground/trooper, a stop tile at 10 for grav/air/grav_fly/teleport,
blocked for boats and pass-only (underneath) for underwater units. Other
units, buildings, walls and blocking ruins are pass-only for flyers and
blocking for everyone else; grav mines block everyone, normal mines block
everyone but grav/air/grav_fly/teleport (mines hidden from the mover's side
are ignored by the planner).

Search is Dijkstra over 8 neighbours in direction order SW, W, NW, N, NE,
E, SE, S (facing indices 0-7, the `M<facing>` suffix of walk animations);
a diagonal adds 1 to the search cost only, not to the AP charged. The path
must end on a stop tile. With overwatch on, the preview budget is AP minus
`actionPointsShoot`; when executing, the reserve applies only if the unit
still has at least that much AP. The unit stops at the last affordable stop
tile (flyers may overspend to reach the next stop tile; AP clamps to 0).

Path dots are `SYMBOLS.DBI` `PATH0000`-`PATH0005`: blue (stop tile), dark
blue (pass-only), red (destination), each with a crossed variant for steps
beyond the budget. Walk animations are `ANMSUNIT\ANMSUNIT.FF` members
`IMG\P<owner>\IMG<owner>M<facing>.SMK` (216x224 frames, 15 fps, assumed
centred on the 54x56 sprite cell); the unit advances
`MoveAnimation_PixSpeed` x 15 map pixels per second. Start/stop (`L`/`N`)
animations, movement sounds and troopers hidden in forest are not yet
reproduced.

### Savegames

Each original save slot is a directory `SAVEGAME\savegame.NN` holding an
empty `<name>.dsc` (its file name is the display name), `GAMEINFO.MDB`,
`COLONY.MDB` and `CITY.MDB`, plus `SCENARIO.MDB` **only when the game was
saved during a battle**. In the shipped campaign saves, slots 01 (Scrap
City, s1-7-1, turn 9) and 09 (Cat and Mouse, s3-11-2, turn 1) are battle
saves; the rest are campaign-only.

`SCENARIO.MDB` is a full scenario with the same schema as the stock maps,
rewritten with the current state (*observed*, by diffing against the stock
`.mdb`):

- Destroyed terrain is stored by swapping the cell image to the burnt `X`
  variant (for example `NORD000` -> `NORX000`); 149 cells differ in slot 01.
- Destroyed units, buildings and roads are deleted; arrived reinforcements
  leave `Reinforcement` and appear in `Units On Map` with new IDs past the
  previous maximum. `Scenario Info.NextID_Units` is **not** updated.
- Units keep `currentAR`/`currentAP` and their order fields. `Turn` and
  `Probability` in `Units On Map` are AI order expiry/retention (they are
  not arrival turns); `negAP` was always 0.
- Buildings and walls carry their current AR (after the fresh-battle reset
  described above).

`GAMEINFO.MDB` holds the battle globals: `Tactic IO` (Current Player,
Current Turn, Objective Turn, Last Turn), `Server Mode Info` (Difficulty,
Turn Time), `Player Tactical` (Player ID, Deploy Res) and the tallies
`Player Tactical Units Destroyed/Lost` and `Buildings Destroyed/Lost`
(Player, Unit/Building Type, Number; player 0 rows included).

Not saved by the original: per-unit overwatch, the elapsed turn timer and
the selection. `Autofire`/`Overwatch` in `LIBDAY.INI` (next to
`ShowMoveAnim` and `ScrollSpeed` in the executable's key strings) are default
preferences, not per-battle state.

openlibday's own format (`libday-data::battle`, `"format":
"openlibday-battle"`, `"version": 1`) is JSON. It stores the source
scenario (a stock `.mdb` name, or the original save's `SCENARIO.MDB` path),
the clock, difficulty, turn time, autofire, per-player records, every
dynamic layer in full (units including overwatch and orders, buildings,
walls, roads, mines, ruins, surfaces, order targets) and only the terrain
cells that differ from the source. Static tables (info, deployment zones,
objectives, texts) come from the source. On import, the stock map is
identified by title and size with the fewest terrain differences (at most
a quarter of the cells), so converted saves do not depend on save slots
that the original game may overwrite. Readers reject other formats and
newer versions.

### Executable bitmaps and the right-button unit info

Some interface art is not in any DBI, SMK or FF file. It exists only as
`RT_BITMAP` resources (uncompressed 8-bit DIBs) inside `EXE\LIBDAY.EXE`, so a
replacement engine must load it from the PE file at runtime
(`libday_formats::pe::bitmap`, `GameData::exe_bitmap`) and never ship it:

| Id | Picture | Use |
|---|---|---|
| 181 | dark figure | stealth (`Jammer` = 0) |
| 182 | tree and man | moves through forest (`Movement` = 2) |
| 183 | arc and fire | indirect fire (`weaponType` = 1) |
| 184 | radar dish | mine detection (`MineDetectionRange` > 0) |
| 185 | crossed-out plane | anti-air (`IsAA`) |
| 309 | yellow star | reduced signature (0 < `Jammer` < 100) |
| 305–308 | `LBULB` light bulb | the blinking bulb on every technology building in the MAP overview: bright, mid, dark, mid, one frame per 175 ms, black transparent (`0x459530`). The captured-building bulb on the map is `LILBULB` in `SYMBOLS.DBI` (see `docs/tech.md`) |
| 240–243 | coloured flags | not yet identified |

While the right button is held on any unit, the minimap well shows the unit
info instead (`0x45593D`; bitmaps loaded at `0x454FD5`). Offsets are relative
to the well: a 38×38 portrait at (8,6), and 14-pixel text lines from y 6.
The first three lines (name, `AR n`, `AP n`) sit beside the portrait at x 49.
The next lines are at x 8: `RANGE n`, `OVERWATCH YES|NO` and the owner name
(`Alien` for owner 0). The strings are `LIBDAYEN.DLL` ids 0x280–0x285 and 0x41.
YES means the current AP covers one shot.

The capability icons (`0x455DCC`) are 25×25 and laid out left to right with
a 30-pixel step from (8,87), in the order 185, 184, 182, 181, 183, 309.
Only the icons whose condition holds are drawn.

### What remains unknown for a replacement engine

The current work establishes data import, substantial static rendering,
fresh-skirmish loading observations and selected lifecycle rules. It does
**not** reconstruct a complete simulation. Open areas include:

- Combat damage/accuracy, range/LOS, armor interaction, overwatch/autofire,
  turn-level action-point rules beyond movement.
- AI order interpretation, Probability/EnemyUnitType semantics, convoy
  behavior, objective-kind rules and exact turn scheduling beyond the
  reinforcement paths already traced.
- Campaign economy/research/production, technology caps and costs,
  colony/base layouts, SX-1-1 counterattacks and HQ mode requirements.
- Native animations, effects, burn transformations, player-color remapping,
  draw-order edge cases and DBI metadata beyond the decoded fields.
- Stored mapScreen preview encoding/regeneration, campaign-state save
  fields (`COLONY.MDB`, `CITY.MDB`) and which load paths reset versus
  preserve health/AP.
- Complete networking/session protocol, two-instance host/join and
  synchronization/determinism.
- Genuine native-resolution viewport expansion, including surfaces,
  clipping, interface layout and input coordinates.

For reconstruction, keep raw data import lossless and implement mode-specific
initialization explicitly. Use the recorded fixtures as golden inputs,
compare named native HUD/screens rather than merely window bounds, and
trace the remaining behavior before replacing uncertain rules with guesses.

## Historical Windows investigation handoff

This section retains the earlier WSL/native-widescreen handoff. Later
windowed gameplay, map-layer editing and scenario-loading fixes above
supersede its unresolved-control/loading notes. It is not a current task list.

The earlier requested target was **1920x1080, starting with a genuinely larger native
tactical viewport**, plus custom maps. Wrapper scaling is a separate fallback,
not completion of the native-resolution goal. Start a new Windows investigation
by reading this README and `archive\evidence.json`; the useful outputs are in this
directory, not dependent on retaining the previous WSL conversation.

### What we learned about the engine and data

The game is a native **32-bit x86 Windows GUI executable**, not DOS, .NET or
Electron. `EXE/LIBDAY.EXE` is 1,101,312 bytes, with PE image base `0x400000`
and a linker timestamp of March 4, 1998. The supported original SHA-256 is:

```text
c138adb009c3fdb7b9090572d68f52ec59e3019ca8d5fc53fac9a16c52ba5389
```

Imports identify MFC42, DAO-related database functionality, ArchiveLib
`AL21MFC.DLL`, DirectDraw, Smacker video and Miles audio. Findings so far come
from executable bytes/objdump, MDB table readback and bounded Windows runtime
probes, not recovered source or a working decompiler session. REA was present
in WSL, but its MCP integration/native analysis engine was unavailable.
Ghidra and x32dbg are proposed next tools, not tools already configured.

The original display choices are 640x480 and 1024x768, selected by
`[User] Resolution=0/1`. DirectDraw mode setup requests **8-bit color**.
The original INI also documents `[Fallen Haven II] InWindow=1` as avoiding
display-resolution changes; that option is distinct from cnc-ddraw's settings.
The UI has resolution-specific `INTERF/640X480.SMK` and `1024X768.SMK`
assets. Changing the display table alone therefore does not establish a
resized interface, larger visible map area or correct mouse hit-testing.
Use the disassembly anchors below to continue tracing those separate paths.

Scenario content is stored in **Access MDB files inside ZIP archives**:

| Resource | Observed role |
| --- | --- |
| `SCENS/DEATHM.ZIP` | 10 deathmatch MDB maps |
| `SCENS/SCEN1.ZIP`, `SCENS/SCEN2.ZIP` | 55 and 51 MDB members respectively |
| `COLONIES/COLONIES.ZIP` | Colony databases C1 through C4 |
| `SCENS/DEATHM.MDB`, `SCENS/SKIRMISH.MDB` | Selectable scenario catalogs |
| `GLOBALS/GLOBAL.MDB` | Terrain, unit and building definitions |
| `GLOBALS/GAMEINFO.MDB` | Player/game state |

The map loader derives equal width/height from the square root of the terrain
row count. This is why the current editor requires a complete square grid and
preserves dimensions. Other scenario tables hold units, buildings, mines,
roads, walls, deployment, objectives and reinforcements; terrain editing alone
does not create a fully designed mission. Binary scenario preview fields are
preserved but their encoding/regeneration has not been solved.

The independently checked deathmatch sample changes Enclosure cell
`Id=56180`, `(0,0)`, from `BORE146` to `BORE000`, preserving the other 21
tables. Its copied catalog has the original ten entries plus `custom.mdb`
("Custom Enclosure", Counter 11). The skirmish sample changes Crime and
Punishment cell `Id=6644`, `(0,0)`, from `NORD000` to `BORE000`;
its template is `s1-3-1.mdb`, skirmish Counter 3. Neither sample has verified
in-game tactical play.

### Earlier playtest state and runtime pitfalls

Both `playtest/EXE/LIBDAY.EXE` and `LIBDAY-mod.exe` were restored to the
original executable bytes. The native-resolution experiment is disabled.
The active isolated wrapper is cnc-ddraw, and `playtest/SCENS/SCEN1.ZIP`
is original again; the copied deathmatch catalog/archive retain the custom
entry. No probe-owned game process was intentionally left running.
Recheck these files before changing the baseline in a new session.

`PrintWindow` produced black DirectDraw captures. Visible capture worked for
the foreground game, but DPI virtualization initially reported 1280x720
bounds for a physical 1920x1080 window. The current probe uses
`SetThreadDpiAwarenessContext(-4)` and physical-pixel coordinates. Posted
mouse messages were unreliable; foreground-targeted physical input still
did not establish dependable tactical navigation. Black screenshots and
loading screens alone do not identify whether the problem is rendering,
map loading, compatibility or automation.

Some runs showed a "CompatWindow" title; renaming the isolated executable
changed observed behavior, but compatibility-shim causation is unproven.
A standalone `DAO.DBEngine.35` COM probe returned a licensing error.
That does **not** prove the game's licensed MFC/DAO initialization fails.
Do not bypass licensing or run `DAO/_install.cmd` as a speculative fix:
that installer copies/registers DLLs globally. No global registry or
permanent execution-policy changes were made during this investigation.

### Earlier resumption plan and current corrections

The original plan below predates the working Windows MCP controller.
Window-scoped capture/input now works, the editor supports units and
scenario text/settings, and the edited demo has reached a playable battle.
Preview regeneration and native viewport expansion still remain open.

1. Work from `F:\libday` in an interactive Windows desktop under a normal
   user account. Direct edits are authorized; use the existing isolated copy
   when a controlled comparison is useful. Its shared artwork paths now
   point to `F:\libday`. First establish that an unmodified game can
   reach a tactical map with ordinary human input; record executable,
   wrapper and scenario identity for each experiment.
2. Use the now-working Windows MCP controller with foreground/PID restrictions
   and DPI-correct client coordinates. The early `probe_game.ps1` launches
   and closes its own process per run; it is not a persistent controller.
   The later `hud_probe.ps1` operates on an existing game PID.
   `/remote` is CLI-session steering, not desktop control.
3. Use Ghidra for static analysis and x32dbg for the 32-bit runtime if
   installed/configured. Follow the display anchors below into surface
   allocation, clipping, drawing, UI layout and input-coordinate mapping.
   Keep the eight-byte experimental patch disabled until those paths agree.
   Prove a larger visible tactical area, not just a larger output window.
4. Verify loading and playing each specific edited fixture rather than
   assuming another successful fixture covers it. Preview regeneration remains
   unfinished; native objective editing and square resizing are now supported
   by the editor, but resized-map playability still needs live-game verification.
   Clear only the relevant extracted MDB under the configured cache
   subdirectory while the game is closed; an MDB misplaced in EXE is a
   known symptom of missing extraction directories, not the normal path.

PowerShell 7 is suitable for the native Copilot CLI session, but the Jet-based
map editor specifically needs **32-bit Windows PowerShell 5.1**, as described
below. WSL can still supply objdump and MDB Tools; their isolated binaries were
kept in the prior session's Linux artifacts, not bundled as Windows tools.
The disassembly, exported terrain registry and evidence in `modding/` remain
usable without those temporary tool locations.

## Archived files and isolated Windows copy

All filenames in this section are relative to `modding\archive`.
The playtest INI still contains pre-archive paths; treat this as historical
material and restage before running it.

`playtest/EXE/LIBDAY-mod.exe` is an **unpatched** copy of the original executable.
The isolated copy contains separate globals/game state, scenario archives,
savegames, cache, DLLs and music. Its `LIBDAY.INI` points those writable paths into
`playtest`; large artwork/animation/video paths still point at the original
installation. The original installation must remain available.

The isolated `ddraw.dll` is cnc-ddraw, with `ddraw.ini` configured for a 1920x1080
borderless window and aspect-ratio preservation. This is **scaling**, not a
larger native viewport. At 4:3, the game content fits a 1440x1080 area inside that
window. The isolated original GOG DLL is retained as `ddraw.gog.dll`.
`cnc-ddraw-LICENSE` contains the upstream license; source and instructions:
<https://github.com/FunkyFr3sh/cnc-ddraw>.

`LIBDAY-1080.EXE` in this directory is the **experimental, display-table-only**
patch. It is not the active executable in `playtest`. Do not replace your
original executable with it.

The sample `custom-edited.mdb` changes one tile in the deathmatch Enclosure
template. `DEATHM.MDB` and `DEATHM.ZIP` are staged catalog/archive outputs with
the additional `custom.mdb` member. `skirmish-edited.mdb` and `SCEN1-custom.zip`
change one tile in the Crime and Punishment scenario. After the final control
experiment, `playtest/SCENS/SCEN1.ZIP` was restored to its original contents.

## Terrain editing

Python commands run from the game root in WSL and require only Python's standard
library. Outputs must not exist. Create a separate work directory first.

```bash
mkdir modding/work
python3 modding/libday_mod.py extract-map SCENS/SCEN1.ZIP s1-3-1.mdb modding/work/template.mdb
```

Run `map_tool.ps1` with **32-bit Windows PowerShell**. On this Windows
installation its executable is
`C:\Windows\SysWOW64\WindowsPowerShell\v1.0\powershell.exe`.
If local script execution is blocked, use an explicitly approved process-only
`-ExecutionPolicy Bypass` invocation; do not change machine-wide policy.
In a 32-bit PowerShell session with the game root as working directory:

```powershell
.\modding\map_tool.ps1 -Action Export `
  -Source .\modding\work\template.mdb -Output .\modding\work\terrain.csv
```

Edit only `TerrainImg` in the CSV. Keep every `Id`, `PosX` and `PosY` unchanged.
`terrain-types.csv` was exported from this installation's
`GLOBALS/GLOBAL.MDB` and provides the valid terrain IDs. Unknown IDs, missing or
duplicate cells, coordinate changes and resizing are rejected.
The editor's explicit `-ResizeMap square` path permits complete square-grid
resizing with building, surface, unit, deployment and script snapshots supplied
together. It preserves retained ID/coordinate pairs, inserts fresh Map rows and
deletes only cropped Map rows inside the output transaction. Final spatial checks
reject newly cropped objects/targets, preserving pre-existing off-map legacy rows.
Ordinary terrain-only Apply retains the strict unchanged-grid rule above.

```powershell
.\modding\map_tool.ps1 -Action Apply `
  -Source .\modding\work\template.mdb -Output .\modding\work\edited.mdb `
  -Csv .\modding\work\terrain.csv -TerrainCsv .\modding\terrain-types.csv
```

`Apply` creates a copy and updates its terrain transactionally. Other tables are
preserved. It does **not** regenerate `mapScreen640` / `mapScreen1024` preview
blobs, so preview/minimap artwork may remain stale. Terrain painting can also
make existing units, buildings or deployment positions inappropriate; validation
does not establish playability.

To replace the existing scenario slot in a new archive:

```bash
python3 modding/libday_mod.py pack-map SCENS/SCEN1.ZIP s1-3-1.mdb \
  modding/work/edited.mdb modding/work/SCEN1.ZIP
```

To add a new same-size map instead, use a new flat filename, maximum 13
characters including `.mdb`, and clone the matching catalog template:

```bash
python3 modding/libday_mod.py pack-map SCENS/SCEN1.ZIP custom.mdb \
  modding/work/edited.mdb modding/work/SCEN1-custom.ZIP
```

```powershell
.\modding\map_tool.ps1 -Action Register `
  -Source .\SCENS\SKIRMISH.MDB -Output .\modding\work\SKIRMISH.MDB `
  -Member custom.mdb -Title "Custom Skirmish" -TemplateCounter 3
```

Counter 3 matches the `s1-3-1.mdb` template. Use `DEATHM.MDB` / `DEATHM.ZIP`
for deathmatch maps, with a matching deathmatch template counter.
Registration changes the catalog titles, not the internal scenario title or
mission descriptions.

Install the generated catalog/archive **only in an isolated copy**, with the
original archive filename (`SCEN1.ZIP`, `DEATHM.ZIP`, etc.). Do this while the
game is closed. On the current installation the game extracts selected MDBs
under the configured cache directory: for example,
`CACHE\SCEN1\s1-99-1.mdb`. Remove only the corresponding cached member in
the installation being edited before replacing a previously loaded scenario.
Do not assume it is in `EXE`; check the INI's cache path and the actual file.
Never delete unrelated MDBs or savegames.

To refresh the terrain registry on a Linux system with MDB Tools available:

```bash
mdb-export GLOBALS/GLOBAL.MDB 'Terrain Types' > modding/work/terrain-types.csv
```

Jet OLE DB rejected the installed global database with a password error, whereas
MDB Tools read its tables. The editor accepts the exported registry rather than
silently skipping terrain validation.

## Reproducing an isolated copy and experimental patch

Both commands refuse an existing output:

```bash
python3 modding/libday_mod.py stage-game . modding/new-playtest \
  --windows-output 'F:\libday\modding\new-playtest'
python3 modding/libday_mod.py patch-display EXE/LIBDAY.EXE \
  modding/work/LIBDAY-1080.EXE --width 1920 --height 1080
```

`--windows-output` must be the actual Windows path corresponding to the WSL
output directory. A newly staged copy copies the installed DirectDraw wrapper;
it does not acquire/install cnc-ddraw or copy its `ddraw.ini`. Copy that
configuration separately to retain the installed window settings.
No registry changes are made.
Staging copies files without permission/timestamp preservation for compatibility
with Windows-mounted filesystems.

Staging also creates an empty cache subdirectory for each scenario archive,
including `CACHE\SCEN1`, `CACHE\SCEN2` and `CACHE\DEATHM`. The native loader
requires these directories to exist before extraction. If they are absent,
the extractor can leave an MDB in `EXE` instead, while the game remains at
**LOADING SKIRMISH**. Creating the missing folders resolved this failure for
an untouched stock scenario in an isolated installation. No executable patch
or colony-template MDB was needed. Existing cached maps are not copied.

The patch accepts only the exact supported executable SHA-256 and changes eight
bytes in mode 1. It leaves mode 0 (640x480 menus) intact. Replacing every occurrence
of 1024 or 768 would corrupt unrelated palette/buffer allocations.

## Disassembly anchors

Addresses are preferred-image virtual addresses, image base `0x400000`.
These are observations from objdump/file bytes, not REA Evidence IDs.
Do not calculate every VA by adding the image base to a raw file offset.
For this build, `.text` has RVA `0x1000` and raw offset `0x400`
(code VA = raw + `0x400C00`); `.rdata` has RVA `0xAC000` and raw
offset `0xAA800` (VA = raw + `0x401800`). Map other sections from the
PE headers. Runtime DLL addresses in hang samples can move with ASLR.

| Address / file offset | Observation |
| --- | --- |
| `0x40AB00` to `0x40AB1C` | Reads `[User] Resolution`, accepts only 0 or 1, stores it at app offset `0x538` |
| `0x4BA000` / `0xB8800` | Two width/height pairs: 640,480 and 1024,768 |
| `0x4858E7`, call at `0x485935` | Reads that table and calls DirectDraw `SetDisplayMode`, 8-bit color |
| `0x485B0A` | Classifies the current display by comparing its width with the mode table |
| `0x4E74C0` / `0x4E74C4` | Pointers to the 640x480 and 1024x768 Smacker interface filenames |
| `0x43A208`, especially `0x43A266` to `0x43A2D4` | Queries `SELECT * From map;`, takes the square root of row count, sets equal map dimensions |
| `0x42F072`, `0x4385B2` | Road/wall connectivity: X- = 8, X+ = 4, Y- = 2, Y+ = 1; sprite lookup is 0,9,10,7,8,13,12,2,11,15,14,5,6,3,4,1 |
| `0x42F1C7`, `0x42F563` | Riverbank bridge orientation/variants and terrain-to-road artwork mode |
| `0x438552`, constant `0x4B1D78` | Wall damaged artwork when current armor is at most maximum armor times 0.33 |
| `0x437D7C` to `0x437D8C`, `0x42781D` to `0x42786D` | MDB wall/building constructors initially load stored `currentAR`; building loading caps it at the type maximum |
| `0x4384DC`, `0x42821B` | Full-health setters replace wall/building current armor with the type definition's maximum, not a scaled MDB value |
| `0x40D598` to `0x40D5A9`, `0x43F34F` | Conditional post-load initialization resets military-unit health/AP, building health and wall health |
| `0x47F537`, `0x47E842`, `0x47FE24` | Deployment-completion initialization explicitly resets all building/wall health again before battle |
| `0x4332B7` | Starting-unit MDB constructor loads type, technology, owner, facing, health/AP and separate AI-order fields |
| `0x4396C8`, `0x439979` | Reinforcement MDB constructor and validation: UnitID below 1000000, type, position, turn 0..100, order flag |
| `0x416576` to `0x416676` | Reinforcements are attempted when their native turn is at most the current turn; successful arrivals are removed from the pending list |
| `0x41671C` to `0x4167CE` | Reinforcement order flag derives an order/target collection from the objective, then sets expiry 99 and retention 100 |
| `0x433C52`, `0x43440C`, `0x434410` | Per-unit authored ID/tile target loading and list accessors |
| `0x470D71`, especially `0x470DA7` onward | Authored AI action factory; clears orders when unit Turn is less than current turn |
| `0x470B55`, `0x46FCBD` | Inclusive retention roll, order clearing and repeatable AI planning state machine |
| `0x4343EC`, `0x46D016` to `0x46D047` | EnemyUnitType access and matched-type target-score weighting |
| `0x46F2AE`, `0x46F477`, `0x46F5DD`, `0x43F9B7` | Automated-event construction, validation, list loading and world integration |
| `0x46FEB0` to `0x46FF2B` | Authored support scheduler: exact arrival equality, request, unconditional row consumption |
| `0x470ACF`, `0x4751F1` | Support request and category-specific native server factory |
| `0x470658` | Separate dynamic AI support selection; not the authored event scheduler |
| `0x47266B` to `0x472961` | Drop payload selection, relative landing offsets, placement and runtime unit generation |
| `0x4723EB` | Bomb effect handler; reads GLOBAL properties and scales base damage by remaining aircraft armor |
| `0x43F41F`, `0x43C37A` | Native objective factory and kind/timing/target-list loader |
| `0x43C067`, `0x43C7B3`, `0x43C70F`, `0x43C987` | Destroy, Hold Position, Escort and Protect predicates |
| `0x43C87A`, `0x43C8FB`, `0x43C11C` | Intercept, Keep and Destroy All Except predicates |
| `0x4405D4`, especially `0x44065A` and `0x44067B` | Reinforcement placement constructs a military unit for owner 0, with runtime-assigned properties; occupied placements can fail and remain pending |
| `0x475562` | Current turn is passed directly to the displayed turn-number formatter, without an editor-side offset |
| `0x457C72` | Map-image allocation uses dimensions from scenario image getters; not established as the main viewport |
| `0x465275` and `0x46528A` | Builds and opens the skirmish catalog path |
| `0x47A9F8` | Formats campaign scenario paths as `SCEN%d\s%d-%d-%d.mdb` |
| `0x427870`, `0x429570` | Building PosX/PosY loading into object +0x28/+0x2C and type footprint-size access; dimensions extend toward increasing coordinates |
| `0x43970E`, `0x439790`, `0x4397A6` | Reinforcement UnitID -> object +4, turn -> +0x1C, order flag -> byte +0x20 |
| `0x47FAA3` | Separate startup handling of turn-0 reinforcement records |
| `0x474116` | Native current-turn getter, object +0x2C |
| `0x43C678`, `0x47D490` | Shared objective timeout logic and global turn-limit getter; not a replacement for all objective-specific TurnNumber rules |
| `0x4652B8` to `0x465366`, `0x4EE8D0` | Skirmish difficulty selection and thresholds `(300,500,1000,-1)` |
| `0x4683B5` | Catalog deployment-budget getter, member +0x10 |
| `0x4B2E60` | Embedded `One Off Deployment` field-name string; referenced in metadata-loading paths near `0x43E9FA` and `0x4817BB` |
| `0x4B9130`, `0x4B9128`, `0x4B9120`, `0x4B9118` | C1/C2/C3/C4 MDB filename strings |
| `0x4EFDF0` / raw `0xEE5F0`, `0x47E0A6`, `0x47E9BC` | Colony filename pointer array and indexed consumers; the array VA must come from actual section mapping |
| `0x485F38`, `0x485F5C`, `0x485F62` | Main message pump, PeekMessage PM_NOREMOVE call and sampled return site |
| `0x4AC774` | PeekMessageA import slot, hooked by cnc-ddraw at runtime |
| `0x4878EF` to `0x4878F8`, `0x487914`, `0x487AC7` | 100 ms timer creation, callback and callback's downstream routine |
| `0x4F1FC8`, `0x4ACD24`, `0x4F45E0` | Sampled application object, application vtable and timer callback's global pointer |

The tactical MDB `Map` schema is `Id` (long), `PosX` / `PosY` (byte),
`TerrainImg` (text, length 9). The observed Enclosure map has 2304 rows, a
48x48 grid. Crime and Punishment has 576 rows, a 24x24 grid.
Scenario catalogs use `[Scenario Info].[FileName]`, localized titles,
`[One Off Deployment]`, and `Size`.

Native viewport expansion remains unresolved: surface allocation, clipping,
interface layout, input coordinates, and preview/overview drawing all need to
agree with any new dimensions. Scaling does not resolve those requirements.

## Verification and runtime probe

```bash
cd tools/modding && python3 run_tests.py --slow
objdump -d -Mintel EXE/LIBDAY.EXE > modding/archive/libday.disasm.txt
```

The tests cover exact patch bytes, original preservation, unsupported builds,
invalid dimensions, archive extraction/repacking and isolated staging.
Independent MDB Tools readback verified the Enclosure edit changed exactly one
cell and preserved the other 21 tables, including binary preview fields.
Catalog readback verified one added entry and preservation of the ten originals.

`archive\probe_game.ps1` is opt-in Windows 10/11 runtime instrumentation. It launches one
specified executable, waits, optionally clicks only within its foreground
window, captures that window, and closes only the process it started.
`-VisibleCapture` is required for DirectDraw content when `PrintWindow` returns
black. `-Clicks` uses physical window-relative pixel pairs such as `"320,212"`,
not logical in-game coordinates. Do not run click probes while interacting with
other applications.

`archive\scaled-menu.png` records the scaled menu; `archive\tactical-setup.png` records the
native-mode loading screen. The remaining probe captures show unresolved
rendering/navigation and must not be treated as successful gameplay evidence.
See `archive\evidence.json` for artifact identities and coverage limits.

Do not redistribute the game's executable, databases, artwork, or modified
copies. The tools operate on this local installation.
