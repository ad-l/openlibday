"""Parity between the openlibday engine's map scene and the map editor canvas.

The engine (`libday map-scene`) must draw exactly the sprites the editor
draws, at the same places and in the same stacking order. Needs a built
`libday` CLI (`cargo build -p libday-tools`); skipped otherwise.
"""
import json
import os
import shutil
import subprocess
import tkinter as tk
import unittest
import zipfile
from pathlib import Path

from testsupport import ROOT, slow
import map_editor as ui

REPO = Path(__file__).resolve().parents[2]
PADDING = 48


def libday_cli() -> Path | None:
    env = os.environ.get("LIBDAY_CLI")
    if env:
        return Path(env)
    builds = [REPO / "target" / profile / name for profile in ("release", "debug")
              for name in ("libday.exe", "libday")]
    builds = [path for path in builds if path.is_file()]
    if builds:
        return max(builds, key=lambda path: path.stat().st_mtime)
    found = shutil.which("libday")
    return Path(found) if found else None


def archive_of(member: str) -> Path:
    for name in ("SCEN1.ZIP", "SCEN2.ZIP", "DEATHM.ZIP"):
        path = ROOT / "SCENS" / name
        if path.is_file() and member in zipfile.ZipFile(path).namelist():
            return path
    raise FileNotFoundError(member)


def engine_scene(cli: Path, spec: str, deployment: bool) -> list[tuple]:
    command = [str(cli), "--root", str(ROOT), "map-scene", spec]
    if deployment:
        command.append("--deployment")
    scene = json.loads(subprocess.run(command, check=True, capture_output=True, text=True).stdout)
    assert not scene["missing"], scene["missing"]
    size = scene["size"]
    items = []
    for item in scene["items"]:
        layer = item["layer"]
        prefix = ("" if layer == "terrain" else "building" if layer in ("building", "unit") else layer)
        left = PADDING + (size - 1 + item["x"] - item["y"]) * 27
        top = PADDING + (item["x"] + item["y"]) * 14
        items.append((prefix, item["sprite"], left, top))
    return items


def editor_scene(app: ui.MapEditor) -> list[tuple]:
    names = {str(image): key for key, image in app.images.items()}
    canvas = app.canvas
    items = []
    for item in canvas.find_all():
        if canvas.type(item) != "image":
            continue
        tags = canvas.gettags(item)
        if "scripting" in tags or "bridges" in tags:
            continue
        name, zoom = names[canvas.itemcget(item, "image")]
        assert zoom == "100%"
        prefix, _, sprite = name.rpartition(":")
        left, top = canvas.coords(item)
        items.append((prefix, sprite, round(left), round(top)))
    return items


@slow
class EngineParityTests(unittest.TestCase):
    MAPS = ("s1-99-1.mdb", "s1-5-1.mdb", "s3-4-1.mdb")

    def test_map_scene_matches_editor_canvas(self):
        cli = libday_cli()
        if cli is None:
            self.skipTest("build the libday CLI first (cargo build -p libday-tools)")
        root = tk.Tk()
        root.withdraw()
        try:
            app = ui.MapEditor(root, ROOT)
            self.assertEqual(app.zoom.get(), "100%")
            for member in self.MAPS:
                archive = archive_of(member)
                spec = f"{archive}!{member}"
                document = app.load_document(archive, member)
                try:
                    app.activate_document(document)
                    for deployment in (False, True):
                        with self.subTest(member=member, deployment=deployment):
                            app.show_deployment.set(deployment)
                            app.render_map()
                            expected = editor_scene(app)
                            actual = engine_scene(cli, spec, deployment)
                            self.assertEqual(len(actual), len(expected))
                            for index, (got, want) in enumerate(zip(actual, expected)):
                                self.assertEqual(got, want, f"draw item {index}")
                finally:
                    app.document = None
                    document.close()
        finally:
            root.destroy()


if __name__ == "__main__":
    unittest.main()
