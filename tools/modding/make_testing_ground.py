"""Builds the "Testing Ground" skirmish (catalog Counter 55, s1-99-1.mdb).

A 96x96 map for scripted tests. It uses the terrain of the stock 96x96
campaign map s3-1-1 (rivers, lakes, banks, forests and mountains), and has:

* player 1: three starting units of every unit type, all races, on the west
  side (boats on the north-west lake and river), plus a deployment zone;
* AI: one of each non-human unit type and one of each building type on the
  east side (water buildings and boats on the south-east sea);
* a neutral middle with paved and dirt roads, two built bridges, two unbuilt
  bridge spans, minefields (AI hidden normal/grav mines, player 1 mines),
  every wall type, building rubble, wrecks and scenery.

Every player-1 unit is at least 20 tiles (Chebyshev) from every AI unit.

The scenario gets 0 deployment points (first in the skirmish list) and the
openlibday-only `Scenario Info.TechPoints` column set to 0, which makes
openlibday skip the technology screen. The original game ignores the extra
column.

Usage (Windows, needs the 32-bit Jet provider used by map_tool.ps1):

    python make_testing_ground.py --root F:\\libday [--install]

Without --install the scenario database is written to the working folder
only. With --install, SCENS\\SCEN1.ZIP and SCENS\\SKIRMISH.MDB are backed
up once (*.pre-testing-ground) and updated in place.
"""

from __future__ import annotations

import argparse
import csv
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

from libday_mod import extract_map, pack_map  # noqa: E402
from map_editor_core import (  # noqa: E402
    Document, ScenarioSettings, load_terrains, run_map_tool, skirmish_unit_types,
)

MEMBER = "s1-99-1.mdb"
TERRAIN_SOURCE = "s3-1-1.mdb"
COUNTER = 55
SIZE = 96
TITLE = "Testing Ground"
BRIEFING = ("A proving ground for scripted tests. Player 1 fields three of every unit in the "
            "west; the AI holds one of every alien unit and building in the east. The middle "
            "has roads, bridges, minefields, walls and wreckage.##Objective: ( 100 turns )##"
            "Destroy all enemy units.")
# 0 sorts it first: the original lists skirmishes ORDER BY [One Off Deployment].
POINTS = 0
GRASS = "BORE000"

PLAYER, AI = 1, 0
# Player 1 west of PLAYER_MAX_X, AI east of AI_MIN_X: at least 20 tiles apart.
PLAYER_MAX_X, AI_MIN_X = 33, 56
WATER_MOVES = (4, 6, 8)

# Built bridges: (start, end) of terrain-generated spans, plus land approaches.
BUILT_BRIDGES = [((35, 33), (35, 41)), ((67, 46), (67, 51))]
UNBUILT_BRIDGES = [((19, 22), (24, 22)), ((58, 61), (64, 61))]


def jet(database: Path, statements: list[str]):
    """Runs SQL statements against a Jet database with the 32-bit provider."""
    lines = ["$ErrorActionPreference = 'Stop'",
             "$c = New-Object -ComObject ADODB.Connection",
             f"$c.Open('Provider=Microsoft.Jet.OLEDB.4.0;Data Source={database}')"]
    lines += ["[void]$c.Execute(\"" + sql.replace('"', '`"') + "\")" for sql in statements]
    lines.append("$c.Close()")
    with tempfile.TemporaryDirectory(prefix="libday-jet-") as work:
        script = Path(work) / "jet.ps1"
        script.write_text("\n".join(lines), encoding="utf-8")
        shell = Path(os.environ["SystemRoot"]) / "SysWOW64" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
        subprocess.run([str(shell), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)],
                       check=True)


def sql_text(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def source_terrain(archive: Path) -> list[tuple[int, int, int, str]]:
    with tempfile.TemporaryDirectory(prefix="libday-terrain-") as folder:
        work = Path(folder)
        mdb = work / "terrain.mdb"
        extract_map(archive, TERRAIN_SOURCE, mdb)
        run_map_tool(HERE / "map_tool.ps1", Action="Export", Source=mdb, Output=work / "terrain.csv")
        with (work / "terrain.csv").open(newline="", encoding="utf-8-sig") as stream:
            rows = [(int(r["PosX"]), int(r["PosY"]), int(r["Id"]), r["TerrainImg"]) for r in csv.DictReader(stream)]
    if len(rows) != SIZE * SIZE:
        raise ValueError(f"{TERRAIN_SOURCE} is not {SIZE}x{SIZE}.")
    return rows


class Builder:
    def __init__(self, document: Document):
        self.doc = document
        self.m = document.model
        self.log: list[str] = []

    def terrain(self, key):
        return self.m.terrains[self.m.cells[key].terrain.upper()]

    def inside(self, key):
        return 0 <= key[0] < SIZE and 0 <= key[1] < SIZE

    def clear(self, key) -> bool:
        """Removes a tree (inside a stroke); True when the tile is clear land."""
        if not self.inside(key):
            return False
        if self.terrain(key).tree:
            self.m.paint(*key, GRASS, "erase")
        return self.terrain(key).paintable

    # Reset.

    def reset(self):
        m = self.m
        for unit in list(m.units.values()):
            if unit.layer == "units" and unit.order_id:
                m.set_unit_order(unit.identifier, 0, unit.order_turn, unit.probability, unit.enemy_type, (), ())
        m.set_objective(6, m.scripts.objectives[0].turn, (), ())
        for identifier in list(m.units):
            m.delete_unit(identifier)
        for identifier in list(m.buildings):
            m.delete_building(identifier)
        m.begin_stroke()
        for layer, items in m.surfaces.items():
            for item in list(items.values()):
                m.paint_surface(layer, item.x, item.y, item.kind, erase=True)
        for item in list(m.deployments.values()):
            m.paint_deployment(item.x, item.y, item.player, erase=True)
        m.end_stroke()
        if m.deployments:
            raise ValueError("Preserved deployment records remain.")

    # Surfaces.

    def road(self, kind: str, points: list[tuple[int, int]]):
        """Cardinal polyline of road; trees on the way are cleared."""
        self.m.begin_stroke()
        for a, b in zip(points, points[1:]):
            dx = (b[0] > a[0]) - (b[0] < a[0])
            dy = (b[1] > a[1]) - (b[1] < a[1])
            key = a
            while True:
                water = self.terrain(key).kind in (1, 2) and not self.terrain(key).overlapping
                if (water and kind == "1") or self.clear(key):
                    self.m.paint_surface("roads", *key, kind)
                else:
                    self.log.append(f"road skipped {key}")
                if key == b:
                    break
                key = key[0] + dx, key[1] + dy
        self.m.end_stroke()

    def bridges(self):
        spans = {(s.start, s.end) for s in self.m.bridge_spans()}
        for span in BUILT_BRIDGES + UNBUILT_BRIDGES:
            if span not in spans:
                raise ValueError(f"Bridge span {span} is not generated by the terrain.")
        for (sx, sy), (ex, ey) in BUILT_BRIDGES:
            dx, dy = (ex > sx, ey > sy)
            self.road("1", [(sx - 3 * dx, sy - 3 * dy), (ex + 3 * dx, ey + 3 * dy)])
        # Dirt approaches to the unbuilt spans, ending one tile short of the banks.
        for (sx, sy), (ex, ey) in UNBUILT_BRIDGES:
            dx, dy = (ex > sx, ey > sy)
            self.road("0", [(sx - 4 * dx, sy - 4 * dy), (sx - dx, sy - dy)])
            self.road("0", [(ex + dx, ey + dy), (ex + 4 * dx, ey + 4 * dy)])

    def roads(self):
        # Paved trunk road west-east, joining both built bridges.
        self.road("1", [(4, 43), (67, 43), (67, 45)])
        self.road("1", [(35, 26), (35, 32)])
        self.road("1", [(67, 52), (67, 55)])
        # Indestructible dirt roads.
        self.road("0", [(39, 16), (80, 16)])
        self.road("0", [(21, 13), (39, 13), (39, 15)])
        self.road("0", [(10, 60), (30, 60), (30, 75), (44, 75), (44, 92)])

    def mines(self):
        hidden_from_humans = dict(armed=True, hidden_AI=False, hidden_1=True, hidden_2=True,
                                  hidden_3=True, hidden_4=True)
        player_owned = dict(armed=True, hidden_AI=True, hidden_1=False, hidden_2=True,
                            hidden_3=True, hidden_4=True)
        fields = [
            ("0", AI, hidden_from_humans, range(37, 43), range(64, 69)),   # normal, hidden
            ("1", AI, hidden_from_humans, range(37, 43), range(80, 85)),   # grav, hidden
            ("0", PLAYER, player_owned, range(37, 41), range(88, 92)),     # normal, player 1
            ("1", PLAYER, player_owned, range(41, 43), range(88, 92)),     # grav, player 1
            ("0", AI, hidden_from_humans, range(46, 49, 2), range(43, 44)),  # on the paved road
            ("0", AI, hidden_from_humans, range(48, 51), range(18, 21)),   # beside the dirt road
        ]
        self.m.begin_stroke()
        for kind, owner, flags, xs, ys in fields:
            for y in ys:
                for x in xs:
                    if self.clear((x, y)):
                        self.m.paint_surface("mines", x, y, kind, owner=owner, mine_flags=flags)
        self.m.end_stroke()

    def walls(self):
        segments = [("0", [(46, 86), (52, 86)]), ("1", [(46, 88), (52, 88)]),
                    ("2", [(46, 90), (52, 90)]), ("3", [(46, 92), (52, 92)]),
                    ("0", [(56, 20), (56, 26)])]
        self.m.begin_stroke()
        for kind, (a, b) in segments:
            for x in range(a[0], b[0] + 1):
                for y in range(a[1], b[1] + 1):
                    if self.clear((x, y)):
                        self.m.paint_surface("walls", x, y, kind, owner=AI)
        self.m.end_stroke()

    def props(self):
        kinds = sorted(self.m.surface_types["decors"])
        groups = {}
        for name in kinds:
            groups.setdefault(name[:2], []).append(name)
        # Up to three of each artwork family, a tile apart, in the north-west woods.
        picks = [name for names in groups.values() for name in names[:: max(1, len(names) // 3)][:3]]
        spots = [(x, y) for y in range(1, 10, 2) for x in range(20, 55, 2)]
        self.m.begin_stroke()
        for name, key in zip(picks, spots):
            if key not in self.m.blocked and self.clear(key):
                self.m.paint_surface("decors", *key, name)
        scenery = [t.name for t in self.m.terrains.values()
                   if t.scenery and t.name.startswith("BORE")][:8]
        for i, name in enumerate(scenery):
            key = 22 + 3 * i, 11
            if key not in self.m.blocked and self.clear(key):
                self.m.paint(*key, name, "scenery")
        # A small fresh forest patch.
        trees = [t.name for t in self.m.terrains.values() if t.tree and t.name.startswith("BORE1")][:6]
        for i, (x, y) in enumerate((x, y) for y in range(33, 38) for x in range(46, 52)):
            if self.terrain((x, y)).paintable and (x, y) not in self.m.blocked:
                self.m.paint(x, y, trees[i % len(trees)], "trees")
        self.m.end_stroke()

    def deployment(self):
        self.m.begin_stroke()
        for y in range(45, 53):
            for x in range(11, 23):
                if self.terrain((x, y)).paintable:
                    self.m.paint_deployment(x, y, PLAYER)
        self.m.end_stroke()

    # Buildings.

    def buildings(self):
        taken: set[tuple[int, int]] = set()
        types = sorted(self.m.building_types.values(), key=lambda t: -t.width * t.height)
        land = [(x, y) for y in range(0, 42) for x in range(AI_MIN_X, SIZE)]
        sea = [(x, y) for y in range(52, SIZE) for x in range(72, SIZE)]
        for kind in types:
            water = kind.position == 1
            for x, y in sea if water else land:
                cells = [(x + dx, y + dy) for dx in range(kind.width) for dy in range(kind.height)]
                ring = [(x + dx, y + dy) for dx in range(-1, kind.width + 1) for dy in range(-1, kind.height + 1)]
                if any(not self.inside(c) or c in self.m.blocked for c in cells) or taken.intersection(ring):
                    continue
                if water:
                    if any(self.terrain(c).kind != 1 or self.terrain(c).overlapping for c in cells):
                        continue
                elif any(not (self.terrain(c).paintable or self.terrain(c).tree) for c in cells):
                    continue
                if not water:
                    self.m.begin_stroke()
                    for c in cells:
                        self.clear(c)
                    self.m.end_stroke()
                self.m.add_building(kind.identifier, AI, x, y)
                taken.update(cells)
                break
            else:
                raise ValueError(f"No room for building {kind.identifier} {kind.name}")

    # Units.

    def candidates(self, area, water: bool, step: int, centre):
        tiles = [(x, y) for x, y in area if x % step == 0 and y % step == 0]
        tiles = [k for k in tiles if (self.terrain(k).kind == 1) == water]
        return sorted(tiles, key=lambda k: max(abs(k[0] - centre[0]), abs(k[1] - centre[1])))

    def place(self, kinds: list[int], owner: int, land: list, sea: list, facing: int) -> list:
        placed = []
        for kind in kinds:
            definition = self.m.unit_types[kind, 0]
            pool = sea if definition.movement in WATER_MOVES else land
            for key in list(pool):
                try:
                    self.m.add_unit(kind, *key, owner=owner, facing=facing)
                except ValueError:
                    continue
                pool.remove(key)
                placed.append((kind, key))
                break
            else:
                raise ValueError(f"No room for unit type {kind} {definition.name}")
        return placed

    def units(self):
        types = sorted({kind for kind, _ in self.m.unit_types})
        aliens = [k for k in types if self.m.unit_types[k, 0].race != 0]
        ai_land = self.candidates([(x, y) for y in range(0, 42) for x in range(AI_MIN_X, SIZE)], False, 2, (76, 20))
        ai_sea = self.candidates([(x, y) for y in range(52, SIZE) for x in range(72, SIZE)], True, 4, (84, 74))
        ai = self.place(aliens, AI, ai_land, ai_sea, facing=6)
        p_land = self.candidates([(x, y) for y in range(24, SIZE) for x in range(1, PLAYER_MAX_X + 1)],
                                 False, 2, (14, 66))
        p_sea = self.candidates([(x, y) for y in range(0, 42) for x in range(0, PLAYER_MAX_X + 1)],
                                True, 2, (6, 6))
        # Big footprints first, so 3x3 types find space.
        ordered = sorted(types, key=lambda k: -self.m.unit_types[k, 0].width)
        player = self.place([k for k in ordered for _ in range(3)], PLAYER, p_land, p_sea, facing=2)
        gap = min(max(abs(a[0] - b[0]), abs(a[1] - b[1])) for _, a in player for _, b in ai)
        if gap < 20:
            raise ValueError(f"Player and AI units are only {gap} tiles apart.")
        self.log.append(f"{len(player)} player units, {len(ai)} AI units, {gap} tiles apart")

    def settings(self):
        allowed = skirmish_unit_types(self.m.unit_types)
        self.m.update_scenario(ScenarioSettings(TITLE, BRIEFING, 100, POINTS,
                                                tuple((kind, True) for kind in allowed)))


def build(root: Path, work: Path) -> Path:
    scens = root / "SCENS"
    archive = scens / "SCEN1.ZIP.pre-testing-ground"
    if not archive.exists():
        archive = scens / "SCEN1.ZIP"
    terrain = source_terrain(archive)
    terrains = load_terrains(HERE / "terrain-types.csv")
    with tempfile.TemporaryDirectory(prefix="libday-stage-") as folder:
        # The object editor only edits land, so the 96x96 terrain (with its water)
        # is staged into [Map] directly. Obsolete campaign HQ rows would land
        # inside the larger map.
        staged = Path(folder) / MEMBER
        extract_map(archive, MEMBER, staged)
        jet(staged, ["DELETE FROM [Map]", *(
            f"INSERT INTO [Map] ([Id],[PosX],[PosY],[TerrainImg]) VALUES ({i},{x},{y},{sql_text(name)})"
            for (x, y, i, name) in terrain), "DELETE FROM [HQ on Map]",
            *(f"DELETE FROM [{table} On Map]" for table in ("Roads", "Walls", "Mines", "Ruins"))])
        document = Document.open(staged, None, HERE / "map_tool.ps1", HERE / "terrain-types.csv", terrains)
    try:
        b = Builder(document)
        b.reset()
        b.bridges()
        b.roads()
        b.deployment()
        b.mines()
        b.walls()
        b.props()
        b.buildings()
        b.units()
        b.settings()
        output = work / MEMBER
        if output.exists():
            output.unlink()
        document.save_copy(output, b.m.rows())
        print("\n".join(b.log))
    finally:
        document.close()
    # openlibday-only extension: 0 technology points skips the technology screen.
    jet(output, ["ALTER TABLE [Scenario Info] ADD COLUMN [TechPoints] LONG",
                 "UPDATE [Scenario Info] SET [TechPoints] = 0"])
    return output


def install(root: Path, database: Path, work: Path):
    scens = root / "SCENS"
    for name in ("SCEN1.ZIP", "SKIRMISH.MDB"):
        backup = scens / f"{name}.pre-testing-ground"
        if not backup.exists():
            shutil.copyfile(scens / name, backup)
    packed = work / "SCEN1.ZIP"
    if packed.exists():
        packed.unlink()
    pack_map(scens / "SCEN1.ZIP", MEMBER, database, packed)
    shutil.copyfile(packed, scens / "SCEN1.ZIP")
    title = sql_text(TITLE)
    description = sql_text("Three of every unit against every alien unit and building. "
                           "Roads, bridges, minefields and props for scripted tests.")
    jet(scens / "SKIRMISH.MDB", [
        f"UPDATE [Scenario Info] SET [TitleEnglish] = {title}, [TitleGerman] = {title}, "
        f"[TitleFrench] = {title}, [descriptionEnglish] = {description}, "
        f"[One Off Deployment] = {POINTS}, [Size] = {SIZE} WHERE [Counter] = {COUNTER}",
    ])
    cached = root / "CACHE" / "SCEN1" / MEMBER
    if cached.exists():
        cached.unlink()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--root", type=Path, required=True, help="Liberation Day installation folder")
    parser.add_argument("--work", type=Path, default=Path.cwd(), help="Folder for the generated database")
    parser.add_argument("--install", action="store_true", help="Install into the game folder")
    args = parser.parse_args()
    args.work.mkdir(parents=True, exist_ok=True)
    database = build(args.root.resolve(), args.work.resolve())
    print(f"wrote {database}")
    if args.install:
        install(args.root.resolve(), database, args.work.resolve())
        print("installed")


if __name__ == "__main__":
    main()
