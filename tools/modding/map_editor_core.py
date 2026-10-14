"""Shared game artwork, editable map layers, and non-destructive Jet/ZIP operations."""

from __future__ import annotations

import csv
from dataclasses import dataclass, field, replace
import json
import math
import os
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import zipfile
import zlib

from libday_mod import extract_map, member_name, pack_map, require_mdb
from scenario_scripts import ORDERS, Objective, ScriptData, SupportEvent, load_support_types


@dataclass(frozen=True)
class Terrain:
    name: str
    kind: int
    overlapping: bool

    @property
    def paintable(self) -> bool:
        return self.kind == 3 and not self.overlapping

    @property
    def tree(self) -> bool:
        return self.kind == 5

    @property
    def scenery(self) -> bool:
        return self.kind in (3, 4, 6) and not self.paintable


def load_terrains(path: Path) -> dict[str, Terrain]:
    result = {}
    with path.open(newline="", encoding="utf-8-sig") as stream:
        for row in csv.DictReader(stream):
            name = row["id"].upper()
            if not re.fullmatch(r"[A-Za-z0-9_]{1,9}", name) or name in result:
                raise ValueError(f"Invalid or duplicate terrain identifier: {name}")
            if row["Overlapping"] not in ("0", "1"):
                raise ValueError(f"Invalid overlapping flag: {name}")
            result[name] = Terrain(name, int(row["type"]), row["Overlapping"] == "1")
    if not result:
        raise ValueError("The terrain registry is empty.")
    return result


@dataclass(frozen=True)
class BuildingType:
    identifier: int
    name: str
    width: int
    height: int
    position: int
    armor: int
    unique: bool
    image: str
    transforms: bool
    ai_target: bool
    damaged_images: bool

    def sprite(self, dx: int, dy: int, damaged: bool = False, level: int = 0) -> str:
        return f"{self.image}{level}{int(damaged and self.damaged_images and not self.transforms)}{dx}{dy}"


def load_building_types(path: Path) -> dict[int, BuildingType]:
    result = {}
    with path.open(newline="", encoding="utf-8-sig") as stream:
        for row in csv.DictReader(stream):
            item = BuildingType(
                int(row["TypeID"]), row["NameEnglish"], int(row["SizeX"]), int(row["SizeY"]),
                int(row["Position"]), int(row["Armor"]), row["UniqueOnMap"] == "1",
                row["ImageID"].upper(), row["TransformIntoUnit"] != "65535", row["isAItarget"] == "1",
                row["DamagedImages"] == "1",
            )
            if (item.identifier in result or not 1 <= item.width <= 9 or not 1 <= item.height <= 9
                    or item.position not in (1, 3) or item.armor < 1
                    or not re.fullmatch(r"[A-Z0-9]{1,4}", item.image)):
                raise ValueError(f"Invalid or duplicate building definition: {item.identifier}")
            result[item.identifier] = item
    if not result:
        raise ValueError("The building registry is empty.")
    return result


BUILDING_FIELDS = ("Counter", "id", "buildingType", "playerOwner", "currentAR", "PosX", "PosY", "IsAiTarget")

UNIT_FIELDS = {
    "units": ("Counter", "ID", "unitType", "unitTL", "playerOwner", "facing", "currentAR", "currentAP",
              "negAP", "PosX", "PosY", "-------------", "OrderID", "Turn", "Probability", "EnemyUnitType"),
    "reinforcements": ("Counter", "UnitID", "type", "posX", "posY", "turn", "order"),
}

SKIRMISH_DIFFICULTIES = {"Easy": 150, "Average": 400, "Hard": 750, "Very hard": 1500}


def skirmish_difficulty(points: int) -> str:
    return ("Easy" if points <= 300 else "Average" if points <= 500 else
            "Hard" if points <= 1000 else "Very hard")


def skirmish_unit_types(unit_types: dict) -> list[int]:
    return sorted(identifier for identifier, tech in unit_types
                  if tech == 0 and (identifier < 35 or identifier == 49 or 54 <= identifier <= 59))


@dataclass(frozen=True)
class ScenarioSettings:
    title: str
    briefing: str
    turns: int
    points: int
    available: tuple[tuple[int, bool], ...]

    @classmethod
    def from_row(cls, row: dict) -> ScenarioSettings:
        return cls(row["title"], row["briefing"], row["turns"], row["points"],
                   tuple(sorted((item["kind"], item["enabled"]) for item in row["available"])))

    def row(self) -> dict:
        return {"title": self.title, "briefing": self.briefing, "turns": self.turns, "points": self.points,
                "available": [{"kind": kind, "enabled": enabled} for kind, enabled in self.available]}

    def validate(self, unit_types: dict):
        if not isinstance(self.title, str) or not self.title.strip() or len(self.title) > 50:
            raise ValueError("Scenario name must contain 1..50 characters.")
        if not isinstance(self.briefing, str) or "\0" in self.title + self.briefing:
            raise ValueError("Scenario text cannot contain NUL characters.")
        if type(self.turns) is not int or not 1 <= self.turns <= 100:
            raise ValueError("Turn limit must be an integer from 1 to 100.")
        if type(self.points) is not int or not 0 <= self.points <= 32767:
            raise ValueError("Technology/deployment points must be an integer from 0 to 32767.")
        allowed = set(skirmish_unit_types(unit_types))
        seen = set()
        for kind, enabled in self.available:
            if type(kind) is not int or kind not in allowed or kind in seen or type(enabled) is not bool:
                raise ValueError("Available units must be unique native skirmish types with Boolean availability.")
            seen.add(kind)
UNIT_TABLES = {"units": "Units On Map", "reinforcements": "Reinforcement"}


@dataclass(frozen=True)
class UnitType:
    identifier: int
    tech: int
    name: str
    race: int
    facings: bool
    width: int
    height: int
    movement: int
    armor: int
    ap: int
    image: str

    def sprite(self, dx: int = 0, dy: int = 0, facing: int = 0, frame: int = 0) -> str:
        return f"{self.image}{frame}{facing if self.facings else 0}{dx}{dy}"


def load_unit_types(path: Path) -> dict[tuple[int, int], UnitType]:
    result = {}
    with path.open(newline="", encoding="utf-8-sig") as stream:
        for row in csv.DictReader(stream):
            item = UnitType(*(int(row[key]) for key in ("typeID", "unitTechLevel")), row["NameEnglish"],
                            int(row["Race"]), row["Facings"] == "1",
                            *(int(row[key]) for key in ("SizeX", "SizeY", "Movement", "Armor", "actionPoints")),
                            row["facingsImageID"].upper())
            key = item.identifier, item.tech
            if (key in result or not 0 <= item.identifier < 65535 or not 0 <= item.tech <= 10
                    or row["Facings"] not in ("0", "1") or not 1 <= item.width <= 9
                    or not 1 <= item.height <= 9 or not 0 <= item.movement <= 9
                    or item.armor < 1 or item.ap < 0 or not re.fullmatch(r"[A-Z0-9]{1,4}", item.image)):
                raise ValueError(f"Invalid or duplicate unit definition: {key}")
            result[key] = item
    if not result or any((kind, 0) not in result for kind, _ in result):
        raise ValueError("Unit definitions require a base technology level for every type.")
    return result


def unit_terrain_compatible(kind: UnitType, terrain: Terrain) -> bool:
    if kind.movement in (4, 6, 8):
        return terrain.kind == 1 and not terrain.overlapping
    if kind.movement == 5:
        return True
    if kind.movement in (3, 9):
        return terrain.kind in (1, 2, 3) and not terrain.overlapping
    return terrain.paintable or (kind.movement == 2 and terrain.tree)


@dataclass(frozen=True)
class Unit:
    identifier: int
    kind: int
    x: int
    y: int
    arrival: int = 0
    layer: str = "units"
    owner: int = 0
    tech: int = 0
    facing: int = 0
    armor: int = 1
    ap: int = 0
    negative_ap: int = 0
    separator: bool = True
    order_id: int = 0
    order_turn: int = 0
    probability: int = 100
    enemy_type: int = 65535
    arrival_order: bool = False
    counter: int | None = None
    origin_layer: str = ""

    @classmethod
    def from_row(cls, layer: str, row: dict, types: dict[tuple[int, int], UnitType]) -> Unit:
        normalized = {key.lower(): value for key, value in row.items()}

        def flag(key: str) -> bool:
            value = str(normalized[key]).lower()
            if value not in ("true", "false", "0", "1"):
                raise ValueError(f"Invalid unit {key} flag.")
            return value in ("true", "1")

        if layer == "reinforcements":
            kind = int(normalized["type"])
            definition = types[kind, 0]
            return cls(int(normalized["unitid"]), kind, int(normalized["posx"]), int(normalized["posy"]),
                       arrival=int(normalized["turn"]), layer=layer, armor=definition.armor, ap=definition.ap,
                       arrival_order=flag("order"), counter=int(normalized["counter"]), origin_layer=layer)
        return cls(int(normalized["id"]), int(normalized["unittype"]),
                   int(normalized["posx"]), int(normalized["posy"]), owner=int(normalized["playerowner"]),
                   tech=int(normalized["unittl"]), facing=int(normalized["facing"]),
                   armor=int(normalized["currentar"]), ap=int(normalized["currentap"]),
                   negative_ap=int(normalized["negap"]), separator=flag("-------------"),
                   order_id=int(normalized["orderid"]), order_turn=int(normalized["turn"]),
                   probability=int(normalized["probability"]), enemy_type=int(normalized["enemyunittype"]),
                   counter=int(normalized["counter"]), origin_layer=layer)

    def row(self) -> dict[str, str]:
        counter = str(self.counter) if self.counter is not None and self.layer == self.origin_layer else ""
        if self.layer == "reinforcements":
            values = (counter, self.identifier, self.kind, self.x, self.y, self.arrival, int(self.arrival_order))
        else:
            values = (counter, self.identifier, self.kind, self.tech, self.owner, self.facing, self.armor,
                      self.ap, self.negative_ap, self.x, self.y, int(self.separator), self.order_id,
                      self.order_turn, self.probability, self.enemy_type)
        return dict(zip(UNIT_FIELDS[self.layer], map(str, values)))

SURFACE_FIELDS = {
    "roads": ("Counter", "roadtype", "playerOwner", "PosX", "PosY"),
    "walls": ("Counter", "roadtype", "playerOwner", "currentAR", "posX", "posY", "isDamaged"),
    "decors": ("Counter", "id", "PosX", "PosY"),
    "mines": ("Counter", "typeID", "playerOwner", "PosX", "PosY", "armed",
              "hidden_AI", "hidden_1", "hidden_2", "hidden_3", "hidden_4"),
}
SURFACE_TABLES = {"roads": "Roads On Map", "walls": "Walls On Map", "decors": "Ruins On Map",
                  "mines": "Mines On Map"}
MINE_FLAGS = ("armed", "hidden_AI", "hidden_1", "hidden_2", "hidden_3", "hidden_4")
# CRoadChunk / CWallChunk use a permuted lookup, not a sprite number equal to the bitmask.
CONNECTION_SPRITES = (0, 9, 10, 7, 8, 13, 12, 2, 11, 15, 14, 5, 6, 3, 4, 1)
NEIGHBORS = ((-1, 0, 8), (1, 0, 4), (0, -1, 2), (0, 1, 1))


@dataclass(frozen=True)
class SurfaceType:
    identifier: str
    name: str
    image: str
    armor: int = 0
    passable: bool = False


def load_surface_types(directory: Path) -> dict[str, dict[str, SurfaceType]]:
    result = {layer: {} for layer in SURFACE_FIELDS}
    for layer, filename in (("roads", "road-types.csv"), ("walls", "wall-types.csv"),
                            ("decors", "decor-types.csv"), ("mines", "mine-types.csv")):
        with (directory / filename).open(newline="", encoding="utf-8-sig") as stream:
            for row in csv.DictReader(stream):
                if layer == "mines":
                    identifier = row["typeID"]
                    if identifier not in ("0", "1") or row["ImageID"] not in ("MINE", "GMINE"):
                        raise ValueError(f"Invalid mine definition: {identifier}")
                    item = SurfaceType(identifier, row["nameEnglish"], row["ImageID"])
                elif layer == "decors":
                    identifier = row["id"].upper()
                    if not re.fullmatch(r"[BU][A-Z]RUI[0-9X]{3}", identifier):
                        raise ValueError(f"Invalid decor artwork identifier: {identifier}")
                    if row["can_move_through"] not in ("0", "1"):
                        raise ValueError(f"Invalid decor movement flag: {identifier}")
                    category = "Building debris" if identifier.startswith("B") else "Unit wreck"
                    item = SurfaceType(identifier, f"{category} {identifier[1]} {identifier[5:]}",
                                       identifier, passable=row["can_move_through"] == "1")
                else:
                    identifier = row["RoadType" if layer == "roads" else "WallType"]
                    item = SurfaceType(identifier, row["nameEnglish"], row["img_id"].upper(),
                                       int(row["ar" if layer == "roads" else "Armor"]))
                if (identifier in result[layer] or not re.fullmatch(r"[A-Z0-9_]{1,8}", item.image)
                        or (layer in ("roads", "walls") and (not identifier.isdigit() or item.armor < 1))):
                    raise ValueError(f"Invalid or duplicate {layer} definition: {identifier}")
                result[layer][identifier] = item
        if not result[layer]:
            raise ValueError(f"The {layer} registry is empty.")
    return result


@dataclass(frozen=True)
class Surface:
    counter: int
    kind: str
    x: int
    y: int
    owner: int = 0
    armor: int = 0
    broken: bool = False
    armed: bool = True
    hidden_AI: bool = False
    hidden_1: bool = True
    hidden_2: bool = True
    hidden_3: bool = True
    hidden_4: bool = True

    @classmethod
    def from_row(cls, layer: str, row: dict) -> Surface:
        normalized = {key.lower(): value for key, value in row.items()}
        flag = str(normalized.get("isdamaged", "false")).lower()
        if flag not in ("true", "false", "0", "1"):
            raise ValueError("Invalid wall isDamaged flag.")
        flags = {}
        if layer == "mines":
            for name in MINE_FLAGS:
                value = str(normalized[name.lower()]).lower()
                if value not in ("true", "false", "0", "1"):
                    raise ValueError(f"Invalid mine {name} flag.")
                flags[name] = value in ("true", "1")
        kind_field = "typeid" if layer == "mines" else "id" if layer == "decors" else "roadtype"
        return cls(int(normalized["counter"]), str(normalized[kind_field]),
                   int(normalized["posx"]), int(normalized["posy"]), int(normalized.get("playerowner", 0)),
                   int(normalized.get("currentar", 0)), flag in ("true", "1"), **flags)

    def row(self, layer: str) -> dict[str, str]:
        values = {"counter": "" if self.counter < 0 else str(self.counter),
                  "id": self.kind, "roadtype": self.kind, "playerowner": str(self.owner),
                  "currentar": str(self.armor), "posx": str(self.x), "posy": str(self.y),
                  "isdamaged": str(int(self.broken))}
        values["typeid"] = self.kind
        values.update({name.lower(): str(int(getattr(self, name))) for name in MINE_FLAGS})
        return {key: values[key.lower()] for key in SURFACE_FIELDS[layer]}


@dataclass(frozen=True)
class BridgeSpan:
    start: tuple[int, int]
    end: tuple[int, int]

    @property
    def tiles(self) -> tuple[tuple[int, int], ...]:
        dx, dy = int(self.end[0] != self.start[0]), int(self.end[1] != self.start[1])
        length = self.end[0] - self.start[0] + self.end[1] - self.start[1]
        return tuple((self.start[0] + dx * step, self.start[1] + dy * step) for step in range(length + 1))


@dataclass(frozen=True)
class Building:
    identifier: int
    kind: int
    owner: int
    armor: int
    x: int
    y: int
    ai_target: bool
    counter: int | None = None

    @classmethod
    def from_row(cls, row: dict) -> Building:
        flag = str(row["IsAiTarget"]).lower()
        if flag not in ("true", "false", "0", "1"):
            raise ValueError("Invalid building AI-target flag.")
        return cls(*(int(row[key]) for key in ("id", "buildingType", "playerOwner", "currentAR", "PosX", "PosY")),
                   flag in ("true", "1"), int(row["Counter"]) if row["Counter"] not in ("", None) else None)

    def row(self) -> dict[str, str]:
        return dict(zip(BUILDING_FIELDS, (
            "" if self.counter is None else str(self.counter), str(self.identifier), str(self.kind),
            str(self.owner), str(self.armor), str(self.x), str(self.y), str(int(self.ai_target)),
        )))


DEPLOYMENT_FIELDS = ("Counter", "player", "PosX", "PosY", "Turn")
# CMapZone::sprite (0x42690D) uses a different connection lookup from roads/walls.
DEPLOYMENT_SPRITES = (0, 13, 15, 14, 10, 1, 7, 4, 12, 3, 9, 6, 11, 2, 8, 5)


@dataclass(frozen=True)
class Deployment:
    counter: int
    player: int
    x: int
    y: int
    turn: int = 0

    @property
    def editable(self) -> bool:
        return self.player in range(1, 5) and self.turn == 0

    @classmethod
    def from_row(cls, row: dict) -> Deployment:
        return cls(*(int(row[name]) for name in DEPLOYMENT_FIELDS))

    def row(self) -> dict[str, int]:
        return dict(zip(DEPLOYMENT_FIELDS, (self.counter, self.player, self.x, self.y, self.turn)))


@dataclass
class Edit:
    cells: dict[tuple[int, int], tuple[str, str]] = field(default_factory=dict)
    buildings: dict[int, tuple[Building | None, Building | None]] = field(default_factory=dict)
    next_id: tuple[int, int] | None = None
    surfaces: dict[tuple[str, int], tuple[Surface | None, Surface | None]] = field(default_factory=dict)
    units: dict[int, tuple[Unit | None, Unit | None]] = field(default_factory=dict)
    next_unit_id: tuple[int, int] | None = None
    scenario: tuple[ScenarioSettings, ScenarioSettings] | None = None
    deployments: dict[int, tuple[Deployment | None, Deployment | None]] = field(default_factory=dict)
    scripts: tuple[ScriptData, ScriptData] | None = None
    grid: tuple[tuple[tuple[int, int, int, str], ...], tuple[tuple[int, int, int, str], ...]] | None = None


@dataclass(frozen=True)
class Texture:
    width: int
    height: int
    rgba: bytes

    def png(self) -> bytes:
        def chunk(kind: bytes, contents: bytes) -> bytes:
            return (struct.pack(">I", len(contents)) + kind + contents
                    + struct.pack(">I", zlib.crc32(kind + contents) & 0xFFFFFFFF))

        rows = b"".join(b"\0" + self.rgba[y * self.width * 4:(y + 1) * self.width * 4]
                        for y in range(self.height))
        header = struct.pack(">IIBBBBB", self.width, self.height, 8, 6, 0, 0, 0)
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header)
                + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


class TerrainArt:
    HEADER = 32
    PALETTE = 1024
    RECORD = 8192
    INDEX = 12

    def __init__(self, path: Path):
        self.data = path.read_bytes()
        if len(self.data) < self.HEADER + self.PALETTE:
            raise ValueError("The terrain artwork database is truncated.")
        magic, version, count, width, height, palette_count = struct.unpack_from("<4sIIIII", self.data)
        if (magic, version, width, height, palette_count) != (b"dbi\0", 3, 54, 56, 1):
            raise ValueError("Unsupported terrain DBI format (expected v3, 54x56, one palette).")
        if not 1 <= count <= 10000:
            raise ValueError("Invalid terrain artwork record count.")
        start = self.HEADER + self.PALETTE
        index_start = start + count * self.RECORD
        if len(self.data) != index_start + count * self.INDEX:
            raise ValueError("Terrain artwork size does not match its fixed-record index.")
        self.palette = [
            bytes((self.data[self.HEADER + i * 4 + 2],
                   self.data[self.HEADER + i * 4 + 1],
                   self.data[self.HEADER + i * 4], 255))
            for i in range(256)
        ]
        self.records = {}
        for index in range(count):
            pos = start + index * self.RECORD
            name = self._name(self.data[pos:pos + 8])
            if name in self.records:
                raise ValueError(f"Duplicate terrain artwork record: {name}")
            self.records[name] = pos
        seen = set()
        for index in range(count):
            pos = index_start + index * self.INDEX
            name = self._name(self.data[pos:pos + 8])
            record = struct.unpack_from("<I", self.data, pos + 8)[0]
            if record >= count or name in seen or self.records.get(name) != start + record * self.RECORD:
                raise ValueError(f"Invalid terrain artwork index: {name}")
            seen.add(name)
        self.cache: dict[str, Texture] = {}

    @staticmethod
    def _name(raw: bytes) -> str:
        name = raw.split(b"\0", 1)[0].decode("ascii")
        if not re.fullmatch(r"[A-Za-z0-9_]{1,8}", name):
            raise ValueError(f"Invalid terrain artwork name: {name!r}")
        return name

    def texture(self, name: str) -> Texture:
        name = name.upper()
        if name in self.cache:
            return self.cache[name]
        if name not in self.records:
            raise ValueError(f"No artwork for terrain {name}.")
        record = self.records[name]
        width, height, size, encoding = struct.unpack_from("<HHHH", self.data, record + 8)
        if (width, height, encoding) != (54, 56, 1) or not 1 <= size <= self.RECORD - 20:
            raise ValueError(f"Invalid terrain artwork header: {name}")
        pos = record + 20
        limit = pos + size
        pixels = bytearray(width * height * 4)
        for y in range(height):
            if pos + 4 > limit:
                raise ValueError(f"Missing texture scanline: {name}, row {y}")
            length = struct.unpack_from("<I", self.data, pos)[0]
            end = pos + length
            pos += 4
            if length < 8 or end > limit + 3:
                raise ValueError(f"Invalid texture scanline length: {name}, row {y}")
            x = 0
            while pos < end:
                # DBI omits the last three bytes of a final transparent run.
                if pos + 4 > limit:
                    if not (y == height - 1 and end == limit + 3 and limit - pos == 1
                            and self.data[pos] == width - x):
                        raise ValueError(f"Truncated texture run: {name}, row {y}")
                    x, pos = width, end
                    break
                count, kind = struct.unpack_from("<HH", self.data, pos)
                pos += 4
                if count == 0 or x + count > width or kind not in (1, 2):
                    raise ValueError(f"Invalid texture run: {name}, row {y}")
                if kind == 2:
                    if pos + count > min(end, limit):
                        raise ValueError(f"Truncated texture pixels: {name}, row {y}")
                    colors = b"".join(self.palette[index] for index in self.data[pos:pos + count])
                    offset = (y * width + x) * 4
                    pixels[offset:offset + count * 4] = colors
                    pos += count
                x += count
            if pos != end or x != width:
                raise ValueError(f"Texture scanline does not cover its width: {name}, row {y}")
        if pos not in (limit - 1, limit + 3):
            raise ValueError(f"Unexpected texture payload length: {name}")
        result = Texture(width, height, bytes(pixels))
        self.cache[name] = result
        return result


@dataclass
class Cell:
    identifier: int
    x: int
    y: int
    terrain: str


class MapModel:
    def __init__(self, rows: list[dict[str, str]], terrains: dict[str, Terrain],
                 building_types: dict[int, BuildingType] | None = None, objects: dict | None = None,
                 surface_types: dict[str, dict[str, SurfaceType]] | None = None,
                 unit_types: dict[tuple[int, int], UnitType] | None = None):
        self.size = math.isqrt(len(rows))
        if not rows or self.size * self.size != len(rows) or self.size > 256:
            raise ValueError("The scenario must contain a complete square map, maximum 256x256.")
        self.terrains = terrains
        self.cells: dict[tuple[int, int], Cell] = {}
        identifiers = set()
        for row in rows:
            values = [row[field] for field in ("Id", "PosX", "PosY")]
            if any(not re.fullmatch(r"[0-9]+", value) for value in values):
                raise ValueError("Map identifiers and coordinates must be nonnegative integers.")
            identifier, x, y = map(int, values)
            name = row["TerrainImg"]
            if identifier > 2147483647 or identifier in identifiers:
                raise ValueError(f"Duplicate or invalid map identifier: {identifier}")
            if not (0 <= x < self.size and 0 <= y < self.size) or (x, y) in self.cells:
                raise ValueError(f"Duplicate or out-of-bounds map cell: {x},{y}")
            if name.upper() not in terrains:
                raise ValueError(f"Unknown terrain in scenario: {name}")
            identifiers.add(identifier)
            self.cells[x, y] = Cell(identifier, x, y, name)
        self.building_types = building_types or {}
        objects = objects or {}
        self.original_cell_ids = {key: cell.identifier for key, cell in self.cells.items()}
        self.original_cell_floor = max(identifiers) + 1
        self.resize_reservations = objects.get("resizeReservations", [])
        self.buildings: dict[int, Building] = {}
        for row in objects.get("buildings", []):
            building = Building.from_row(row)
            if building.identifier in self.buildings or building.kind not in self.building_types:
                raise ValueError(f"Duplicate building ID or unknown building type: {building.identifier}")
            self.buildings[building.identifier] = building
        self.references = set(map(int, objects.get("references", [])))
        self.scripts = ScriptData.from_rows(objects["scripts"]) if "scripts" in objects else None
        self.support_types = load_support_types(Path(__file__).with_name("support-types.csv"))
        self.fixed_references = set(map(int, objects.get("fixedReferences", []))) \
            if self.scripts is not None else set(self.references)
        self.reserved_ids = set(map(int, objects.get("reservedIds", []))) | self.references
        self.unit_types = unit_types if unit_types is not None else load_unit_types(Path(__file__).with_name("unit-types.csv"))
        self.units: dict[int, Unit] = {}
        self.unit_positions: dict[tuple[int, int], set[int]] = {}
        for layer in UNIT_FIELDS:
            for row in objects.get(layer, []):
                unit = Unit.from_row(layer, row, self.unit_types)
                if (unit.identifier in self.units or not 0 <= unit.identifier < 1000000
                        or (unit.kind, unit.tech) not in self.unit_types
                        or (layer == "reinforcements" and not 0 <= unit.arrival <= 100)):
                    raise ValueError(f"Invalid/duplicate unit ID, type or arrival turn: {unit.identifier}")
                self._set_unit(unit.identifier, unit)
        self.unit_references = set(map(int, objects.get("unitReferences", [])))
        self.fixed_unit_references = set(self.fixed_references) if self.scripts is not None \
            else set(self.unit_references)
        self._refresh_script_references()
        self.reserved_unit_ids = {identifier for identifier in self.reserved_ids if identifier < 1000000}
        self.next_unit_id = max(int(objects.get("nextUnitId", 0)), 0,
                                max(self.units.keys() | self.reserved_unit_ids, default=-1) + 1)
        self.deployments: dict[int, Deployment] = {}
        self.deployment_positions: dict[tuple[int, int], set[int]] = {}
        for row in objects.get("deployments", []):
            item = Deployment.from_row(row)
            if item.counter < 0 or item.counter in self.deployments:
                raise ValueError(f"Duplicate or invalid deployment counter: {item.counter}")
            self._set_deployment(item.counter, item)
        self.next_deployment_counter = -1
        self.static_blocked: dict[tuple[int, int], str] = {}
        for row in objects.get("blocked", []):
            if (row["reason"] == "Deployment" and "deployments" in objects
                    or row["reason"] == "Automated Units" and self.scripts is not None):
                continue
            if any(row["reason"] == table and layer in objects for layer, table in SURFACE_TABLES.items()):
                continue
            if any(row["reason"] == table and layer in objects for layer, table in UNIT_TABLES.items()):
                continue
            key = int(row["x"]), int(row["y"])
            if key in self.cells:
                self.static_blocked[key] = row["reason"]
        self.surface_types = surface_types if surface_types is not None else load_surface_types(Path(__file__).parent)
        self.surfaces: dict[str, dict[int, Surface]] = {layer: {} for layer in SURFACE_FIELDS}
        self.surface_positions: dict[str, dict[tuple[int, int], set[int]]] = {
            layer: {} for layer in SURFACE_FIELDS
        }
        for layer in self.surfaces:
            for row in objects.get(layer, []):
                item = Surface.from_row(layer, row)
                if item.counter < 0 or item.counter in self.surfaces[layer]:
                    raise ValueError(f"Duplicate or invalid {layer} counter: {item.counter}")
                if item.kind.upper() not in self.surface_types[layer]:
                    raise ValueError(f"Unknown {layer} type: {item.kind}")
                self._set_surface(layer, item.counter, item)
        self.next_surface_counter = -1
        self.next_building_id = max(int(objects.get("nextBuildingId", 1000000)), 1000000,
                                    max(self.buildings.keys() | self.reserved_ids, default=999999) + 1)
        self.scenario = ScenarioSettings.from_row(objects["scenario"]) if "scenario" in objects else None
        self.saved_state = self.state()
        self.undo_stack: list[Edit] = []
        self.redo_stack: list[Edit] = []
        self.stroke: dict[tuple[int, int], tuple[str, str]] | None = None
        self.surface_stroke: dict[tuple[str, int], tuple[Surface | None, Surface | None]] = {}
        self.deployment_stroke: dict[int, tuple[Deployment | None, Deployment | None]] = {}

    def state(self) -> tuple:
        return (tuple((self.cells[x, y].identifier, self.cells[x, y].terrain)
                      for y in range(self.size) for x in range(self.size)),
                tuple(self.buildings[key] for key in sorted(self.buildings)), self.next_building_id,
                tuple(tuple(items[key] for key in sorted(items)) for items in self.surfaces.values()),
                tuple(self.units[key] for key in sorted(self.units)), self.next_unit_id, self.scenario,
                tuple(self.deployments[key] for key in sorted(self.deployments)), self.scripts)

    def _refresh_script_references(self):
        if self.scripts is not None:
            self.references = self.fixed_references | self.scripts.references
            self.unit_references = self.fixed_unit_references | self.scripts.references

    def _script_edit(self, scripts: ScriptData, units: dict | None = None):
        self.end_stroke()
        if self.scripts is None:
            raise ValueError("This document has no native script snapshot.")
        scripts.validate_change(self.scripts, self.size, self.units.keys() | self.buildings.keys(),
                                {identifier for identifier, unit in self.units.items() if unit.layer == "units"},
                                self.support_types)
        changes = units or {}
        if scripts == self.scripts and not changes:
            return
        edit = Edit(scripts=(self.scripts, scripts), units=changes)
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()

    def add_support(self, kind: int, x: int, y: int, turn: int,
                    dest_x: int | None = None, dest_y: int | None = None) -> SupportEvent:
        if self.scripts is None:
            raise ValueError("This document has no native script snapshot.")
        item = SupportEvent(self.scripts.next_counter("events"), kind, x, y,
                            x if dest_x is None else dest_x, y if dest_y is None else dest_y, turn)
        self._script_edit(replace(self.scripts, events=(*self.scripts.events, item)))
        return item

    def update_support(self, counter: int, **properties) -> SupportEvent:
        if self.scripts is None or set(properties) - {"kind", "x", "y", "dest_x", "dest_y", "turn"}:
            raise ValueError("Support counters cannot be edited.")
        old = next((item for item in self.scripts.events if item.counter == counter), None)
        if old is None:
            raise ValueError("Choose an existing support event.")
        item = replace(old, **properties)
        self._script_edit(replace(self.scripts, events=tuple(
            item if event.counter == counter else event for event in self.scripts.events)))
        return item

    def delete_support(self, counter: int):
        if self.scripts is None or not any(item.counter == counter for item in self.scripts.events):
            raise ValueError("Choose an existing support event.")
        self._script_edit(replace(self.scripts, events=tuple(
            event for event in self.scripts.events if event.counter != counter)))

    def set_unit_order(self, identifier: int, order_id: int, turn: int, probability: int, enemy_type: int,
                       ids: tuple[int, ...], positions: tuple[tuple[int, int], ...]):
        unit = self.units.get(identifier)
        if self.scripts is None or unit is None or unit.layer != "units" or unit.order_id not in ORDERS:
            raise ValueError("Only verified starting-unit AI orders can be edited.")
        if (type(order_id) is not int or order_id not in ORDERS or type(turn) is not int
                or not 0 <= turn <= 2147483647 or type(probability) is not int or not 0 <= probability <= 100
                or type(enemy_type) is not int or enemy_type != 65535 and (enemy_type, 0) not in self.unit_types):
            raise ValueError("Choose order 0..6, a nonnegative 32-bit expiry, retention 0..100 and a known enemy type.")
        old_ids = tuple(row.identifier for row in self.scripts.order_ids if row.unit == identifier)
        old_positions = tuple((row.x, row.y) for row in self.scripts.order_positions if row.unit == identifier)
        if (ids != old_ids and order_id not in (3, 4, 6) and ids
                or positions != old_positions and order_id not in (1, 2, 5) and positions):
            raise ValueError("Choose ID targets for Escort/Defend/Attack or tile targets for Goto/Goto Fast/Patrol.")
        scripts = self.scripts.targets("order_ids", ids, identifier).targets("order_positions", positions, identifier)
        updated = replace(unit, order_id=order_id, order_turn=turn, probability=probability, enemy_type=enemy_type)
        self._script_edit(scripts, {identifier: (unit, updated)} if updated != unit else {})

    def set_objective(self, kind: int, turn: int, ids: tuple[int, ...], positions: tuple[tuple[int, int], ...]):
        if self.scripts is None or len(self.scripts.objectives) != 1:
            raise ValueError("Only one existing native objective can be edited.")
        old = self.scripts.objectives[0]
        if kind not in range(7) or old.kind not in range(7):
            raise ValueError("Deathmatch/internal objectives are preserve-only.")
        old_ids = tuple(row.identifier for row in self.scripts.objective_ids)
        old_positions = tuple((row.x, row.y) for row in self.scripts.objective_positions)
        if (ids != old_ids and kind not in (0, 2, 4, 5, 6) and ids
                or positions != old_positions and kind not in (0, 1, 2, 3, 4) and positions):
            raise ValueError("This objective kind does not use the edited target collection.")
        scripts = replace(self.scripts, objectives=(Objective(old.counter, kind, turn),))
        self._script_edit(scripts.targets("objective_ids", ids).targets("objective_positions", positions))

    def _set_deployment(self, counter: int, item: Deployment | None):
        old = self.deployments.get(counter)
        if old:
            key = old.x, old.y
            self.deployment_positions[key].remove(counter)
            if not self.deployment_positions[key]:
                del self.deployment_positions[key]
        if item is None:
            self.deployments.pop(counter, None)
        else:
            self.deployments[counter] = item
            self.deployment_positions.setdefault((item.x, item.y), set()).add(counter)

    def deployment_at(self, key: tuple[int, int]) -> Deployment | None:
        items = (self.deployments[counter] for counter in self.deployment_positions.get(key, ()))
        return min((item for item in items if item.editable),
                   key=lambda item: (item.player, item.counter), default=None)

    def deployment_rows(self) -> list[dict[str, int]]:
        return [self.deployments[counter].row() for counter in sorted(self.deployments)]

    def deployment_sprite(self, item: Deployment) -> str:
        mask = sum(bit for dx, dy, bit in NEIGHBORS
                   if any(self.deployments[counter].editable and self.deployments[counter].player == item.player
                          for counter in self.deployment_positions.get((item.x + dx, item.y + dy), ())))
        return f"DEPL{item.player}{DEPLOYMENT_SPRITES[mask]:02d}"

    def paint_deployment(self, x: int, y: int, player: int, erase: bool = False) -> bool:
        if self.stroke is None:
            raise ValueError("Painting requires an active stroke.")
        if type(player) is not int or player not in range(1, 5) or (x, y) not in self.cells:
            raise ValueError("Deployment painting requires player 1..4 and a position inside the map.")
        items = sorted((self.deployments[counter]
                        for counter in self.deployment_positions.get((x, y), ())),
                       key=lambda item: (item.player != player, item.counter))
        if erase:
            changes = [(item, None) for item in items if item.editable and item.player == player]
        else:
            if any(not item.editable for item in items):
                raise ValueError("This tile has a preserved AI/unknown-player/nonzero-turn deployment record.")
            old = items[0] if items else None
            item = Deployment(old.counter if old else self.next_deployment_counter, player, x, y)
            changes = [(old, item), *((other, None) for other in items[1:])]
        changed = False
        for old, item in changes:
            if old == item:
                continue
            counter = (item or old).counter
            first = self.deployment_stroke.get(counter, (old, item))[0]
            self._set_deployment(counter, item)
            if first == item:
                self.deployment_stroke.pop(counter, None)
            else:
                self.deployment_stroke[counter] = first, item
            if old is None:
                self.next_deployment_counter -= 1
            changed = True
        return changed

    def update_scenario(self, settings: ScenarioSettings):
        self.end_stroke()
        if self.scenario is None:
            raise ValueError("This document has no scenario settings.")
        settings.validate(self.unit_types)
        if not {kind for kind, _ in self.scenario.available} <= {kind for kind, _ in settings.available}:
            raise ValueError("Existing roster rows must be retained; disable units rather than deleting rows.")
        if settings != self.scenario:
            edit = Edit(scenario=(self.scenario, settings))
            self._apply_edit(edit, 1)
            self.undo_stack.append(edit)
            self.redo_stack.clear()

    @property
    def blocked(self) -> dict[tuple[int, int], str]:
        result = dict(self.static_blocked)
        for key in self.deployment_positions.keys() & self.cells.keys():
            result[key] = "Deployment"
        for layer, items in self.surfaces.items():
            for item in items.values():
                if (item.x, item.y) in self.cells:
                    result[item.x, item.y] = SURFACE_TABLES[layer]
        for key in self.unit_positions.keys() & self.cells.keys():
            result[key] = "Unit / reinforcement"
        return result

    def _set_unit(self, identifier: int, unit: Unit | None):
        old = self.units.get(identifier)
        if old:
            for key in self.unit_footprint(old):
                self.unit_positions[key].remove(identifier)
                if not self.unit_positions[key]:
                    del self.unit_positions[key]
        if unit is None:
            self.units.pop(identifier, None)
        else:
            self.units[identifier] = unit
            for key in self.unit_footprint(unit):
                self.unit_positions.setdefault(key, set()).add(identifier)

    def unit_footprint(self, unit: Unit) -> set[tuple[int, int]]:
        kind = self.unit_types[unit.kind, unit.tech if unit.layer == "units" else 0]
        return {(unit.x + dx, unit.y + dy) for dx in range(kind.width) for dy in range(kind.height)}

    def units_at(self, key: tuple[int, int], arrival: int) -> list[Unit]:
        return [self.units[identifier] for identifier in sorted(self.unit_positions.get(key, ()))
                if self.units[identifier].arrival == arrival]

    def unit_rows(self) -> dict[str, list[dict[str, str]]]:
        return {layer: [self.units[key].row() for key in sorted(self.units) if self.units[key].layer == layer]
                for layer in UNIT_FIELDS}

    def validate_unit(self, unit: Unit, old: Unit | None = None):
        key = unit.kind, unit.tech if unit.layer == "units" else 0
        if key not in self.unit_types or unit.layer not in UNIT_FIELDS:
            raise ValueError("Choose a known unit type and supported technology level.")
        definition = self.unit_types[key]
        if not 0 <= unit.owner <= 4 or not 0 <= unit.arrival <= 100:
            raise ValueError("Owner must be 0..4 and arrival turn must be Start or 1..100.")
        if unit.layer == "reinforcements":
            if unit.owner != 0:
                raise ValueError("Native reinforcements belong to owner 0 (AI); player units must start on the map.")
        elif (unit.arrival != 0 or not 0 <= unit.facing <= 7 or not 1 <= unit.armor <= definition.armor
              or not 0 <= unit.ap <= definition.ap):
            raise ValueError(f"Starting units need facing 0..7, health 1..{definition.armor} and AP 0..{definition.ap}.")
        relocating = old is None or (unit.kind, unit.x, unit.y, unit.arrival, unit.layer) != (
            old.kind, old.x, old.y, old.arrival, old.layer)
        if not relocating:
            return
        footprint = self.unit_footprint(unit)
        if not footprint <= self.cells.keys():
            raise ValueError("The unit footprint extends outside the map.")
        for key in footprint:
            if not unit_terrain_compatible(definition, self.terrains[self.cells[key].terrain.upper()]):
                raise ValueError(f"Unit requires compatible terrain at {key} (land, forest infantry, water or air).")
            if key in self.static_blocked or key in self.deployment_positions:
                raise ValueError(f"Tile is reserved by {self.static_blocked.get(key, 'Deployment')}.")
            if self.building_at(key):
                raise ValueError("Unit overlaps a building footprint.")
            for layer in ("walls", "decors", "mines"):
                for counter in self.surface_positions[layer].get(key, ()):
                    item = self.surfaces[layer][counter]
                    if layer in ("walls", "mines") or not self.surface_types[layer][item.kind.upper()].passable:
                        raise ValueError(f"Unit overlaps blocking {layer} at {key}.")
            if any(other.identifier != unit.identifier for other in self.units_at(key, unit.arrival)):
                raise ValueError(f"Unit overlaps another placement on the same arrival turn at {key}.")

    def add_unit(self, kind: int, x: int, y: int, arrival: int = 0, owner: int = 0, tech: int = 0,
                 facing: int = 0, armor: int | None = None, ap: int | None = None,
                 arrival_order: bool = False) -> Unit:
        self.end_stroke()
        if (kind, tech) not in self.unit_types:
            raise ValueError("Choose a known unit type and supported technology level.")
        if arrival and (tech != 0 or facing != 0 or armor is not None or ap is not None):
            raise ValueError("Reinforcement technology, facing, health and AP are assigned by the game.")
        definition = self.unit_types[kind, tech]
        unit = Unit(self.next_unit_id, kind, x, y, arrival, "reinforcements" if arrival else "units",
                    owner, tech, facing, definition.armor if armor is None else armor,
                    definition.ap if ap is None else ap, arrival_order=arrival_order)
        if self.next_unit_id >= 1000000:
            raise ValueError("The scenario has exhausted unit IDs (must be below 1000000).")
        self.validate_unit(unit)
        edit = Edit(units={unit.identifier: (None, unit)}, next_unit_id=(self.next_unit_id, self.next_unit_id + 1))
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()
        return unit

    def update_unit(self, identifier: int, **properties) -> Unit:
        self.end_stroke()
        old = self.units[identifier]
        if set(properties) - {"kind", "x", "y", "arrival", "owner", "tech", "facing", "armor", "ap", "arrival_order"}:
            raise ValueError("Unit IDs, database counters and existing AI-order fields cannot be edited.")
        layer = ("reinforcements" if properties["arrival"] else "units") \
            if "arrival" in properties and properties["arrival"] != old.arrival else old.layer
        if layer != old.layer and identifier in self.unit_references:
            raise ValueError("Mission objectives/orders reference this unit; changing its arrival category is blocked.")
        if self.scripts is not None and layer != old.layer and old.layer == "units" and old.order_id:
            raise ValueError("Clear the starting unit's AI order before changing its arrival category.")
        if layer == "reinforcements" and old.layer == layer:
            for key in ("owner", "tech", "facing", "armor", "ap"):
                if key in properties and properties[key] != getattr(old, key):
                    raise ValueError("Reinforcement owner, technology, facing, health and AP are assigned by the game.")
        unit = replace(old, layer=layer, **properties)
        self.validate_unit(unit, old)
        if unit != old:
            edit = Edit(units={identifier: (old, unit)})
            self._apply_edit(edit, 1)
            self.undo_stack.append(edit)
            self.redo_stack.clear()
        return unit

    def delete_unit(self, identifier: int):
        self.end_stroke()
        if identifier in self.unit_references:
            raise ValueError("Mission objectives/orders reference this unit; deletion is blocked.")
        edit = Edit(units={identifier: (self.units[identifier], None)})
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()

    def surface_at(self, layer: str, key: tuple[int, int]) -> Surface | None:
        counters = self.surface_positions[layer].get(key)
        return self.surfaces[layer][min(counters)] if counters else None

    def _set_surface(self, layer: str, counter: int, item: Surface | None):
        old = self.surfaces[layer].get(counter)
        if old:
            key = old.x, old.y
            self.surface_positions[layer][key].remove(counter)
            if not self.surface_positions[layer][key]:
                del self.surface_positions[layer][key]
        if item is None:
            self.surfaces[layer].pop(counter, None)
        else:
            self.surfaces[layer][counter] = item
            self.surface_positions[layer].setdefault((item.x, item.y), set()).add(counter)

    def surface_rows(self) -> dict[str, list[dict[str, str]]]:
        return {layer: [items[key].row(layer) for key in sorted(items)]
                for layer, items in self.surfaces.items()}

    def connection_sprite(self, layer: str, item: Surface) -> int:
        mask = sum(bit for dx, dy, bit in NEIGHBORS
                   if (neighbor := self.surface_at(layer, (item.x + dx, item.y + dy)))
                   and neighbor.kind == item.kind)
        return CONNECTION_SPRITES[mask]

    def surface_sprite(self, layer: str, item: Surface) -> str:
        definition = self.surface_types[layer][item.kind.upper()]
        if layer == "mines":
            return f"{definition.image}000"
        if layer == "decors":
            return definition.image
        shape = self.connection_sprite(layer, item)
        if layer == "walls":
            if item.broken:
                shape = {8: 19, 9: 18, 10: 16, 11: 17}.get(shape, shape)
            # Fresh-battle initialization resets HP; preview intact artwork without changing stored health.
            return f"{definition.image}0{shape:02d}"
        terrain = self.terrains[self.cells[item.x, item.y].terrain.upper()]
        mode = "B" if terrain.kind == 1 else "R" if terrain.kind == 2 else "G"
        if mode == "R":
            def terrain_kind(dx, dy):
                cell = self.cells.get((item.x + dx, item.y + dy))
                return self.terrains[cell.terrain.upper()].kind if cell else None

            orientation = (0 if terrain_kind(-1, 0) in (1, 2) and terrain_kind(1, 0) == 3 else
                           1 if terrain_kind(1, 0) in (1, 2) and terrain_kind(-1, 0) == 3 else
                           2 if terrain_kind(0, -1) in (1, 2) and terrain_kind(0, 1) == 3 else 3)
            variants = {0: (16, 20, 24, 28), 6: (17, 21, 32, 32), 8: (18, 22, 32, 32),
                        11: (19, 23, 32, 32), 7: (32, 32, 25, 29),
                        9: (32, 32, 26, 30), 10: (32, 32, 27, 31)}
            shape = variants.get(shape, (32, 32, 32, 32))[orientation]
        return f"{definition.image}{mode}{shape:03d}"

    def bridge_spans(self) -> tuple[BridgeSpan, ...]:
        def kind(key):
            cell = self.cells.get(key)
            return self.terrains[cell.terrain.upper()].kind if cell else None

        candidates = []
        for y in range(self.size - 1):
            for x in range(self.size - 1):
                if kind((x, y)) != 2:
                    continue
                direction = ((1, 0) if kind((x + 1, y)) in (1, 2) and kind((x - 1, y)) == 3 else
                             (0, 1) if kind((x, y + 1)) in (1, 2) and kind((x, y - 1)) == 3 else None)
                if direction is None:
                    continue
                dx, dy = direction
                end = x + dx, y + dy
                while kind(end) == 1:
                    end = end[0] + dx, end[1] + dy
                if kind(end) == 2 and kind((end[0] + dx, end[1] + dy)) == 3:
                    candidates.append(BridgeSpan((x, y), end))
        pruned = []
        while candidates:
            span = candidates.pop(0)
            pruned.append(span)
            # The native comparison uses axis-wise OR, not Manhattan distance or orientation.
            candidates = [other for other in candidates if not (
                any(abs(span.start[axis] - other.start[axis]) == 1 for axis in (0, 1))
                and any(abs(span.end[axis] - other.end[axis]) == 1 for axis in (0, 1)))]
        result, used = [], set()
        for span in pruned:
            if not used.intersection(span.tiles):
                result.append(span)
                used.update(span.tiles)
        return tuple(result)

    @property
    def protected(self) -> set[tuple[int, int]]:
        return {key for key, cell in self.cells.items() if not self.terrains[cell.terrain.upper()].paintable}

    def footprint(self, building: Building) -> set[tuple[int, int]]:
        kind = self.building_types[building.kind]
        return {(building.x + dx, building.y + dy) for dx in range(kind.width) for dy in range(kind.height)}

    def building_at(self, key: tuple[int, int]) -> Building | None:
        return next((item for item in self.buildings.values() if key in self.footprint(item)), None)

    @property
    def dirty(self) -> bool:
        return self.state() != self.saved_state

    def mark_saved(self):
        self.saved_state = self.state()

    def begin_stroke(self):
        if self.stroke is not None:
            raise ValueError("Finish the current paint stroke first.")
        self.stroke = {}
        self.surface_stroke = {}
        self.deployment_stroke = {}

    def paint(self, x: int, y: int, terrain: str, mode: str = "ground") -> bool:
        terrain = terrain.upper()
        if self.stroke is None:
            raise ValueError("Painting requires an active stroke.")
        if terrain not in self.terrains or mode not in (
                "ground", "trees", "erase", "scenery", "erase_scenery", "bridge", "erase_bridge"):
            raise ValueError("Unknown texture or painting mode.")
        target = self.terrains[terrain]
        if not (target.kind == 2 and not target.overlapping if mode == "bridge" else
                target.kind == 1 and not target.overlapping if mode == "erase_bridge" else
                target.tree if mode == "trees" else target.scenery if mode == "scenery" else target.paintable):
            raise ValueError("Choose a matching ground, forest or scenery texture for this tool.")
        if (x, y) not in self.cells:
            raise ValueError("The brush is outside the map.")
        current = self.terrains[self.cells[x, y].terrain.upper()]
        if mode in ("bridge", "erase_bridge"):
            if mode == "erase_bridge" and current.kind != 2:
                return False
            if current.kind not in (1, 2) or current.overlapping:
                raise ValueError("Bridge banks can only replace deep-water or riverbank terrain.")
            if (x, y) in self.blocked or self.building_at((x, y)):
                raise ValueError("Bridge bank edits cannot overlap objects, units or deployment reservations.")
        if mode == "erase" and not current.tree:
            return False
        if mode == "erase_scenery" and not current.scenery:
            return False
        if not (mode in ("bridge", "erase_bridge") or current.paintable or (mode in ("trees", "erase") and current.tree)
                or (mode in ("scenery", "erase_scenery") and current.scenery)):
            raise ValueError("This terrain contains a protected tree or obstacle.")
        if mode in ("trees", "scenery"):
            if (x, y) in self.blocked:
                raise ValueError(f"Tile is reserved by {self.blocked[x, y]}.")
            if self.building_at((x, y)):
                raise ValueError("Trees/scenery cannot overlap a building footprint.")
        cell = self.cells[x, y]
        if cell.terrain.upper() == terrain:
            return False
        old = self.stroke.get((x, y), (cell.terrain, terrain))[0]
        cell.terrain = terrain
        if old.upper() == terrain:
            cell.terrain = old
            self.stroke.pop((x, y), None)
        else:
            self.stroke[x, y] = old, terrain
        return True

    def end_stroke(self):
        if self.stroke is not None and (self.stroke or self.surface_stroke or self.deployment_stroke):
            self.undo_stack.append(Edit(cells=self.stroke, surfaces=self.surface_stroke,
                                        deployments=self.deployment_stroke))
            self.redo_stack.clear()
        self.stroke = None
        self.surface_stroke = {}
        self.deployment_stroke = {}

    def validate_surface(self, layer: str, item: Surface, old: Surface | None = None):
        if layer not in self.surfaces or item.kind.upper() not in self.surface_types[layer]:
            raise ValueError("Choose a known object type.")
        definition = self.surface_types[layer][item.kind.upper()]
        if layer != "decors" and not 0 <= item.owner <= 4:
            raise ValueError("Owner must be 0..4.")
        if layer == "walls" and not 1 <= item.armor <= definition.armor:
            raise ValueError(f"Wall armor must be 1..{definition.armor}.")
        if layer == "walls" and item.broken and item.kind != "0":
            raise ValueError("Broken-section shapes are available only for concrete walls.")
        if layer == "mines" and any(type(getattr(item, name)) is not bool for name in MINE_FLAGS):
            raise ValueError("Mine armed/hidden flags must be Boolean values.")
        relocating = old is None or (item.kind, item.x, item.y) != (old.kind, old.x, old.y)
        if not relocating:
            return
        key = item.x, item.y
        if key not in self.cells:
            raise ValueError("The object is outside the map.")
        terrain = self.terrains[self.cells[key].terrain.upper()]
        compatible = terrain.paintable or (layer == "roads" and item.kind == "1"
                                           and terrain.kind in (1, 2) and not terrain.overlapping)
        if not compatible:
            raise ValueError(f"{terrain.name} at {key} is incompatible. Objects require clear land; "
                             "paved roads also support deep water and riverbanks.")
        if layer != "roads" and (key in self.static_blocked or key in self.deployment_positions
                                or key in self.unit_positions):
            reason = self.static_blocked.get(
                key, "Deployment" if key in self.deployment_positions else "a unit / reinforcement")
            raise ValueError(f"Tile is reserved by {reason}.")
        if self.building_at(key):
            raise ValueError("Object overlaps a building footprint.")
        for other_layer, positions in self.surface_positions.items():
            if {layer, other_layer} == {"roads", "mines"}:
                continue
            if any((other_layer, counter) != (layer, item.counter) for counter in positions.get(key, ())):
                raise ValueError(f"Object overlaps {SURFACE_TABLES[other_layer]} at {key}.")

    def _check_road_shapes(self, item: Surface):
        for dx, dy, _ in ((0, 0, 0), *NEIGHBORS):
            neighbor = self.surface_at("roads", (item.x + dx, item.y + dy))
            if neighbor and (neighbor.x, neighbor.y) in self.cells:
                name = self.surface_sprite("roads", neighbor)
                if ((name[4] == "B" and int(name[5:]) not in (0, 6, 7, 8, 9, 10, 11))
                        or name.endswith("R032")):
                    raise ValueError("This road connection has no bridge artwork; bridges must run straight.")

    def _record_surface(self, layer: str, old: Surface | None, item: Surface | None):
        counter = (item or old).counter
        values = self.surface_stroke.get((layer, counter), (old, item))
        self._set_surface(layer, counter, item)
        if values[0] == item:
            self.surface_stroke.pop((layer, counter), None)
        else:
            self.surface_stroke[layer, counter] = values[0], item

    def paint_surface(self, layer: str, x: int, y: int, kind: str, owner: int = 0,
                      armor: int | None = None, broken: bool = False, erase: bool = False,
                      mine_flags: dict[str, bool] | None = None) -> bool:
        if self.stroke is None:
            raise ValueError("Painting requires an active stroke.")
        if layer not in self.surfaces or (x, y) not in self.cells:
            raise ValueError("Unknown layer or position outside the map.")
        old = self.surface_at(layer, (x, y))
        if erase:
            if old is None:
                return False
            item = None
        else:
            if kind.upper() not in self.surface_types[layer]:
                raise ValueError("Choose a known object type.")
            definition = self.surface_types[layer][kind.upper()]
            counter = old.counter if old else self.next_surface_counter
            item = Surface(counter, kind, x, y, owner if layer != "decors" else 0,
                           (definition.armor if armor is None else armor) if layer == "walls" else 0,
                           broken if layer == "walls" else False)
            if layer == "mines" and mine_flags is not None:
                if set(mine_flags) != set(MINE_FLAGS):
                    raise ValueError("Provide all six native mine flags.")
                item = replace(item, **mine_flags)
            self.validate_surface(layer, item, old)
        if old == item:
            return False
        if layer == "roads":
            counter = (item or old).counter
            self._set_surface(layer, counter, item)
            try:
                self._check_road_shapes(item or old)
            finally:
                self._set_surface(layer, counter, old)
        self._record_surface(layer, old, item)
        if old is None:
            self.next_surface_counter -= 1
        return True

    def update_surface(self, layer: str, counter: int, **properties) -> Surface:
        self.end_stroke()
        allowed = {"kind", "x", "y"} | ({"owner"} if layer == "roads" else
                                      {"owner", "armor", "broken"} if layer == "walls" else
                                      {"owner", *MINE_FLAGS} if layer == "mines" else set())
        if set(properties) - allowed:
            raise ValueError("Object counters and unrelated properties cannot be edited.")
        old = self.surfaces[layer][counter]
        item = replace(old, **properties)
        self.validate_surface(layer, item, old)
        if item == old:
            return old
        self._set_surface(layer, counter, item)
        try:
            if layer == "roads":
                self._check_road_shapes(old)
                self._check_road_shapes(item)
        finally:
            self._set_surface(layer, counter, old)
        edit = Edit(surfaces={(layer, counter): (old, item)})
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()
        return item

    def delete_surface(self, layer: str, counter: int):
        self.end_stroke()
        old = self.surfaces[layer][counter]
        self._set_surface(layer, counter, None)
        try:
            if layer == "roads":
                self._check_road_shapes(old)
        finally:
            self._set_surface(layer, counter, old)
        edit = Edit(surfaces={(layer, counter): (old, None)})
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()

    def undo(self) -> set[tuple[int, int]]:
        self.end_stroke()
        if not self.undo_stack:
            return set()
        changes = self.undo_stack.pop()
        keys = self._apply_edit(changes, 0)
        self.redo_stack.append(changes)
        return keys

    def redo(self) -> set[tuple[int, int]]:
        self.end_stroke()
        if not self.redo_stack:
            return set()
        changes = self.redo_stack.pop()
        keys = self._apply_edit(changes, 1)
        self.undo_stack.append(changes)
        return keys

    def _apply_edit(self, edit: Edit, direction: int) -> set[tuple[int, int]]:
        keys = set(edit.cells)
        if edit.grid is not None:
            keys.update(self.cells)
            self.cells = {(x, y): Cell(identifier, x, y, terrain)
                          for identifier, x, y, terrain in edit.grid[direction]}
            self.size = math.isqrt(len(self.cells))
            keys.update(self.cells)
        for key, values in edit.cells.items():
            self.cells[key].terrain = values[direction]
        for identifier, values in edit.buildings.items():
            for building in values:
                if building:
                    keys.update(self.footprint(building) & self.cells.keys())
            if values[direction] is None:
                self.buildings.pop(identifier, None)
            else:
                self.buildings[identifier] = values[direction]
        if edit.next_id is not None:
            self.next_building_id = edit.next_id[direction]
        for identifier, values in edit.units.items():
            for unit in values:
                if unit:
                    keys.update(self.unit_footprint(unit) & self.cells.keys())
            self._set_unit(identifier, values[direction])
        if edit.next_unit_id is not None:
            self.next_unit_id = edit.next_unit_id[direction]
        if edit.scenario is not None:
            self.scenario = edit.scenario[direction]
        if edit.scripts is not None:
            self.scripts = edit.scripts[direction]
            self._refresh_script_references()
        for counter, values in edit.deployments.items():
            for item in values:
                if item:
                    keys.add((item.x, item.y))
            self._set_deployment(counter, values[direction])
        for (layer, counter), values in edit.surfaces.items():
            for item in values:
                if item:
                    keys.add((item.x, item.y))
            self._set_surface(layer, counter, values[direction])
        return keys

    def resize(self, size: int, terrain: str):
        self.end_stroke()
        if type(size) is not int or not 1 <= size <= 256:
            raise ValueError("Choose a square map size from 1 to 256.")
        terrain = terrain.upper()
        if terrain not in self.terrains or not self.terrains[terrain].paintable:
            raise ValueError("Choose clear ground for newly added tiles.")
        if size == self.size:
            return False
        if size < self.size:
            outside = lambda key: key in self.cells and not (0 <= key[0] < size and 0 <= key[1] < size)
            blockers = []
            for item in self.buildings.values():
                if any(map(outside, self.footprint(item))):
                    blockers.append(f"Building {item.identifier}")
            for item in self.units.values():
                if any(map(outside, self.unit_footprint(item))):
                    blockers.append(f"Unit/reinforcement {item.identifier}")
            for layer, items in self.surfaces.items():
                blockers.extend(f"{layer} {item.counter}" for item in items.values()
                                if outside((item.x, item.y)))
            blockers.extend(f"Deployment {item.counter}" for item in self.deployments.values()
                            if outside((item.x, item.y)))
            blockers.extend(f"{reason} at {key}" for key, reason in self.static_blocked.items() if outside(key))
            for row in self.resize_reservations:
                if row["table"] == "Taurans On Map" or outside((int(row["x"]), int(row["y"]))):
                    blockers.append(f"Preserved {row['table']} row {row['counter']}")
            if self.scripts:
                for event in self.scripts.events:
                    if outside((event.x, event.y)) or outside((event.dest_x, event.dest_y)):
                        blockers.append(f"Support event {event.counter}")
                for target in (*self.scripts.objective_positions, *self.scripts.order_positions):
                    if outside((target.x, target.y)):
                        blockers.append(f"Script tile {target.x},{target.y}")
            if blockers:
                raise ValueError("Cannot shrink: move/remove affected records first:\n"
                                 + "\n".join(blockers[:20])
                                 + (f"\n...and {len(blockers) - 20} more." if len(blockers) > 20 else ""))
        before = tuple((cell.identifier, cell.x, cell.y, cell.terrain) for cell in self.cells.values())
        next_id = max(self.original_cell_floor, max(cell.identifier for cell in self.cells.values()) + 1)
        after = []
        for y in range(size):
            for x in range(size):
                old = self.cells.get((x, y))
                if old:
                    after.append((old.identifier, x, y, old.terrain))
                else:
                    identifier = self.original_cell_ids.get((x, y))
                    if identifier is None:
                        identifier = next_id
                        next_id += 1
                    if identifier > 2147483647:
                        raise ValueError("Map cell IDs are exhausted.")
                    after.append((identifier, x, y, terrain))
        edit = Edit(grid=(before, tuple(after)))
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()
        return True

    def validate_building(self, building: Building, old: Building | None = None):
        if building.kind not in self.building_types:
            raise ValueError("Choose a known building type.")
        kind = self.building_types[building.kind]
        if not 0 <= building.owner <= 4 or not 1 <= building.armor <= kind.armor:
            raise ValueError(f"Owner must be 0..4 and armor must be 1..{kind.armor}.")
        # Property edits must not invalidate unrelated, pre-existing scenario placements.
        relocating = old is None or (building.kind, building.x, building.y) != (old.kind, old.x, old.y)
        if not relocating:
            return
        cells = self.footprint(building)
        if not cells <= self.cells.keys():
            raise ValueError("The building footprint extends outside the map.")
        for key in cells:
            terrain = self.terrains[self.cells[key].terrain.upper()]
            if terrain.kind != kind.position or terrain.overlapping:
                raise ValueError(f"Building needs {'deep water' if kind.position == 1 else 'clear land'} at {key}.")
            if key in self.blocked:
                raise ValueError(f"Building overlaps {self.blocked[key]} at {key}.")
        for other in self.buildings.values():
            if other.identifier == building.identifier:
                continue
            if kind.unique and other.kind == building.kind:
                raise ValueError("This building type is unique on the map.")
            if cells & self.footprint(other):
                raise ValueError(f"Building overlaps building {other.identifier}.")

    def add_building(self, kind: int, owner: int, x: int, y: int,
                     armor: int | None = None, ai_target: bool | None = None) -> Building:
        self.end_stroke()
        if kind not in self.building_types:
            raise ValueError("Choose a known building type.")
        definition = self.building_types[kind]
        item = Building(self.next_building_id, kind, owner, definition.armor if armor is None else armor,
                        x, y, definition.ai_target if ai_target is None else ai_target)
        if item.identifier >= 2147483647:
            raise ValueError("The scenario has exhausted building IDs.")
        self.validate_building(item)
        edit = Edit(buildings={item.identifier: (None, item)},
                    next_id=(self.next_building_id, self.next_building_id + 1))
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()
        return item

    def update_building(self, identifier: int, **properties) -> Building:
        self.end_stroke()
        old = self.buildings[identifier]
        if set(properties) - {"kind", "owner", "armor", "x", "y", "ai_target"}:
            raise ValueError("Building IDs and database counters cannot be edited.")
        item = replace(old, **properties)
        self.validate_building(item, old)
        if item != old:
            edit = Edit(buildings={identifier: (old, item)})
            self._apply_edit(edit, 1)
            self.undo_stack.append(edit)
            self.redo_stack.clear()
        return item

    def delete_building(self, identifier: int):
        self.end_stroke()
        if identifier in self.references:
            raise ValueError("This building is referenced by a mission objective or unit order; deletion is blocked.")
        old = self.buildings[identifier]
        edit = Edit(buildings={identifier: (old, None)})
        self._apply_edit(edit, 1)
        self.undo_stack.append(edit)
        self.redo_stack.clear()

    def building_rows(self) -> list[dict[str, str]]:
        return [self.buildings[key].row() for key in sorted(self.buildings)]

    def rows(self) -> list[dict[str, str]]:
        return [{"Id": str(cell.identifier), "PosX": str(cell.x), "PosY": str(cell.y),
                 "TerrainImg": cell.terrain}
                for _, cell in sorted(self.cells.items(), key=lambda item: (item[0][1], item[0][0]))]


def tile_at(px: float, py: float, origin_x: float, origin_y: float,
            scale: float, size: int) -> tuple[int, int] | None:
    dx, dy = (px - origin_x) / (27 * scale), (py - origin_y) / (14 * scale)
    x, y = math.floor((dx + dy) / 2 + 0.5), math.floor((dy - dx) / 2 + 0.5)
    return (x, y) if 0 <= x < size and 0 <= y < size else None


def line_cells(start: tuple[int, int], end: tuple[int, int], cardinal: bool = False) -> list[tuple[int, int]]:
    x0, y0 = start
    x1, y1 = end
    distance = max(abs(x1 - x0), abs(y1 - y0))
    if distance == 0:
        return [start]
    points = [(round(x0 + (x1 - x0) * step / distance),
               round(y0 + (y1 - y0) * step / distance)) for step in range(distance + 1)]
    if not cardinal:
        return points
    result = []
    for point in points:
        if result and point[0] != result[-1][0] and point[1] != result[-1][1]:
            result.append((point[0], result[-1][1]))
        result.append(point)
    return result


class MapToolError(RuntimeError):
    pass


def run_map_tool(script: Path, **arguments: object):
    if os.name != "nt":
        raise MapToolError("MDB editing requires Windows and 32-bit Jet OLE DB.")
    powershell = Path(os.environ.get("SystemRoot", r"C:\Windows")) / "SysWOW64" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
    command = [str(powershell), "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
               "-File", str(script)]
    for name, value in arguments.items():
        command.extend((f"-{name}", str(value)))
    process = subprocess.run(command, capture_output=True, creationflags=subprocess.CREATE_NO_WINDOW,
                             timeout=120)
    if process.returncode != 0:
        output = (process.stderr + b"\n" + process.stdout).decode("utf-8", errors="backslashreplace").strip()
        raise MapToolError(f"The database editor failed:\n{output}")


def archive_members(path: Path) -> list[str]:
    with zipfile.ZipFile(path) as archive:
        members = [entry.filename for entry in archive.infolist()
                   if entry.filename.lower().endswith(".mdb") and not entry.is_dir()]
        if not members or len({name.lower() for name in members}) != len(members):
            raise ValueError("The archive has no scenarios or has duplicate scenario names.")
        for name in members:
            member_name(name)
        return sorted(members, key=str.lower)


CAMPAIGN_MAP_NAME = re.compile(r"(s[1-4]-[0-9]+-)([12])\.mdb", re.IGNORECASE)


def campaign_map_variant(source: Path, member: str | None = None) -> int | None:
    match = CAMPAIGN_MAP_NAME.fullmatch(member or source.name)
    return int(match[2]) if match else None


def alternate_map_source(source: Path, member: str | None = None) -> tuple[Path, str | None] | None:
    source = source.resolve()
    match = CAMPAIGN_MAP_NAME.fullmatch(member or source.name)
    if match is None:
        return None
    variant = 3 - int(match[2])
    name = f"{match[1]}{variant}.mdb"
    if source.suffix.lower() == ".mdb" and member is None:
        other = source.with_name(name)
        return (other, None) if other.is_file() else None
    if source.suffix.lower() != ".zip" or member is None:
        return None
    candidates = (source, source.with_name(f"SCEN{variant}.ZIP"))
    for archive in candidates:
        if archive.is_file():
            other = next((entry for entry in archive_members(archive) if entry.lower() == name.lower()), None)
            if other is not None:
                return archive, other
    return None


class Document:
    def __init__(self, source: Path, member: str | None, temporary: tempfile.TemporaryDirectory,
                 model: MapModel, script: Path, registry: Path):
        self.source = source
        self.member = member
        self.temporary = temporary
        self.directory = Path(temporary.name)
        self.model = model
        self.original_scenario = model.scenario
        self.script = script
        self.registry = registry

    @classmethod
    def open(cls, source: Path, member: str | None, script: Path, registry: Path,
             terrains: dict[str, Terrain]) -> Document:
        source = source.resolve()
        temporary = tempfile.TemporaryDirectory(prefix="libday-map-")
        folder = Path(temporary.name)
        complete = False
        try:
            if source.suffix.lower() == ".zip":
                if member is None:
                    raise ValueError("Choose a scenario from the archive.")
                shutil.copyfile(source, folder / "source.zip")
                extract_map(folder / "source.zip", member, folder / "source.mdb")
            elif source.suffix.lower() == ".mdb":
                if member is not None:
                    raise ValueError("A standalone MDB cannot have an archive member.")
                require_mdb(source.read_bytes())
                shutil.copyfile(source, folder / "source.mdb")
            else:
                raise ValueError("Open a scenario MDB or a scenario ZIP archive.")
            run_map_tool(script, Action="Export", Source=folder / "source.mdb", Output=folder / "map.csv")
            run_map_tool(script, Action="ExportObjects", Source=folder / "source.mdb", Output=folder / "objects.json")
            objects = json.loads((folder / "objects.json").read_text(encoding="utf-8-sig"))
            with (folder / "map.csv").open(newline="", encoding="utf-8-sig") as stream:
                model = MapModel(list(csv.DictReader(stream)), terrains,
                                 load_building_types(script.with_name("building-types.csv")), objects,
                                 unit_types=load_unit_types(script.with_name("unit-types.csv")))
            document = cls(source, member, temporary, model, script, registry)
            complete = True
            return document
        finally:
            if not complete:
                temporary.cleanup()

    def save_copy(self, output: Path, rows: list[dict[str, str]],
                  buildings: list[dict[str, str]] | None = None, next_id: int | None = None,
                  surfaces: dict[str, list[dict[str, str]]] | None = None,
                  units: dict[str, list[dict[str, str]]] | None = None, next_unit_id: int | None = None,
                  scenario: ScenarioSettings | None = None,
                  deployments: list[dict[str, int]] | None = None,
                  scripts: ScriptData | None = None):
        output = output.resolve()
        if output.exists():
            raise FileExistsError("Choose a new filename. Saving never overwrites an existing file.")
        if output.suffix.lower() not in (".mdb", ".zip"):
            raise ValueError("Save the map as .mdb, or an opened archive as .zip.")
        if output.suffix.lower() == ".zip" and self.member is None:
            raise ValueError("Open a ZIP archive first to save an archive copy.")
        with tempfile.TemporaryDirectory(prefix="libday-save-") as work:
            folder = Path(work)
            csv_path = folder / "map.csv"
            with csv_path.open("x", newline="", encoding="utf-8-sig") as stream:
                writer = csv.DictWriter(stream, fieldnames=("Id", "PosX", "PosY", "TerrainImg"))
                writer.writeheader()
                writer.writerows(rows)
            edited = folder / "edited.mdb"
            buildings_path = folder / "buildings.csv"
            with buildings_path.open("x", newline="", encoding="utf-8-sig") as stream:
                writer = csv.DictWriter(stream, fieldnames=BUILDING_FIELDS)
                writer.writeheader()
                writer.writerows(self.model.building_rows() if buildings is None else buildings)
            surface_path = folder / "surfaces.json"
            surface_path.write_text(json.dumps(self.model.surface_rows() if surfaces is None else surfaces),
                                    encoding="utf-8")
            unit_path = folder / "units.json"
            unit_path.write_text(json.dumps(self.model.unit_rows() if units is None else units), encoding="utf-8")
            deployment_path = folder / "deployments.json"
            deployment_path.write_text(json.dumps(self.model.deployment_rows() if deployments is None else deployments),
                                       encoding="utf-8")
            settings = self.model.scenario if scenario is None else scenario
            settings_arguments = {}
            script_data = self.model.scripts if scripts is None else scripts
            if script_data is not None:
                script_path = folder / "scripts.json"
                script_path.write_text(json.dumps(script_data.rows()), encoding="utf-8")
                settings_arguments["ScriptJson"] = script_path
            if settings is not None and settings != self.original_scenario:
                settings_path = folder / "scenario.json"
                settings_path.write_text(json.dumps(settings.row()), encoding="utf-8")
                settings_arguments["ScenarioJson"] = settings_path
            run_map_tool(self.script, Action="Apply", Source=self.directory / "source.mdb",
                         Output=edited, Csv=csv_path, TerrainCsv=self.registry, BuildingCsv=buildings_path,
                         BuildingTypesCsv=self.script.with_name("building-types.csv"),
                         SurfaceJson=surface_path,
                         BridgeTerrain="banks",
                         ResizeMap="square",
                         DeploymentJson=deployment_path,
                         UnitJson=unit_path, NextUnitId=self.model.next_unit_id if next_unit_id is None else next_unit_id,
                         NextBuildingId=self.model.next_building_id if next_id is None else next_id,
                         **settings_arguments)
            # Stage beside the destination, then publish without an overwrite race.
            with tempfile.TemporaryDirectory(prefix=".libday-save-", dir=output.parent) as staging:
                staged = Path(staging) / output.name
                if output.suffix.lower() == ".zip":
                    pack_map(self.directory / "source.zip", self.member, edited, staged)
                else:
                    shutil.copyfile(edited, staged)
                os.rename(staged, output)

    def export_skirmish(self, output: Path, catalog: Path, archive: Path, *, rows: list,
                        buildings: list, next_id: int, surfaces: dict, units: dict,
                        next_unit_id: int, scenario: ScenarioSettings,
                        deployments: list[dict[str, int]] | None = None,
                        scripts: ScriptData | None = None):
        output = output.resolve()
        if output.exists():
            raise FileExistsError("Choose a new folder. Skirmish export never overwrites an existing folder.")
        scenario.validate(self.model.unit_types)
        with tempfile.TemporaryDirectory(prefix=".libday-skirmish-", dir=output.parent) as work:
            folder = Path(work)
            shutil.copyfile(catalog, folder / "catalog-source.mdb")
            shutil.copyfile(archive, folder / "archive-source.zip")
            entries_path = folder / "catalog.json"
            run_map_tool(self.script, Action="ExportCatalog", Source=folder / "catalog-source.mdb",
                         Output=entries_path)
            entries = json.loads(entries_path.read_text(encoding="utf-8-sig"))
            names = {entry["FileName"].lower() for entry in entries}
            names.update(name.lower() for name in archive_members(folder / "archive-source.zip"))
            index = 1
            while f"edit{index}.mdb" in names:
                index += 1
            member = f"edit{index}.mdb"
            member_name(member)
            source_name = self.member or self.source.name
            template = next((entry for entry in entries if entry["FileName"].lower() == source_name.lower()),
                            entries[0] if entries else None)
            if template is None:
                raise ValueError("The installed skirmish catalog is empty.")
            edited = folder / "edited.mdb"
            self.save_copy(edited, rows, buildings, next_id, surfaces, units, next_unit_id, scenario, deployments, scripts)
            bundle = folder / "bundle"
            scens = bundle / "SCENS"
            scens.mkdir(parents=True)
            pack_map(folder / "archive-source.zip", member, edited, scens / "SCEN1.ZIP")
            run_map_tool(self.script, Action="Register", Source=folder / "catalog-source.mdb",
                         Output=scens / "SKIRMISH.MDB", Member=member, Title=scenario.title,
                         TemplateCounter=template["Counter"], ScenarioDatabase=edited)
            os.rename(bundle, output)

    def close(self):
        self.temporary.cleanup()
