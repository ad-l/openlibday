"""Generates the editor's CSV type registries from the game's GLOBAL.MDB.

The registries are game data, so they are never committed. They are written
next to the tools (git-ignored) on first use and regenerated when the game
root changes. Extraction uses the openlibday ``libday`` CLI.
"""
from __future__ import annotations

import csv
import json
import os
import shutil
import subprocess
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
REPO = TOOLS.parent.parent
STAMP = TOOLS / ".registries-root"


def _bool(value):
    return "1" if value else "0"


def _text(value):
    if value is None:
        return ""
    if isinstance(value, bool):
        return _bool(value)
    if isinstance(value, float) and value.is_integer():
        return str(int(value))
    return str(value)


# file name -> (table, columns); None columns means every column.
SIMPLE = {
    "building-types.csv": ("Building Types", ["TypeID", "NameEnglish", "SizeX", "SizeY", "Position", "Armor",
                                              "UniqueOnMap", "TransformIntoUnit", "isAItarget", "ImageID",
                                              "BuildingCat", "DamagedImages"]),
    "decor-types.csv": ("Ruin Types", ["id", "can_fire_through", "can_move_through"]),
    "mine-types.csv": ("Mine Types", ["typeID", "nameEnglish", "ImageID", "deployTerrain", "damageBase",
                                      "damageRandom"]),
    "road-types.csv": ("Road Types", ["RoadType", "nameEnglish", "ar", "img_id"]),
    "terrain-types.csv": ("Terrain Types", None),
    "unit-types.csv": ("Unit Types", ["typeID", "unitTechLevel", "NameEnglish", "Race", "Facings", "SizeX",
                                      "SizeY", "Movement", "Armor", "actionPoints", "facingsImageID"]),
    "wall-types.csv": ("Wall Types", ["WallType", "nameEnglish", "Armor", "img_id"]),
}
SORT_KEYS = {"building-types.csv": ("TypeID",), "unit-types.csv": ("typeID", "unitTechLevel")}
SUPPORT = "support-types.csv"
# Automated Units Properties 0..17 describe gameplay; 18+ are animation data.
SUPPORT_PROPERTIES = 18
REGISTRIES = (*SIMPLE, SUPPORT)


def find_cli() -> list[str]:
    """Returns the command prefix for the libday CLI, building it if needed."""
    override = os.environ.get("LIBDAY_CLI")
    if override:
        return [override]
    built = [REPO / "target" / profile / name for profile in ("release", "debug")
             for name in ("libday.exe", "libday")]
    built = [path for path in built if path.is_file()]
    if built:
        return [str(max(built, key=lambda path: path.stat().st_mtime))]
    found = shutil.which("libday")
    if found:
        return [found]
    if (REPO / "Cargo.toml").is_file() and shutil.which("cargo"):
        subprocess.run(["cargo", "build", "--release", "-q", "-p", "libday-tools"], cwd=REPO, check=True)
        return find_cli()
    raise RuntimeError("libday CLI not found; run 'cargo build --release' in the openlibday repository "
                       "or set LIBDAY_CLI")


def run_cli(game_root: Path, *args: str) -> bytes:
    """Runs a libday CLI command against ``game_root`` and returns its stdout."""
    flags = subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0
    result = subprocess.run([*find_cli(), "--root", str(game_root), *map(str, args)],
                            capture_output=True, check=False, creationflags=flags)
    if result.returncode:
        raise RuntimeError(f"libday {args[0]} failed: {result.stderr.decode(errors='replace').strip()}")
    return result.stdout


def export_table(game_root: Path, table: str, database: str = "GLOBALS/GLOBAL.MDB") -> list[dict]:
    output = run_cli(game_root, "mdb-export", database, table)
    return [json.loads(line) for line in output.decode("utf-8").splitlines() if line.strip()]


def _write(path: Path, header: list[str], rows: list[list[str]]) -> None:
    temp = path.with_suffix(".tmp")
    with temp.open("w", newline="", encoding="utf-8") as stream:
        writer = csv.writer(stream)
        writer.writerow(header)
        writer.writerows(rows)
    temp.replace(path)


def generate(game_root: Path, output: Path = TOOLS) -> None:
    game_root = Path(game_root)
    for name, (table, columns) in SIMPLE.items():
        rows = export_table(game_root, table)
        header = columns or (list(rows[0]) if rows else [])
        if name in SORT_KEYS:
            rows.sort(key=lambda row: tuple(row[k] for k in SORT_KEYS[name]))
        values = [[_text(row[c]) for c in header] for row in rows]
        if "ImageID" in header and name == "mine-types.csv":
            column = header.index("ImageID")
            for row in values:
                row[column] = row[column].upper()
        _write(output / name, header, values)
    names = {row["Enum"]: row["PropertyID"] for row in export_table(game_root, "Automated Units Properties")}
    infos = sorted(export_table(game_root, "Automated Units Infos"), key=lambda r: r["Counter"])
    support = []
    for unit in export_table(game_root, "Automated Units"):
        properties = {names[i["Property"]]: i["Value"] for i in infos
                      if i["typeID"] == unit["typeID"] and i["Property"] < SUPPORT_PROPERTIES}
        support.append([_text(unit["typeID"]), unit["NameEnglish"], _text(unit["Race"]), _text(unit["Category"]),
                        json.dumps(properties, separators=(",", ":"))])
    _write(output / SUPPORT, ["typeID", "NameEnglish", "Race", "Category", "properties"], support)


def ensure(game_root: Path, output: Path = TOOLS) -> None:
    """Generates the registries unless they already exist for this game root."""
    stamp = output / STAMP.name
    key = str(Path(game_root).resolve())
    current = stamp.read_text(encoding="utf-8") if stamp.is_file() else None
    if current == key and all((output / name).is_file() for name in REGISTRIES):
        return
    generate(Path(game_root), output)
    stamp.write_text(key, encoding="utf-8")


def default_game_root() -> Path | None:
    value = os.environ.get("LIBDAY_ROOT")
    return Path(value) if value else None


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root", type=Path, default=default_game_root(), required=default_game_root() is None)
    parser.add_argument("--output", type=Path, default=TOOLS)
    arguments = parser.parse_args()
    generate(arguments.game_root, arguments.output)
    (arguments.output / STAMP.name).write_text(str(arguments.game_root.resolve()), encoding="utf-8")
