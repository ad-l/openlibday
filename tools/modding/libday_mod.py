#!/usr/bin/env python3
"""Non-destructive tools for the March 1998 Liberation Day executable/assets."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil
import struct
import sys
import zipfile


ORIGINAL_SHA256 = "c138adb009c3fdb7b9090572d68f52ec59e3019ca8d5fc53fac9a16c52ba5389"
MODE_OFFSET = 0xB8808
ORIGINAL_MODE = struct.pack("<II", 1024, 768)
JET_HEADER = b"\x00\x01\x00\x00Standard Jet DB\x00"


def digest(data):
    return hashlib.sha256(data).hexdigest()


def require_mdb(data):
    if not data.startswith(JET_HEADER):
        raise ValueError("Not a supported Jet MDB database")


def write_new(path, data):
    path = Path(path)
    with path.open("xb") as stream:
        stream.write(data)


def patch_display(source, output, width, height):
    """Replace only the mode-1 table; this is not a complete viewport patch."""
    if not (1024 <= width <= 4096 and 768 <= height <= 2160 and width % 4 == 0):
        raise ValueError("Experimental mode requires width 1024..4096, multiple of 4, height 768..2160")
    original = Path(source).read_bytes()
    if digest(original) != ORIGINAL_SHA256:
        raise ValueError("Unsupported executable SHA-256; no patch was written")
    if original[MODE_OFFSET:MODE_OFFSET + 8] != ORIGINAL_MODE:
        raise ValueError("Display-table bytes do not match the supported build")
    patched = bytearray(original)
    patched[MODE_OFFSET:MODE_OFFSET + 8] = struct.pack("<II", width, height)
    write_new(output, patched)
    return {
        "kind": "experimental-display-table-only",
        "source_sha256": digest(original),
        "output_sha256": digest(patched),
        "file_offset": hex(MODE_OFFSET),
        "virtual_address": "0x4ba008",
        "original": [1024, 768],
        "replacement": [width, height],
        "warning": "Not verified as a larger viewport. UI, surfaces and input still need runtime investigation.",
    }


def member_name(name):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,9}\.[Mm][Dd][Bb]", name):
        raise ValueError("Map member must be a flat MDB filename of at most 13 characters")
    return name


def extract_map(archive, member, output):
    member_name(member)
    with zipfile.ZipFile(archive) as source:
        matches = [item for item in source.infolist() if item.filename.lower() == member.lower()]
        if len(matches) != 1:
            raise ValueError("Expected exactly one matching map member")
        item = matches[0]
        if item.file_size > 64 * 1024 * 1024:
            raise ValueError("Map exceeds the 64 MiB extraction limit")
        data = source.read(item)
    require_mdb(data)
    write_new(output, data)
    return {"member": item.filename, "output": str(output), "sha256": digest(data)}


def pack_map(archive, member, database, output):
    member_name(member)
    data = Path(database).read_bytes()
    require_mdb(data)
    # Read and verify all members before creating an output, keeping compression,
    # metadata and untouched members compatible with the original ArchiveLib.
    with zipfile.ZipFile(archive) as source:
        items = source.infolist()
        names = [item.filename.lower() for item in items]
        if len(names) != len(set(names)):
            raise ValueError("Archive contains ambiguous duplicate names")
        if any(item.compress_type not in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED)
               or item.flag_bits & 1 for item in items):
            raise ValueError("Archive uses unsupported compression or encryption")
        if sum(item.file_size for item in items) > 512 * 1024 * 1024:
            raise ValueError("Archive exceeds the 512 MiB repacking limit")
        contents = [(item, source.read(item)) for item in items]
        comment = source.comment
    with Path(output).open("xb") as stream:
        with zipfile.ZipFile(stream, "w", allowZip64=False) as target:
            target.comment = comment
            replaced = False
            for item, contents_data in contents:
                if item.filename.lower() == member.lower():
                    contents_data = data
                    replaced = True
                target.writestr(item, contents_data)
            if not replaced:
                target.writestr(member, data, compress_type=zipfile.ZIP_DEFLATED)
    return {"member": member, "output": str(output), "replaced": replaced,
            "database_sha256": digest(data)}


def inspect_game(root):
    root = Path(root)
    executable = (root / "EXE" / "LIBDAY.EXE").read_bytes()
    archives = {}
    for name in ("DEATHM.ZIP", "SCEN1.ZIP", "SCEN2.ZIP"):
        with zipfile.ZipFile(root / "SCENS" / name) as archive:
            archives[name] = [
                {"name": item.filename, "bytes": item.file_size, "crc32": f"{item.CRC:08x}"}
                for item in archive.infolist()
            ]
    return {"executable_sha256": digest(executable),
            "supported_build": digest(executable) == ORIGINAL_SHA256,
            "archives": archives}


def stage_game(root, output, windows_output, executable=None):
    """Copy writable assets; keep large, read-only artwork at its installed path."""
    root = Path(root)
    output = Path(output)
    binary = Path(executable or root / "EXE" / "LIBDAY.EXE").read_bytes()
    restored = bytearray(binary)
    restored[MODE_OFFSET:MODE_OFFSET + 8] = ORIGINAL_MODE
    if digest(restored) != ORIGINAL_SHA256:
        raise ValueError("Staging requires the supported executable or its display-table patch")
    if not re.match(r"^[A-Za-z]:\\", windows_output) or any(c in windows_output for c in "\r\n"):
        raise ValueError("--windows-output must be the absolute Windows path corresponding to output")
    ini = (root / "EXE" / "LIBDAY.INI").read_bytes().decode("cp1252")
    for name in ("exe", "globals", "scenarios", "savegame", "cache"):
        directory = "SCENS" if name == "scenarios" else name.upper()
        value = windows_output.rstrip("\\") + "\\" + directory
        ini, count = re.subn(rf"(?im)^{name}=.*$", lambda _: f"{name}={value}\r", ini)
        if count != 1:
            raise ValueError(f"Expected one installation path for {name} in LIBDAY.INI")
    ini, count = re.subn(r"(?im)^Resolution=.*$", "Resolution=1\r", ini)
    if count != 1:
        raise ValueError("Expected one Resolution setting in LIBDAY.INI")
    output.mkdir()
    for name in ("EXE", "GLOBALS", "SCENS", "SAVEGAME", "CACHE"):
        (output / name).mkdir()
    write_new(output / "EXE" / "LIBDAY.EXE", binary)
    for dll in (root / "EXE").iterdir():
        if dll.suffix.lower() in (".dll", ".tsk"):
            shutil.copyfile(dll, output / "EXE" / dll.name)
    music = root / "EXE" / "MUSIC"
    if music.is_dir():
        (output / "EXE" / "MUSIC").mkdir()
        for track in music.iterdir():
            if not track.is_file():
                raise ValueError(f"Unexpected non-file music asset: {track}")
            shutil.copyfile(track, output / "EXE" / "MUSIC" / track.name)
    for name in ("GLOBALS", "SCENS"):
        for asset in (root / name).iterdir():
            if asset.is_file() and asset.suffix.lower() in (".mdb", ".zip"):
                shutil.copyfile(asset, output / name / asset.name)
                if name == "SCENS" and asset.suffix.lower() == ".zip":
                    (output / "CACHE" / asset.stem).mkdir()
    write_new(output / "EXE" / "LIBDAY.INI", ini.encode("cp1252"))
    return {"output": str(output), "windows_output": windows_output,
            "executable_sha256": digest(binary),
            "note": "Separate catalogs, game state, savegames and cache; artwork paths still point to the original installation."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    inspect = commands.add_parser("inspect")
    inspect.add_argument("root", type=Path)
    patch = commands.add_parser("patch-display", help="Experimental display mode only, NOT viewport expansion")
    patch.add_argument("source", type=Path)
    patch.add_argument("output", type=Path)
    patch.add_argument("--width", type=int, default=1920)
    patch.add_argument("--height", type=int, default=1080)
    extract = commands.add_parser("extract-map")
    extract.add_argument("archive", type=Path)
    extract.add_argument("member")
    extract.add_argument("output", type=Path)
    pack = commands.add_parser("pack-map")
    pack.add_argument("archive", type=Path)
    pack.add_argument("member")
    pack.add_argument("database", type=Path)
    pack.add_argument("output", type=Path)
    stage = commands.add_parser("stage-game", help="Create an isolated Windows playtest directory")
    stage.add_argument("root", type=Path)
    stage.add_argument("output", type=Path)
    stage.add_argument("--windows-output", required=True)
    stage.add_argument("--executable", type=Path)
    args = parser.parse_args()
    try:
        if args.command == "inspect":
            result = inspect_game(args.root)
        elif args.command == "patch-display":
            result = patch_display(args.source, args.output, args.width, args.height)
        elif args.command == "extract-map":
            result = extract_map(args.archive, args.member, args.output)
        elif args.command == "pack-map":
            result = pack_map(args.archive, args.member, args.database, args.output)
        else:
            result = stage_game(args.root, args.output, args.windows_output, args.executable)
    except (OSError, ValueError, zipfile.BadZipFile, zipfile.LargeZipFile, RuntimeError) as error:
        parser.exit(1, f"error: {error}\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
