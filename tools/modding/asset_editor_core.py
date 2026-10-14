"""Model for the unit/building asset editor (no Tk dependency).

Static sprites live in ``IMGS\\UB.DBI`` (54x56 cells); animations are Smacker
files inside ``ANMSUNIT\\ANMSUNIT.FF``. All decoding and encoding is done by
the openlibday ``libday`` CLI; edits are staged in a private work directory
until they are saved as copies or installed into the game folder.
"""
from __future__ import annotations

import json
import re
import shutil
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

import registries
from map_editor_core import Texture, TerrainArt

# Action letter of unit animation members -> (label, Unit Types field).
ACTIONS = {
    "M": ("Move", "MoveAnimation_Begin"),
    "L": ("Move start", "MoveStartAnimation_Begin"),
    "N": ("Move end", "MoveEndAnimation_Begin"),
    "S": ("Shoot begin", "ShootBeginAnimation_Begin"),
    "T": ("Shoot", "ShootShootAnimation_Begin"),
    "U": ("Shoot end", "ShootEndAnimation_Begin"),
    "P": ("Projectile", "ShootProjectileAnimation_Begin"),
    "I": ("Impact", "ShootImpactAnimation_Begin"),
    "E": ("Explosion", "ExplosionAnimation_Begin"),
}
PLAYER_COLOURS = (1, 2, 3, 4)
TECH_LEVELS = range(11)
CELL_WIDTH, CELL_HEIGHT = 54, 56
UNIT_MEMBER = re.compile(r"(?P<prefix>[A-Z]+)\\P(?P<colour>\d)\\(?P=prefix)(?P=colour)"
                         r"(?P<action>[A-Z])(?P<facing>\d)\.SMK", re.IGNORECASE)

# Unit Types columns grouped for display; anything else falls into "Other".
FIELD_GROUPS = {
    "General": ["NameEnglish", "Race", "Category", "AIcategory", "Prerequisite", "Cost", "CostToNextTL",
                "SizeX", "SizeY", "Facings", "isConvoy", "isAITarget", "DanyUnitValue"],
    "Combat": ["Armor", "actionPoints", "actionPointsShoot", "UnitHasWeapon", "weaponNameEnglish", "weaponType",
               "weaponRange", "weaponMinimumRange", "damageType", "damageBase", "damageRandom", "DamageArea",
               "IsAA", "DamageAA", "HitShortRange", "HitMediumRange", "HitLongRange", "canOverwatch"],
    "Movement and detection": ["Movement", "MineDetectionRange", "Jammer"],
    "Artwork": ["facingsImageID", "ruinImageID", "EncyclopediaFrame", *[f for _, f in ACTIONS.values()],
                "MoveAnimation_PixSpeed", "ShootShootAnimation_ProjBegin", "ShootProjectileAnimation_PixOffset",
                "ShootProjectileAnimation_PixSpeed", "ShootProjectileAnimation_Curve"],
    "Encyclopedia": ["EncyclopediaEnglish", "weaponNameGerman", "weaponNameFrench", "NameGerman", "NameFrench",
                     "EncyclopediaGerman", "EncyclopediaFrench"],
}
HIDDEN_FIELDS = {"Counter", "typeID", "unitTechLevel"}
# Unit Types column -> lookup table whose values label it.
LOOKUPS = {
    "Movement": "Unit Movement Lookup",
    "Category": "Unit Category Lookup",
    "Race": "Race Lookup",
    "AIcategory": "AI Category Lookup",
    "weaponType": "Weapon Type Lookup",
    "damageType": "DamageType Lookup",
    "BuildingCat": "Building Category Lookup",
}


def capabilities(row: dict, labels: dict[str, dict[int, str]] | None = None) -> list[str]:
    """Human-readable capability summary of one Unit Types row."""
    labels = labels or {}
    result = []
    movement = labels.get("Movement", {}).get(row.get("Movement"))
    if movement:
        result.append(f"Movement: {movement}")
    if not row.get("UnitHasWeapon"):
        result.append("Unarmed")
    else:
        result.append("Indirect fire" if row.get("weaponType") == 1 else "Direct fire")
        area = row.get("DamageArea") or 1
        if area > 1:
            result.append(f"Area damage {area}x{area}")
        if row.get("damageType") == 1:
            result.append("Drains action points")
        if row.get("weaponMinimumRange"):
            result.append(f"Minimum range {row['weaponMinimumRange']}")
    if row.get("IsAA"):
        result.append(f"Anti-air (damage {row.get('DamageAA')})")
    if row.get("canOverwatch"):
        result.append("Overwatch")
    if row.get("MineDetectionRange"):
        result.append(f"Mine detection {row['MineDetectionRange']}")
    jammer = row.get("Jammer")
    if jammer == 0:
        result.append("Stealth (Jammer 0)")
    elif jammer is not None and jammer < 100:
        result.append(f"Low signature (Jammer {jammer})")
    if row.get("isConvoy"):
        result.append("Convoy")
    return result


def display_value(column: str, value, labels: dict[str, dict[int, str]]) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "yes" if value else "no"
    label = labels.get(column, {}).get(value) if isinstance(value, int) else None
    if label:
        return f"{value} {label}"
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


def compose(parts: dict[tuple[int, int], Texture], size_x: int, size_y: int) -> Texture:
    """Draws footprint parts in isometric order (as the map editor does)."""
    width, height = (size_x + size_y) * 27, (size_x + size_y - 2) * 14 + CELL_HEIGHT
    rgba = bytearray(width * height * 4)
    for (dx, dy) in sorted(parts, key=lambda point: (sum(point), point[0])):
        texture = parts[dx, dy]
        left, top = (dx - dy + size_y - 1) * 27, (dx + dy) * 14
        for y in range(texture.height):
            row = (top + y) * width
            for x in range(texture.width):
                source = (y * texture.width + x) * 4
                if texture.rgba[source + 3]:
                    target = (row + left + x) * 4
                    rgba[target:target + 4] = texture.rgba[source:source + 4]
    return Texture(width, height, bytes(rgba))


@dataclass
class UnitKind:
    type_id: int
    name: str
    image_id: str
    size: tuple[int, int]
    levels: dict[int, dict] = field(default_factory=dict)

    @property
    def base(self) -> dict:
        return self.levels[min(self.levels)]


@dataclass
class BuildingKind:
    type_id: int
    name: str
    image_id: str
    size: tuple[int, int]
    row: dict


@dataclass(frozen=True)
class Animation:
    member: str
    colour: int | None = None
    action: str | None = None
    facing: int | None = None

    @property
    def label(self) -> str:
        if self.action is None:
            return self.member
        name = ACTIONS.get(self.action, (self.action,))[0]
        return f"P{self.colour} {name} ({self.action}) facing {self.facing}"


class AssetStore:
    """Game data catalogue plus staged sprite/animation edits."""

    def __init__(self, game_root: Path, work: Path | None = None):
        self.game_root = Path(game_root)
        dirs = {}
        for line in self.cli("info").decode().splitlines():
            key, _, value = line.partition(": ")
            dirs[key.strip().lower()] = Path(value.strip())
        self.dbi_path = dirs.get("imgs", self.game_root / "IMGS") / "UB.DBI"
        self.ff_path = dirs.get("anmsunit", self.game_root / "ANMSUNIT") / "ANMSUNIT.FF"
        for path in (self.dbi_path, self.ff_path):
            if not path.is_file():
                raise FileNotFoundError(f"Missing game file {path}")
        self._own_work = work is None
        self.work = Path(work or tempfile.mkdtemp(prefix="libday-assets-"))
        self.work.mkdir(parents=True, exist_ok=True)
        self.staged_dbi = self.work / "UB.DBI"
        shutil.copyfile(self.dbi_path, self.staged_dbi)
        self.art = TerrainArt(self.staged_dbi)
        self.sprite_edits: set[str] = set()
        self.animation_edits: dict[str, Path] = {}
        self._frames: dict[str, Path] = {}
        self.labels = self._load_labels()
        self.units = self._load_units()
        self.buildings = self._load_buildings()
        self.members = [line.split("\t", 1)[0] for line in
                        self.cli("ff-list", self.ff_path).decode().splitlines() if line]
        self._members_upper = {member.upper(): member for member in self.members}

    # -- data -------------------------------------------------------------

    def cli(self, *args) -> bytes:
        return registries.run_cli(self.game_root, *args)

    def table(self, name: str) -> list[dict]:
        return registries.export_table(self.game_root, name)

    def _load_labels(self) -> dict[str, dict[int, str]]:
        labels = {}
        for column, table in LOOKUPS.items():
            mapping = {}
            for row in self.table(table):
                keys = [v for v in row.values() if isinstance(v, int) and not isinstance(v, bool)]
                names = [v for k, v in row.items() if isinstance(v, str) and "English" in k]
                names += [v for v in row.values() if isinstance(v, str)]
                if keys and names:
                    mapping[keys[0]] = re.sub(r"^(movetype|unitcategory|AI_category|weapontype|damagetype|"
                                              r"building_category)_", "", names[0])
            labels[column] = mapping
        return labels

    def _load_units(self) -> dict[int, UnitKind]:
        units: dict[int, UnitKind] = {}
        for row in sorted(self.table("Unit Types"), key=lambda r: (r["typeID"], r["unitTechLevel"])):
            kind = units.get(row["typeID"])
            if kind is None:
                kind = units[row["typeID"]] = UnitKind(row["typeID"], row["NameEnglish"],
                                                       row["facingsImageID"].upper(),
                                                       (row["SizeX"], row["SizeY"]))
            kind.levels[row["unitTechLevel"]] = row
        return units

    def _load_buildings(self) -> dict[int, BuildingKind]:
        return {row["TypeID"]: BuildingKind(row["TypeID"], row["NameEnglish"], row["ImageID"].upper(),
                                            (row["SizeX"], row["SizeY"]), row)
                for row in sorted(self.table("Building Types"), key=lambda r: r["TypeID"])}

    # -- sprites ------------------------------------------------------------

    def sprite_sets(self, prefix: str) -> dict[tuple[int, int], dict[tuple[int, int], str]]:
        """``(frame|level, facing|state) -> {(dx, dy): sprite name}`` for a prefix."""
        pattern = re.compile(re.escape(prefix.upper()) + r"(\d)(\d)(\d)(\d)")
        sets: dict[tuple[int, int], dict[tuple[int, int], str]] = {}
        for name in self.art.records:
            match = pattern.fullmatch(name.upper())
            if match:
                a, b, dx, dy = map(int, match.groups())
                sets.setdefault((a, b), {})[dx, dy] = name
        return dict(sorted(sets.items()))

    def texture(self, name: str) -> Texture:
        return self.art.texture(name)

    def composite(self, parts: dict[tuple[int, int], str], size: tuple[int, int]) -> Texture:
        width = max([size[0]] + [dx + 1 for dx, _ in parts])
        height = max([size[1]] + [dy + 1 for _, dy in parts])
        return compose({key: self.texture(name) for key, name in parts.items()}, width, height)

    def export_sprites(self, names: list[str], folder: Path) -> None:
        self.cli("dbi-export", self.staged_dbi, folder, *names)

    def replace_sprites(self, replacements: dict[str, Path]) -> None:
        """Replaces sprites (name -> PNG) in the staged UB.DBI."""
        if not replacements:
            return
        missing = [name for name in replacements if name.upper() not in self.art.records]
        if missing:
            raise KeyError(f"Unknown sprites: {', '.join(missing)}")
        temp = self.staged_dbi.with_suffix(".tmp")
        self.cli("dbi-replace", self.staged_dbi, temp,
                 *(f"{name}={Path(path).resolve()}" for name, path in replacements.items()))
        temp.replace(self.staged_dbi)
        self.art = TerrainArt(self.staged_dbi)
        self.sprite_edits.update(name.upper() for name in replacements)

    def import_sprite_folder(self, folder: Path, names: list[str]) -> list[str]:
        """Replaces each of ``names`` that has a ``<name>.png`` in ``folder``."""
        files = {path.stem.upper(): path for path in Path(folder).glob("*.png")}
        found = {name: files[name.upper()] for name in names if name.upper() in files}
        self.replace_sprites(found)
        return sorted(found)

    # -- animations ---------------------------------------------------------

    def unit_animations(self, kind: UnitKind) -> list[Animation]:
        result = []
        for member in self.members:
            match = UNIT_MEMBER.fullmatch(member)
            if match and match["prefix"].upper() == kind.image_id:
                result.append(Animation(member, int(match["colour"]), match["action"].upper(),
                                        int(match["facing"])))
        order = list(ACTIONS)
        return sorted(result, key=lambda a: (a.colour, order.index(a.action) if a.action in order else 99,
                                             a.facing))

    def building_animations(self, kind: BuildingKind) -> list[Animation]:
        member = self._members_upper.get(f"BUILDING\\{kind.image_id}.SMK")
        return [Animation(member)] if member else []

    def _source_spec(self, member: str) -> str:
        edited = self.animation_edits.get(member)
        return str(edited) if edited else f"{self.ff_path}!{member}"

    def frames(self, member: str) -> tuple[list[Path], dict]:
        """Exports (cached) the current frames of ``member``; returns PNG paths and metadata."""
        folder = self._frames.get(member)
        if folder is None:
            folder = self.work / "frames" / f"{len(self._frames):05d}"
            if folder.exists():
                shutil.rmtree(folder)
            self.cli("smk-export", self._source_spec(member), folder)
            self._frames[member] = folder
        meta = json.loads((folder / "meta.json").read_text(encoding="utf-8"))
        return sorted(folder.glob("frame_*.png")), meta

    def export_animation(self, member: str, folder: Path) -> None:
        self.cli("smk-export", self._source_spec(member), folder)

    def replace_animation(self, member: str, folder: Path) -> None:
        """Re-encodes ``member`` from ``frame_NNN.png`` files (same frame count and size)."""
        target = self.work / "smk" / member.replace("\\", "/")
        target.parent.mkdir(parents=True, exist_ok=True)
        temp = target.with_suffix(".tmp")
        self.cli("smk-import", self._source_spec(member), Path(folder).resolve(), temp)
        temp.replace(target)
        self.animation_edits[member] = target
        self._frames.pop(member, None)

    def revert_animation(self, member: str) -> None:
        self.animation_edits.pop(member, None)
        self._frames.pop(member, None)

    # -- saving -------------------------------------------------------------

    @property
    def dirty(self) -> bool:
        return bool(self.sprite_edits or self.animation_edits)

    def save(self, dbi_out: Path | None, ff_out: Path | None) -> list[Path]:
        """Writes the edited UB.DBI and/or ANMSUNIT.FF; returns the written paths."""
        written = []
        if dbi_out is not None and self.sprite_edits:
            temp = Path(str(dbi_out) + ".tmp")
            shutil.copyfile(self.staged_dbi, temp)
            temp.replace(dbi_out)
            written.append(Path(dbi_out))
        if ff_out is not None and self.animation_edits:
            temp = Path(str(ff_out) + ".tmp")
            self.cli("ff-replace", self.ff_path, temp,
                     *(f"{member}={path}" for member, path in self.animation_edits.items()))
            temp.replace(ff_out)
            written.append(Path(ff_out))
        return written

    def save_copies(self, folder: Path) -> list[Path]:
        folder = Path(folder)
        folder.mkdir(parents=True, exist_ok=True)
        return self.save(folder / "UB.DBI", folder / "ANMSUNIT.FF")

    def install(self) -> list[Path]:
        """Overwrites the game files, keeping a one-time ``.orig`` backup of each."""
        targets = []
        if self.sprite_edits:
            targets.append(self.dbi_path)
        if self.animation_edits:
            targets.append(self.ff_path)
        for path in targets:
            backup = path.with_name(path.name + ".orig")
            if not backup.exists():
                shutil.copyfile(path, backup)
        if self.animation_edits:
            # ff-replace reads the original archive, so build next to it first.
            pending = self.work / "ANMSUNIT.FF"
            self.save(None, pending)
            shutil.copyfile(pending, self.ff_path)
            pending.unlink()
        if self.sprite_edits:
            self.save(self.dbi_path, None)
        return targets

    def close(self) -> None:
        if self._own_work:
            shutil.rmtree(self.work, ignore_errors=True)
