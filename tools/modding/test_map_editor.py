import csv
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import struct
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch
import zipfile
import zlib

import map_editor_core as core


from testsupport import GREEN_SKIRMISH, ROOT, TOOLS, slow
TERRAINS = {
    "NORD000": core.Terrain("NORD000", 3, False),
    "BORE000": core.Terrain("BORE000", 3, False),
    "ARID000": core.Terrain("ARID000", 3, False),
    "NORD116": core.Terrain("NORD116", 5, True),
}


def sample_rows():
    return [{"Id": str(10 + y * 2 + x), "PosX": str(x), "PosY": str(y),
             "TerrainImg": "NORD116" if (x, y) == (1, 1) else "NORD000"}
            for y in range(2) for x in range(2)]


def sample_art(truncated=False):
    header = struct.pack("<4sIIIII", b"dbi\0", 3, 1, 54, 56, 1) + bytes(8)
    palette = bytes((0, 0, 255, 0)) + bytes(1020)
    if truncated:
        rows = (struct.pack("<IHH", 8, 54, 1) * 56)[:-3]
    else:
        rows = (struct.pack("<IHH", 62, 54, 2) + bytes(54)) * 56 + b"\0"
    record = (b"FAKE000\0" + struct.pack("<HHHHI", 54, 56, len(rows), 1, 0) + rows)
    return header + palette + record.ljust(8192, b"\0") + b"FAKE000\0" + struct.pack("<I", 0)


class ArtworkTests(unittest.TestCase):
    def test_palette_runs_and_png(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / "test.dbi"
            path.write_bytes(sample_art())
            art = core.TerrainArt(path)
            texture = art.texture("fake000")
            self.assertEqual(texture.rgba, bytes((255, 0, 0, 255)) * (54 * 56))
            png = texture.png()
            self.assertTrue(png.startswith(b"\x89PNG\r\n\x1a\n"))
            chunks = {}
            offset = 8
            while offset < len(png):
                size = struct.unpack_from(">I", png, offset)[0]
                name = png[offset + 4:offset + 8]
                data = png[offset + 8:offset + 8 + size]
                crc = struct.unpack_from(">I", png, offset + 8 + size)[0]
                self.assertEqual(crc, zlib.crc32(name + data) & 0xFFFFFFFF)
                chunks[name] = data
                offset += size + 12
            self.assertEqual(struct.unpack_from(">II", chunks[b"IHDR"]), (54, 56))
            self.assertEqual(zlib.decompress(chunks[b"IDAT"]),
                             (b"\0" + bytes((255, 0, 0, 255)) * 54) * 56)
            self.assertIs(art.texture("FAKE000"), texture)

    def test_truncated_final_transparent_run(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / "test.dbi"
            path.write_bytes(sample_art(truncated=True))
            self.assertEqual(core.TerrainArt(path).texture("FAKE000").rgba, bytes(54 * 56 * 4))

    def test_invalid_format_index_and_runs_are_rejected(self):
        with tempfile.TemporaryDirectory() as work:
            path = Path(work) / "test.dbi"
            for offset, replacement in [(0, b"bad\0"), (-4, struct.pack("<I", 10))]:
                data = bytearray(sample_art())
                actual = len(data) + offset if offset < 0 else offset
                data[actual:actual + len(replacement)] = replacement
                path.write_bytes(data)
                with self.assertRaises(ValueError):
                    core.TerrainArt(path)
            data = bytearray(sample_art())
            struct.pack_into("<H", data, 1056 + 20 + 6, 7)
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                core.TerrainArt(path).texture("FAKE000")

    def test_all_installed_artwork_and_registry_case(self):
        art = core.TerrainArt(ROOT / "IMGS" / "TERRAINS.DBI")
        terrains = core.load_terrains(TOOLS / "terrain-types.csv")
        self.assertTrue(set(terrains) <= set(art.records))
        self.assertEqual(len(art.records), 1119)
        for name in art.records:
            texture = art.texture(name)
            self.assertEqual(len(texture.rgba), 54 * 56 * 4, name)
        self.assertEqual(sum(terrain.paintable for terrain in terrains.values()), 355)
        for name in ("BORE000", "NORD000"):
            texture = art.texture(name)
            self.assertEqual(set(texture.rgba[3::4]), {0, 255})
            self.assertEqual(texture.rgba[:54 * 28 * 4], bytes(54 * 28 * 4))
        self.assertNotEqual(art.texture("BORE000").rgba, art.texture("NORD000").rgba)


class AlternateMapTests(unittest.TestCase):
    def test_installed_campaign_pairs_and_unpaired_maps(self):
        normal, alternate = ROOT / "SCENS" / "SCEN1.ZIP", ROOT / "SCENS" / "SCEN2.ZIP"
        retry_members = core.archive_members(alternate)
        self.assertEqual(len(retry_members), 50)
        for member in retry_members:
            with self.subTest(member=member):
                first = member[:-5] + "1.mdb"
                self.assertEqual(core.campaign_map_variant(alternate, member), 2)
                self.assertEqual(core.alternate_map_source(alternate, member), (normal, first))
                self.assertEqual(core.alternate_map_source(normal, first), (alternate, member))
        for member in ("s1-1-1.mdb", "s2-1-1.mdb", "s3-1-1.mdb", "s4-1-1.mdb", "s1-99-1.mdb"):
            self.assertIsNone(core.alternate_map_source(normal, member))
        self.assertIsNone(core.campaign_map_variant(Path("edited-map.mdb")))
        self.assertIsNone(core.alternate_map_source(ROOT / "SCENS" / "DEATHM.ZIP", "dm-1.mdb"))

    def test_local_mdb_and_same_archive_pairs_case_handling_and_missing_variants(self):
        with tempfile.TemporaryDirectory() as work:
            folder = Path(work)
            normal, alternate = folder / "s1-2-1.mdb", folder / "s1-2-2.mdb"
            normal.write_bytes(b"normal")
            self.assertIsNone(core.alternate_map_source(normal))
            alternate.write_bytes(b"alternate")
            self.assertEqual(core.alternate_map_source(normal), (alternate, None))
            self.assertEqual(core.alternate_map_source(alternate), (normal, None))
            self.assertEqual(core.campaign_map_variant(folder / "S1-2-2.MDB"), 2)
            archive = folder / "both.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("S1-2-1.MDB", b"normal")
                output.writestr("S1-2-2.MDB", b"alternate")
            self.assertEqual(core.alternate_map_source(archive, "S1-2-1.MDB"), (archive, "S1-2-2.MDB"))
            self.assertEqual(core.alternate_map_source(archive, "S1-2-2.MDB"), (archive, "S1-2-1.MDB"))
            archive = folder / "copy.zip"
            with zipfile.ZipFile(archive, "w") as output:
                output.writestr("s1-2-1.mdb", b"normal")
            self.assertIsNone(core.alternate_map_source(archive, "s1-2-1.mdb"),
                              "Do not silently pair a local copy with another installed game/archive.")
            peer = folder / "SCEN2.ZIP"
            with zipfile.ZipFile(peer, "w") as output:
                output.writestr("S1-2-2.MDB", b"alternate")
            self.assertEqual(core.alternate_map_source(archive, "s1-2-1.mdb"), (peer, "S1-2-2.MDB"))
            peer.write_bytes(b"not a zip")
            with self.assertRaises(zipfile.BadZipFile):
                core.alternate_map_source(archive, "s1-2-1.mdb")


class DeploymentTests(unittest.TestCase):
    @staticmethod
    def model(objects=None, size=5):
        return core.MapModel(
            [{"Id": str(y * size + x), "PosX": str(x), "PosY": str(y), "TerrainImg": "NORD000"}
             for y in range(size) for x in range(size)], TERRAINS, objects=objects)

    def test_four_players_grouped_strokes_transfer_erase_and_saved_state(self):
        original = core.Deployment(30, 1, 0, 0)
        model = self.model({"deployments": [original.row()],
                            "blocked": [{"x": 0, "y": 0, "reason": "Deployment"}]})
        before = model.state()
        model.begin_stroke()
        self.assertFalse(model.paint_deployment(0, 0, 1))
        for player in range(1, 5):
            self.assertTrue(model.paint_deployment(player, 0, player))
        model.paint_deployment(0, 0, 4)
        model.end_stroke()
        self.assertEqual(len(model.undo_stack), 1)
        self.assertEqual(model.deployment_at((0, 0)).counter, 30)
        self.assertEqual(model.deployment_at((0, 0)).player, 4)
        edited = model.state()
        self.assertTrue(model.dirty)
        self.assertEqual(model.undo(), {(x, 0) for x in range(5)})
        self.assertEqual(model.state(), before)
        self.assertFalse(model.dirty)
        self.assertNotIn((1, 0), model.blocked)
        model.redo()
        self.assertEqual(model.state(), edited)
        model.mark_saved()
        model.begin_stroke()
        self.assertFalse(model.paint_deployment(0, 0, 1, erase=True))
        self.assertTrue(model.paint_deployment(0, 0, 4, erase=True))
        model.end_stroke()
        self.assertNotIn((0, 0), model.blocked)
        self.assertTrue(model.dirty)
        model.undo()
        self.assertFalse(model.dirty)
        self.assertEqual(model.blocked[0, 0], "Deployment")
        model.begin_stroke()
        model.paint_deployment(0, 0, 2)
        model.paint_deployment(0, 0, 4)
        model.end_stroke()
        self.assertFalse(model.dirty)
        self.assertTrue(model.redo_stack)

    def test_zone_geometry_is_independent_of_terrain_and_occupancy(self):
        model = ModelTests().object_model({"blocked": [{"x": 3, "y": 3, "reason": "Mines On Map"}]})
        building = model.add_building(480, 0, 0, 0)
        model.cells[2, 2].terrain = "NORD116"
        model.terrains = {**model.terrains, "WATER000": core.Terrain("WATER000", 0, False)}
        model.cells[3, 2].terrain = "WATER000"
        before = model.state()
        model.begin_stroke()
        for key in ((0, 0), (1, 1), (2, 2), (3, 2), (3, 3)):
            self.assertTrue(model.paint_deployment(*key, 3))
        model.end_stroke()
        self.assertEqual(model.buildings[building.identifier], building)
        self.assertEqual(model.cells[2, 2].terrain, "NORD116")
        self.assertEqual(model.cells[3, 2].terrain, "WATER000")
        model.undo()
        self.assertEqual(model.state(), before)

    def test_preserved_rows_duplicate_loading_and_paint_validation(self):
        future = core.Deployment(40, 2, 3, 3, 7)
        ai = core.Deployment(41, 0, 3, 4)
        other = core.Deployment(42, 8, 4, 4, 123)
        first, duplicate = core.Deployment(1, 1, 0, 0), core.Deployment(2, 4, 0, 0)
        model = self.model({"deployments": [item.row() for item in (first, duplicate, future, ai, other)]})
        before = model.state()
        self.assertEqual(model.deployment_at((0, 0)), first)
        with self.assertRaises(ValueError):
            model.paint_deployment(0, 0, 1)
        model.begin_stroke()
        for player in (0, 5, -1, True, "1"):
            with self.assertRaises(ValueError):
                model.paint_deployment(1, 1, player)
        with self.assertRaises(ValueError):
            model.paint_deployment(5, 0, 1)
        for key in ((3, 3), (3, 4), (4, 4)):
            self.assertIsNone(model.deployment_at(key))
            with self.assertRaisesRegex(ValueError, "preserved"):
                model.paint_deployment(*key, 1)
            self.assertFalse(model.paint_deployment(*key, 2, erase=True))
        model.paint_deployment(0, 0, 3)
        model.end_stroke()
        self.assertEqual(len(model.deployment_positions[0, 0]), 1)
        for item in (future, ai, other):
            self.assertEqual(model.deployments[item.counter], item)
        model.undo()
        self.assertEqual(model.state(), before)
        model.redo()
        self.assertEqual(model.deployment_at((0, 0)).player, 3)

    def test_native_connection_shapes_and_player_separation(self):
        expected = (0, 13, 15, 14, 10, 1, 7, 4, 12, 3, 9, 6, 11, 2, 8, 5)
        art = core.TerrainArt(ROOT / "IMGS" / "SYMBOLS.DBI")
        colors = []
        for player in range(1, 5):
            colors.append(art.texture(f"DEPL{player}00").rgba)
            for mask in range(16):
                with self.subTest(player=player, mask=mask):
                    model = self.model()
                    model.begin_stroke()
                    model.paint_deployment(2, 2, player)
                    for dx, dy, bit in core.NEIGHBORS:
                        model.paint_deployment(2 + dx, 2 + dy, player if mask & bit else player % 4 + 1)
                    model.end_stroke()
                    sprite = model.deployment_sprite(model.deployment_at((2, 2)))
                    self.assertEqual(sprite, f"DEPL{player}{expected[mask]:02d}")
                    self.assertEqual(len(art.texture(sprite).rgba), 54 * 56 * 4)
        self.assertEqual(len(set(colors)), 4)

    def test_mixed_layer_history_and_empty_noop_strokes(self):
        model = self.model()
        before = model.state()
        model.begin_stroke()
        model.paint(0, 0, "BORE000")
        model.paint_surface("roads", 1, 0, "0")
        model.paint_deployment(0, 0, 1)
        model.paint_deployment(2, 0, 2)
        model.end_stroke()
        edited = model.state()
        self.assertEqual(len(model.undo_stack), 1)
        model.undo()
        self.assertEqual(model.state(), before)
        model.begin_stroke()
        model.paint_deployment(3, 0, 3)
        model.paint_deployment(3, 0, 3, erase=True)
        model.end_stroke()
        self.assertEqual(model.state(), before)
        self.assertFalse(model.undo_stack)
        model.redo()
        self.assertEqual(model.state(), edited)

    def test_units_walls_decors_and_roads_follow_dynamic_zone_reservations(self):
        model = self.model()
        model.begin_stroke()
        for key in ((0, 0), (1, 1), (2, 2)):
            model.paint_deployment(*key, 1)
        model.end_stroke()
        with self.assertRaisesRegex(ValueError, "Deployment"):
            model.add_unit(4, 0, 0)
        model.begin_stroke()
        with self.assertRaisesRegex(ValueError, "Deployment"):
            model.paint_surface("walls", 1, 1, "0")
        with self.assertRaisesRegex(ValueError, "Deployment"):
            model.paint_surface("decors", 2, 2, "BBRUI000")
        self.assertTrue(model.paint_surface("roads", 0, 0, "0"))
        model.end_stroke()
        model.begin_stroke()
        for key in ((0, 0), (1, 1), (2, 2)):
            model.paint_deployment(*key, 1, erase=True)
        model.end_stroke()
        self.assertIsNotNone(model.add_unit(4, 0, 0))
        model.begin_stroke()
        self.assertTrue(model.paint_surface("walls", 1, 1, "0"))
        self.assertTrue(model.paint_surface("decors", 2, 2, "BBRUI000"))
        model.end_stroke()

    def test_ui_hidden_default_native_overlay_brush_picking_and_history(self):
        import map_editor as ui
        import tkinter as tk

        root = tk.Tk()
        root.withdraw()
        editor = ui.MapEditor(root, ROOT)
        model = self.model({"deployments": [core.Deployment(1, 1, 0, 0).row()]})
        editor.document = SimpleNamespace(model=model, source=Path("test.mdb"), member=None)
        try:
            editor.render_map()
            self.assertFalse(editor.show_deployment.get())
            self.assertFalse(editor.canvas.find_withtag("deployment"))
            editor.mode.set("Deployment")
            editor.change_mode()
            self.assertTrue(editor.show_deployment.get())
            self.assertEqual(editor.palette_names, ["1", "2", "3", "4"])
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 1)
            editor.deployment_player.set("4")
            editor.deployment_player_changed()
            editor.brush.set("3x3")
            editor.event_cell = Mock(return_value=(2, 2))
            event = SimpleNamespace(x=0, y=0)
            editor.paint_start(event)
            editor.paint_end()
            self.assertEqual(len(model.undo_stack), 1)
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 10)
            self.assertIn("P4: 9", editor.deployment_counts.get())
            editor.deployment_player.set("2")
            editor.eyedropper(event)
            self.assertEqual(editor.deployment_player.get(), "4")
            editor.deployment_action.set("Erase")
            editor.paint_start(event)
            editor.paint_end()
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 1)
            editor.undo()
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 10)
            for zoom, (multiply, divide) in ui.ZOOMS.items():
                editor.zoom.set(zoom)
                editor.render_map()
                image = editor.image("DEPL400", zoom, layer="deployment")
                self.assertEqual(image.width(), (54 * multiply + divide - 1) // divide)
                self.assertEqual(image.height(), (56 * multiply + divide - 1) // divide)
                marker = editor.canvas.find_withtag("deployment")[0]
                self.assertEqual(editor.canvas.coords(marker),
                                 [editor.center(0, 0)[0] - 27 * editor.scale,
                                  editor.center(0, 0)[1] - 42 * editor.scale])
            editor.mode.set("Ground")
            editor.change_mode()
            self.assertFalse(editor.show_deployment.get())
            self.assertFalse(editor.canvas.find_withtag("deployment"))
            editor.show_deployment.set(True)
            editor.toggle_deployment()
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 10)
            editor.mode.set("Deployment")
            editor.change_mode()
            editor.undo()
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 1)
            editor.redo()
            self.assertEqual(len(editor.canvas.find_withtag("deployment")), 10)
        finally:
            editor.executor.shutdown(wait=True)
            root.update_idletasks()
            root.destroy()


class ScenarioTests(unittest.TestCase):
    def model(self):
        settings = core.ScenarioSettings("A scenario", "Briefing##Objective", 10, 150,
                                         ((2, True), (8, False)))
        return core.MapModel(sample_rows(), TERRAINS, objects={"scenario": settings.row()})

    def test_settings_share_history_dirty_state_and_noop_behavior(self):
        model = self.model()
        old = model.scenario
        settings = replace(old, title="Changed", turns=15, points=420,
                           available=((2, True), (8, True)))
        model.begin_stroke()
        model.paint(0, 0, "BORE000")
        model.update_scenario(settings)
        self.assertEqual(len(model.undo_stack), 2)
        self.assertTrue(model.dirty)
        self.assertIsNone(model.stroke)
        model.undo()
        self.assertEqual(model.scenario, old)
        self.assertEqual(model.cells[0, 0].terrain, "BORE000")
        model.undo()
        self.assertFalse(model.dirty)
        model.redo()
        model.redo()
        self.assertEqual(model.scenario, settings)
        model.mark_saved()
        model.update_scenario(settings)
        self.assertFalse(model.dirty)
        self.assertEqual(len(model.undo_stack), 2)

    def test_invalid_settings_and_roster_removal_are_rejected(self):
        model = self.model()
        old = model.scenario
        for fields in ({"title": ""}, {"title": " "}, {"title": "x" * 51}, {"briefing": "\0"},
                       {"turns": 0}, {"turns": 101}, {"turns": True}, {"points": -1},
                       {"points": 32768}, {"points": 5.5}, {"available": ((2, 1), (8, False))},
                       {"available": ((2, True), (2, False))},
                       {"available": ((2, True), (8, False), (40, True))},
                       {"available": ((2, True),)}):
            with self.subTest(fields=fields), self.assertRaises(ValueError):
                model.update_scenario(replace(old, **fields))
            self.assertFalse(model.dirty)
        self.assertEqual(len(core.skirmish_unit_types(model.unit_types)), 39)

    def test_difficulty_exact_native_boundaries(self):
        for points, name in ((0, "Easy"), (300, "Easy"), (301, "Average"), (500, "Average"),
                             (501, "Hard"), (1000, "Hard"), (1001, "Very hard"), (32767, "Very hard")):
            self.assertEqual(core.skirmish_difficulty(points), name)
        for name, points in core.SKIRMISH_DIFFICULTIES.items():
            self.assertEqual(core.skirmish_difficulty(points), name)

    def test_scenario_dialog_selection_filtering_presets_validation_and_apply(self):
        import map_editor as ui
        import tkinter as tk

        root = tk.Tk()
        root.withdraw()
        model = self.model()
        editor = SimpleNamespace(root=root, document=SimpleNamespace(model=model), unit_types=model.unit_types,
                                 script_dialog=None,
                                 update_controls=Mock(), status=tk.StringVar(), export_skirmish=Mock(),
                                 unit_label=lambda kind: f"{model.unit_types[kind, 0].name} ({kind})")
        dialog = ui.ScenarioDialog(editor)
        dialog.withdraw()
        try:
            self.assertEqual(dialog.settings(), model.scenario)
            self.assertEqual(len(dialog.visible), 39)
            dialog.search.set("LAV")
            self.assertEqual(dialog.visible, [8])
            dialog.roster.selection_set(0)
            dialog.remember_selection()
            dialog.search.set("Trooper (2)")
            self.assertEqual(dialog.visible, [2])
            self.assertIn(8, dialog.enabled)
            dialog.roster.selection_clear(0)
            dialog.remember_selection()
            dialog.search.set("")
            self.assertEqual(dialog.enabled, {8})
            dialog.difficulty.set("Hard")
            dialog.choose_difficulty()
            self.assertEqual(dialog.points.get(), "750")
            dialog.points.set("300")
            self.assertEqual(dialog.difficulty.get(), "Easy")
            dialog.points.set("301")
            self.assertEqual(dialog.difficulty.get(), "Average")
            dialog.points.set("1.5")
            with patch.object(ui.messagebox, "showerror") as error:
                dialog.accept()
                error.assert_called_once()
            self.assertFalse(model.dirty)
            dialog.points.set("420")
            dialog.name.set("A new title")
            dialog.turns.set("15")
            dialog.briefing.delete("1.0", "end")
            dialog.briefing.insert("1.0", "Briefing\nObjective: defend")
            dialog.accept(export=True)
            editor.export_skirmish.assert_called_once()
            self.assertEqual(model.scenario.title, "A new title")
            self.assertEqual(model.scenario.briefing, "Briefing##Objective: defend")
            self.assertEqual(model.scenario.available, ((2, False), (8, True)))
            self.assertTrue(model.dirty)
            model.undo()
            self.assertFalse(model.dirty)
        finally:
            if dialog.winfo_exists():
                dialog.destroy()
            root.update_idletasks()
            root.destroy()

class UnitTests(unittest.TestCase):
    def model(self, objects=None):
        rows = [{"Id": str(y * 8 + x), "PosX": str(x), "PosY": str(y), "TerrainImg": "NORD000"}
                for y in range(8) for x in range(8)]
        return core.MapModel(rows, dict(TERRAINS), core.load_building_types(TOOLS / "building-types.csv"),
                             objects or {"nextUnitId": 200})

    def test_registry_zero_based_types_technology_and_all_static_artwork(self):
        types = core.load_unit_types(TOOLS / "unit-types.csv")
        self.assertEqual(len(types), 217)
        self.assertEqual(sum(tech == 0 for _, tech in types), 62)
        self.assertEqual(types[4, 0].name, "Footman")
        self.assertEqual(types[0, 0].name, "Commando")
        self.assertNotEqual(types[0, 0].ap, types[0, 4].ap)
        art = core.TerrainArt(ROOT / "IMGS" / "UB.DBI")
        for (identifier, tech), kind in types.items():
            if tech:
                continue
            frame = next(frame for frame in range(10)
                         if all(kind.sprite(x, y, face, frame) in art.records
                                for face in range(8 if kind.facings else 1)
                                for x in range(kind.width) for y in range(kind.height)))
            for facing in range(8):
                for x in range(kind.width):
                    for y in range(kind.height):
                        self.assertEqual(len(art.texture(kind.sprite(x, y, facing, frame)).rgba), 54 * 56 * 4)

    def test_initial_and_reinforcement_placement_turn_occupancy_and_history(self):
        model = self.model()
        before = model.state()
        start = model.add_unit(2, 0, 0, owner=1, facing=7, tech=4)
        later = model.add_unit(4, 0, 0, arrival=3, arrival_order=True)
        self.assertEqual(start.identifier, 200)
        self.assertEqual(later.identifier, 201)
        self.assertEqual(model.units_at((0, 0), 0), [start])
        self.assertEqual(model.units_at((0, 0), 3), [later])
        with self.assertRaisesRegex(ValueError, "same arrival"):
            model.add_unit(4, 0, 0, arrival=3)
        with self.assertRaisesRegex(ValueError, "owner 0"):
            model.add_unit(4, 2, 0, arrival=3, owner=1)
        with self.assertRaisesRegex(ValueError, "assigned by the game"):
            model.add_unit(2, 2, 0, arrival=3, tech=4)
        for invalid in (-1, 101):
            with self.assertRaises(ValueError):
                model.add_unit(4, 2, 0, arrival=invalid)
        model.update_unit(later.identifier, x=1, y=0, arrival=100)
        model.delete_unit(start.identifier)
        self.assertNotIn(start.identifier, model.units)
        for _ in range(4):
            model.undo()
        self.assertEqual(model.state(), before)
        for _ in range(4):
            model.redo()
        self.assertEqual(model.units[later.identifier].arrival, 100)

    def test_native_ai_turn_is_not_arrival_and_original_counters_survive_conversions(self):
        unit = core.Unit(100, 4, 0, 0, armor=10, ap=120, order_id=1, order_turn=2,
                         counter=572, origin_layer="units")
        model = self.model({"units": [unit.row()], "reinforcements": [], "nextUnitId": 200})
        self.assertEqual(model.units[100].arrival, 0)
        self.assertEqual(model.units[100].order_turn, 2)
        later = model.update_unit(100, arrival=3)
        self.assertEqual(later.row()["Counter"], "")
        self.assertEqual(later.row()["turn"], "3")
        self.assertNotIn("playerOwner", later.row())
        restored = model.update_unit(100, arrival=0)
        self.assertEqual(restored.row(), unit.row())
        self.assertEqual(restored.row()["Turn"], "2")
        with self.assertRaisesRegex(ValueError, "counters"):
            model.update_unit(100, counter=1)

    def test_references_and_unit_id_bounds(self):
        unit = core.Unit(100, 4, 0, 0, armor=10, ap=120, counter=1, origin_layer="units")
        model = self.model({"units": [unit.row()], "unitReferences": [100], "nextUnitId": 999999})
        with self.assertRaisesRegex(ValueError, "reference"):
            model.delete_unit(100)
        with self.assertRaisesRegex(ValueError, "reference"):
            model.update_unit(100, arrival=3)
        last = model.add_unit(4, 1, 0)
        self.assertEqual(last.identifier, 999999)
        with self.assertRaisesRegex(ValueError, "exhausted"):
            model.add_unit(4, 2, 0)
        model.undo()
        self.assertEqual(model.next_unit_id, 999999)

    def test_multi_tile_units_forests_water_roads_and_dynamic_protection(self):
        model = self.model()
        model.cells[0, 0].terrain = "NORD116"
        infantry = model.add_unit(2, 0, 0)
        with self.assertRaises(ValueError):
            model.add_unit(9, 0, 0, arrival=3)
        model.delete_unit(infantry.identifier)
        model.cells[0, 0].terrain = "NORD000"
        outpost = model.add_unit(47, 0, 0)
        self.assertEqual(len(model.unit_footprint(outpost)), 9)
        self.assertIn((2, 2), model.blocked)
        with self.assertRaises(ValueError):
            model.add_building(480, 0, 2, 2)
        model.begin_stroke()
        model.paint_surface("roads", 2, 2, "0")
        with self.assertRaises(ValueError):
            model.paint_surface("walls", 1, 1, "0")
        with self.assertRaises(ValueError):
            model.paint(2, 2, "NORD116", "trees")
        model.end_stroke()
        model.delete_unit(outpost.identifier)
        self.assertNotIn((1, 1), model.blocked)
        model.terrains["WATER"] = core.Terrain("WATER", 1, False)
        model.cells[7, 7].terrain = "WATER"
        model.add_unit(28, 7, 7)
        with self.assertRaises(ValueError):
            model.add_unit(4, 7, 7, arrival=3)
        model.add_unit(30, 7, 7, arrival=3)


class ModelTests(unittest.TestCase):
    def object_model(self, objects=None):
        rows = [{"Id": str(y * 4 + x), "PosX": str(x), "PosY": str(y), "TerrainImg": "NORD000"}
                for y in range(4) for x in range(4)]
        rows[-1]["TerrainImg"] = "NORD116"
        return core.MapModel(rows, TERRAINS, core.load_building_types(TOOLS / "building-types.csv"),
                             objects or {"nextBuildingId": 1000005})

    def test_tree_plant_replace_erase_and_dynamic_protection(self):
        model = self.object_model()
        original = model.state()
        model.begin_stroke()
        model.paint(0, 0, "NORD116", "trees")
        self.assertIn((0, 0), model.protected)
        with self.assertRaises(ValueError):
            model.paint(0, 0, "BORE000")
        model.end_stroke()
        model.begin_stroke()
        self.assertTrue(model.paint(0, 0, "BORE000", "erase"))
        self.assertFalse(model.paint(0, 1, "BORE000", "erase"))
        model.end_stroke()
        self.assertNotIn((0, 0), model.protected)
        model.undo()
        self.assertEqual(model.cells[0, 0].terrain, "NORD116")
        model.undo()
        self.assertEqual(model.state(), original)
        self.assertFalse(model.dirty)
        model.redo()
        model.redo()
        self.assertEqual(model.cells[0, 0].terrain, "BORE000")

    def test_building_full_editing_history_and_stable_ids(self):
        model = self.object_model()
        original = model.state()
        building = model.add_building(480, 0, 0, 0)
        self.assertEqual(building.identifier, 1000005)
        self.assertEqual(model.next_building_id, 1000006)
        self.assertIs(model.building_at((1, 1)), building)
        moved = model.update_building(building.identifier, x=1, y=0)
        self.assertEqual(moved.identifier, building.identifier)
        changed = model.update_building(building.identifier, kind=100, owner=2, armor=7, ai_target=True)
        self.assertEqual(changed.counter, building.counter)
        self.assertEqual(changed.owner, 2)
        self.assertEqual(changed.armor, 7)
        model.mark_saved()
        model.references.add(building.identifier)
        with self.assertRaisesRegex(ValueError, "referenced"):
            model.delete_building(building.identifier)
        model.references.clear()
        model.delete_building(building.identifier)
        self.assertFalse(model.buildings)
        self.assertTrue(model.dirty)
        model.undo()
        self.assertFalse(model.dirty)
        for _ in range(3):
            model.undo()
        self.assertEqual(model.state(), original)
        for _ in range(4):
            model.redo()
        self.assertFalse(model.buildings)

    def test_bounds_terrain_collision_unique_and_invalid_properties(self):
        model = self.object_model({"blocked": [{"x": 0, "y": 0, "reason": "unit"}]})
        for x, y in ((0, 0), (3, 0), (2, 2), (-1, 0)):
            with self.assertRaises(ValueError):
                model.add_building(480, 0, x, y)
        for owner, armor in ((5, 40), (0, 0), (0, 41)):
            with self.assertRaises(ValueError):
                model.add_building(480, owner, 1, 0, armor)
        building = model.add_building(480, 0, 1, 0)
        with self.assertRaises(ValueError):
            model.add_building(480, 0, 2, 0)
        model.building_types[100] = replace(model.building_types[100], unique=True)
        model.add_building(100, 0, 0, 2)
        with self.assertRaisesRegex(ValueError, "unique"):
            model.add_building(100, 1, 0, 3)
        with self.assertRaises(ValueError):
            model.update_building(building.identifier, counter=22)
        model.begin_stroke()
        with self.assertRaisesRegex(ValueError, "reserved"):
            model.paint(0, 0, "NORD116", "trees")
        with self.assertRaisesRegex(ValueError, "footprint"):
            model.paint(1, 0, "NORD116", "trees")
        model.end_stroke()

    def test_mixed_undo_and_id_allocation_avoid_references(self):
        model = self.object_model({"nextBuildingId": 1000000, "reservedIds": [1000007]})
        original = model.state()
        model.begin_stroke()
        model.paint(0, 0, "NORD116", "trees")
        model.end_stroke()
        building = model.add_building(480, 0, 1, 1)
        self.assertEqual(building.identifier, 1000008)
        model.undo()
        self.assertEqual(model.cells[0, 0].terrain, "NORD116")
        model.undo()
        self.assertEqual(model.state(), original)
        self.assertFalse(model.dirty)

    def test_strokes_undo_redo_and_saved_checkpoint(self):
        model = core.MapModel(sample_rows(), TERRAINS)
        original = model.rows()
        model.begin_stroke()
        self.assertTrue(model.paint(0, 0, "BORE000"))
        self.assertTrue(model.paint(1, 0, "BORE000"))
        self.assertFalse(model.paint(1, 0, "BORE000"))
        model.end_stroke()
        self.assertEqual(len(model.undo_stack), 1)
        self.assertTrue(model.dirty)
        self.assertEqual(model.undo(), {(0, 0), (1, 0)})
        self.assertEqual(model.rows(), original)
        self.assertFalse(model.dirty)
        model.redo()
        model.mark_saved()
        self.assertFalse(model.dirty)
        model.undo()
        self.assertTrue(model.dirty)
        model.redo()
        self.assertFalse(model.dirty)
        model.undo()
        model.begin_stroke()
        model.paint(0, 1, "ARID000")
        model.end_stroke()
        self.assertFalse(model.redo_stack)

    def test_noop_stroke_and_case_preservation(self):
        rows = sample_rows()
        rows[0]["TerrainImg"] = "Nord000"
        model = core.MapModel(rows, TERRAINS)
        model.begin_stroke()
        self.assertFalse(model.paint(0, 0, "NORD000"))
        model.end_stroke()
        self.assertEqual(model.rows()[0]["TerrainImg"], "Nord000")
        self.assertFalse(model.dirty)
        self.assertFalse(model.undo_stack)
        model.begin_stroke()
        model.paint(0, 0, "BORE000")
        model.paint(0, 0, "NORD000")
        model.end_stroke()
        self.assertFalse(model.dirty)

    def test_protected_terrain_invalid_brush_and_grid(self):
        model = core.MapModel(sample_rows(), TERRAINS)
        with self.assertRaises(ValueError):
            model.paint(0, 0, "BORE000")
        model.begin_stroke()
        for x, y, terrain in [(1, 1, "BORE000"), (0, 0, "NORD116"), (0, 0, "UNKNOWN"),
                              (-1, 0, "BORE000")]:
            with self.assertRaises(ValueError):
                model.paint(x, y, terrain)
        self.assertFalse(model.dirty)
        variants = [sample_rows()[:-1], sample_rows() + sample_rows()]
        duplicate = sample_rows()
        duplicate[1]["Id"] = duplicate[0]["Id"]
        variants.append(duplicate)
        moved = sample_rows()
        moved[1]["PosX"] = "2"
        variants.append(moved)
        for rows in variants:
            with self.assertRaises(ValueError):
                core.MapModel(rows, TERRAINS)

    def test_hit_testing_at_every_zoom_and_drag_interpolation(self):
        for scale in (0.5, 1, 2, 3):
            for x in range(24):
                for y in range(24):
                    px = 800 + (x - y) * 27 * scale
                    py = 80 + (x + y) * 14 * scale
                    self.assertEqual(core.tile_at(px, py, 800, 80, scale, 24), (x, y))
                    self.assertEqual(core.tile_at(px + 3 * scale, py + 2 * scale,
                                                  800, 80, scale, 24), (x, y))
            self.assertIsNone(core.tile_at(800, 80 - 40 * scale, 800, 80, scale, 24))
        self.assertEqual(core.line_cells((0, 0), (5, 0)), [(x, 0) for x in range(6)])
        self.assertEqual(core.line_cells((0, 0), (3, 3)), [(x, x) for x in range(4)])
        self.assertEqual(core.line_cells((0, 0), (3, 3), cardinal=True),
                         [(0, 0), (1, 0), (1, 1), (2, 1), (2, 2), (3, 2), (3, 3)])
        self.assertEqual(core.line_cells((1, 2), (1, 2)), [(1, 2)])


class SurfaceTests(unittest.TestCase):
    def model(self, objects=None):
        terrains = dict(TERRAINS, ROCK000=core.Terrain("ROCK000", 4, True),
                        DEEP000=core.Terrain("DEEP000", 1, False),
                        BANK000=core.Terrain("BANK000", 2, False))
        rows = [{"Id": str(y * 5 + x), "PosX": str(x), "PosY": str(y), "TerrainImg": "NORD000"}
                for y in range(5) for x in range(5)]
        return core.MapModel(rows, terrains, core.load_building_types(TOOLS / "building-types.csv"),
                             objects)

    def test_all_connection_masks_type_boundaries_and_owner_independence(self):
        expected = (0, 9, 10, 7, 8, 13, 12, 2, 11, 15, 14, 5, 6, 3, 4, 1)
        for layer in ("roads", "walls"):
            for mask, shape in enumerate(expected):
                model = self.model()
                model.begin_stroke()
                model.paint_surface(layer, 2, 2, "0")
                for dx, dy, bit in core.NEIGHBORS:
                    if mask & bit:
                        model.paint_surface(layer, 2 + dx, 2 + dy, "0", owner=4)
                model.end_stroke()
                item = model.surface_at(layer, (2, 2))
                self.assertEqual(model.connection_sprite(layer, item), shape, (layer, mask))
                name = f"ROALG{shape:03d}" if layer == "roads" else f"WALLC0{shape:02d}"
                self.assertEqual(model.surface_sprite(layer, item), name)
                if mask & 8:
                    neighbor = model.surface_at(layer, (1, 2))
                    model.update_surface(layer, neighbor.counter, kind="1",
                                         **({"armor": 150} if layer == "walls" else {}))
                    self.assertEqual(model.connection_sprite(layer, item), expected[mask & ~8])
                    model.undo()
                    self.assertEqual(model.connection_sprite(layer, item), shape)

    def test_scenery_mixed_stroke_history_and_dynamic_occupancy(self):
        model = self.model({"blocked": [{"x": 4, "y": 4, "reason": "unit"}]})
        original = model.state()
        model.begin_stroke()
        model.paint_surface("roads", 0, 0, "0")
        model.paint_surface("walls", 1, 0, "0")
        model.paint_surface("decors", 2, 0, "BBRUI000")
        model.paint(3, 0, "ROCK000", "scenery")
        model.end_stroke()
        self.assertEqual(len(model.undo_stack), 1)
        self.assertEqual(model.blocked[0, 0], "Roads On Map")
        model.undo()
        self.assertEqual(model.state(), original)
        self.assertFalse(model.dirty)
        model.redo()
        model.begin_stroke()
        with self.assertRaisesRegex(ValueError, "reserved"):
            model.paint(0, 0, "ROCK000", "scenery")
        with self.assertRaises(ValueError):
            model.paint(3, 0, "BORE000")
        self.assertTrue(model.paint(3, 0, "BORE000", "erase_scenery"))
        self.assertFalse(model.paint(3, 1, "BORE000", "erase_scenery"))
        self.assertTrue(model.paint_surface("roads", 0, 0, "0", erase=True))
        self.assertNotIn((0, 0), model.blocked)
        model.paint(0, 0, "NORD116", "trees")
        with self.assertRaisesRegex(ValueError, "reserved"):
            model.paint_surface("walls", 4, 4, "0")
        self.assertTrue(model.paint_surface("roads", 4, 4, "0"))
        model.end_stroke()
        self.assertEqual(model.blocked[4, 4], "Roads On Map")
        model.undo()
        self.assertEqual(model.blocked[4, 4], "unit")
        self.assertEqual(model.cells[0, 0].terrain, "NORD000")

    def test_stable_counters_move_properties_delete_and_duplicate_decor_preservation(self):
        objects = {
            "roads": [{"Counter": 88, "roadtype": 0, "playerOwner": 0, "PosX": 0, "PosY": 0}],
            "decors": [{"Counter": counter, "id": name, "PosX": 4, "PosY": 0}
                       for counter, name in ((90, "UBRUI000"), (91, "UBRUIXXX"))],
        }
        model = self.model(objects)
        original = model.state()
        road = model.update_surface("roads", 88, x=1, y=1, owner=2, kind="1")
        self.assertEqual(road.counter, 88)
        self.assertNotIn((0, 0), model.blocked)
        self.assertEqual(model.surface_rows()["roads"][0]["Counter"], "88")
        model.delete_surface("decors", 91)
        self.assertEqual(set(model.surfaces["decors"]), {90})
        model.undo()
        model.undo()
        self.assertEqual(model.state(), original)
        model.begin_stroke()
        model.paint_surface("roads", 0, 0, "0", owner=0)
        model.end_stroke()
        self.assertFalse(model.undo_stack)
        self.assertFalse(model.dirty)
        model.begin_stroke()
        model.paint_surface("roads", 2, 2, "0")
        model.paint_surface("roads", 2, 2, "0", erase=True)
        model.end_stroke()
        self.assertFalse(model.dirty)
        self.assertFalse(model.undo_stack)
        with self.assertRaises(TypeError):
            model.update_surface("roads", 88, counter=7)

    def test_low_hp_walls_show_intact_artwork_and_preserve_broken_concrete_caps(self):
        model = self.model()
        model.begin_stroke()
        model.paint_surface("walls", 2, 2, "0")
        model.paint_surface("walls", 3, 2, "0")
        model.end_stroke()
        item = model.surface_at("walls", (2, 2))
        self.assertEqual(model.surface_sprite("walls", item), "WALLC008")
        item = model.update_surface("walls", item.counter, armor=19)
        self.assertEqual(model.surface_sprite("walls", item), "WALLC008")
        self.assertEqual(item.armor, 19)
        item = model.update_surface("walls", item.counter, armor=20, broken=True)
        self.assertEqual(model.surface_sprite("walls", item), "WALLC019")
        item = model.update_surface("walls", item.counter, armor=1)
        self.assertEqual(model.surface_sprite("walls", item), "WALLC019")
        self.assertTrue(item.broken)
        for properties in ({"armor": 0}, {"armor": 61}, {"owner": 5}, {"kind": "1", "broken": True}):
            with self.assertRaises(ValueError):
                model.update_surface("walls", item.counter, **properties)
        for kind, boundary in (("0", 19), ("1", 49), ("2", 99), ("3", 1)):
            item = model.update_surface("walls", item.counter, kind=kind, armor=boundary, broken=False)
            self.assertEqual(model.surface_sprite("walls", item)[5], "0")
            self.assertEqual(item.armor, boundary)
            item = model.update_surface("walls", item.counter, armor=boundary + 1)
            self.assertEqual(model.surface_sprite("walls", item)[5], "0")

    def test_legacy_blocked_export_does_not_leave_deleted_objects_reserved(self):
        model = self.model({
            "blocked": [{"x": 0, "y": 0, "reason": "Roads On Map"},
                        {"x": 4, "y": 4, "reason": "Deployment"}],
            "roads": [{"Counter": 88, "roadtype": 0, "playerOwner": 0, "PosX": 0, "PosY": 0}],
        })
        self.assertNotIn((0, 0), model.static_blocked)
        model.delete_surface("roads", 88)
        self.assertNotIn((0, 0), model.blocked)
        self.assertEqual(model.blocked[4, 4], "Deployment")
        model.add_building(100, 0, 0, 0)

    def test_bridge_sprites_and_unsupported_junction_rollback(self):
        model = self.model()
        model.cells[2, 2].terrain = "DEEP000"
        model.cells[3, 2].terrain = "BANK000"
        model.mark_saved()
        model.begin_stroke()
        for x in range(1, 5):
            model.paint_surface("roads", x, 2, "1")
        model.end_stroke()
        self.assertEqual(model.surface_sprite("roads", model.surface_at("roads", (2, 2))), "ROAHB006")
        self.assertEqual(model.surface_sprite("roads", model.surface_at("roads", (3, 2))), "ROAHR017")
        before = model.state()
        model.begin_stroke()
        with self.assertRaisesRegex(ValueError, "bridge artwork"):
            model.paint_surface("roads", 2, 1, "1")
        model.end_stroke()
        self.assertEqual(model.state(), before)
        bridge = model.surface_at("roads", (2, 2))
        with self.assertRaises(ValueError):
            model.update_surface("roads", bridge.counter, kind="0")
        model.delete_surface("roads", model.surface_at("roads", (3, 2)).counter)
        self.assertEqual(model.surface_sprite("roads", bridge), "ROAHB011")
        model.undo()
        self.assertEqual(model.state(), before)

    def test_registry_and_every_road_wall_ruin_texture(self):
        types = core.load_surface_types(TOOLS)
        self.assertEqual(tuple(map(len, types.values())), (2, 4, 136, 2))
        self.assertFalse(types["decors"]["BBRUI000"].passable)
        self.assertTrue(types["decors"]["UBRUI000"].passable)
        for filename, count in (("ROADS.DBI", 55), ("WALLS.DBI", 140), ("RUINS.DBI", 137)):
            art = core.TerrainArt(ROOT / "IMGS" / filename)
            self.assertEqual(len(art.records), count)
            for name in art.records:
                self.assertEqual(len(art.texture(name).rgba), 54 * 56 * 4)
        terrains = core.load_terrains(TOOLS / "terrain-types.csv")
        self.assertEqual(sum(item.scenery for item in terrains.values()), 354)


@slow
@unittest.skipUnless(os.name == "nt", "Jet database bridge requires Windows")
class WindowsBridgeTests(unittest.TestCase):
    @staticmethod
    def snapshot(database):
        powershell = Path(os.environ["SystemRoot"]) / "SysWOW64" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
        command = (
            "$ErrorActionPreference='Stop';[Console]::OutputEncoding=New-Object System.Text.UTF8Encoding;"
            "$c=New-Object -ComObject ADODB.Connection;"
            "$b=New-Object System.Data.OleDb.OleDbConnectionStringBuilder;"
            "$b.Provider='Microsoft.Jet.OLEDB.4.0';$b['Mode']='Read';"
            f"$b['Data Source']='{str(database).replace(chr(39), chr(39) * 2)}';"
            "$c.Open($b.ConnectionString);try {$s=$c.OpenSchema(20);$tables=@();"
            "while(-not $s.EOF){if($s.Fields.Item('TABLE_TYPE').Value -eq 'TABLE')"
            "{$tables+=[string]$s.Fields.Item('TABLE_NAME').Value};$s.MoveNext()};$s.Close();"
            "$snapshot=@{};foreach($table in $tables){$r=$c.Execute(\"SELECT * FROM [$table]\");$rows=@();"
            "while(-not $r.EOF){$row=@{};foreach($f in $r.Fields){$v=$f.Value;"
            "if($v -is [DBNull]){$v=$null}elseif($v -is [byte[]]){$v=[Convert]::ToBase64String($v)};"
            "$row[$f.Name]=$v};$rows+=,$row;$r.MoveNext()};$r.Close();$snapshot[$table]=$rows};"
            "$snapshot|ConvertTo-Json -Depth 8}finally{$c.Close()}"
        )
        result = core.subprocess.run([str(powershell), "-NoProfile", "-NonInteractive", "-Command", command],
                                     capture_output=True, timeout=120, creationflags=core.subprocess.CREATE_NO_WINDOW)
        if result.returncode:
            raise AssertionError(result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    def test_deployment_mdb_zip_and_skirmish_roundtrip_preserves_other_tables(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        fixtures = ((GREEN_SKIRMISH, None),
                    (ROOT / "SCENS" / "SCEN1.ZIP", "s1-99-1.mdb"))
        for source, member in fixtures:
            with self.subTest(source=source), tempfile.TemporaryDirectory() as work:
                source_hash = hashlib.sha256(source.read_bytes()).digest()
                document = core.Document.open(source, member, script, registry, core.load_terrains(registry))
                try:
                    model = document.model
                    before = self.snapshot(document.directory / "source.mdb")
                    original = model.deployment_rows()
                    first = next(item for item in model.deployments.values() if item.player == 1)
                    erased = next(item for item in model.deployments.values() if item.player == 2)
                    free = [key for key in model.cells if key not in model.deployment_positions][:4]
                    model.begin_stroke()
                    model.paint_deployment(first.x, first.y, 4)
                    model.paint_deployment(erased.x, erased.y, 2, erase=True)
                    for player, key in enumerate(free, 1):
                        model.paint_deployment(*key, player)
                    model.end_stroke()
                    expected = model.deployment_rows()
                    output = Path(work) / ("zones.zip" if member else "zones.mdb")
                    document.save_copy(output, model.rows(), deployments=expected)
                    reopened = core.Document.open(output, member, script, registry, model.terrains)
                    try:
                        after = self.snapshot(reopened.directory / "source.mdb")
                        self.assertEqual(before.keys(), after.keys())
                        for table in before:
                            if table != "Deployment":
                                self.assertEqual(before[table], after[table], table)
                        actual = reopened.model.deployment_rows()
                        self.assertEqual(sorted((r["player"], r["PosX"], r["PosY"], r["Turn"]) for r in expected),
                                         sorted((r["player"], r["PosX"], r["PosY"], r["Turn"]) for r in actual))
                        by_counter = {row["Counter"]: row for row in actual}
                        self.assertEqual(by_counter[first.counter]["player"], 4)
                        self.assertNotIn(erased.counter, by_counter)
                        for row in original:
                            if row["Counter"] not in (first.counter, erased.counter):
                                self.assertEqual(by_counter[row["Counter"]], row)
                        new = [r for r in actual if r["Counter"] not in {r["Counter"] for r in original}]
                        self.assertEqual(len(new), 4)
                        self.assertEqual(len({r["Counter"] for r in actual}), len(actual))
                        self.assertTrue(all(r["Counter"] >= 0 for r in actual))
                    finally:
                        reopened.close()
                    if member:
                        with zipfile.ZipFile(source) as old, zipfile.ZipFile(output) as new:
                            self.assertEqual(old.namelist(), new.namelist())
                            for name in old.namelist():
                                if name != member:
                                    self.assertEqual(old.read(name), new.read(name))
                        bundle = Path(work) / "bundle"
                        document.export_skirmish(bundle, ROOT / "SCENS" / "SKIRMISH.MDB", source,
                                                 rows=model.rows(), buildings=model.building_rows(),
                                                 next_id=model.next_building_id, surfaces=model.surface_rows(),
                                                 units=model.unit_rows(), next_unit_id=model.next_unit_id,
                                                 scenario=model.scenario, deployments=expected)
                        catalog = self.snapshot(bundle / "SCENS" / "SKIRMISH.MDB")
                        entry = next(row for row in catalog["Scenario Info"] if row["FileName"].startswith("edit"))
                        exported = core.Document.open(bundle / "SCENS" / "SCEN1.ZIP", entry["FileName"],
                                                      script, registry, model.terrains)
                        try:
                            self.assertEqual(exported.model.deployment_rows(), actual)
                        finally:
                            exported.close()
                    captured = model.deployment_rows()
                    model.begin_stroke()
                    model.paint_deployment(*free[0], 4)
                    model.end_stroke()
                    snapshot_copy = Path(work) / "captured.mdb"
                    document.save_copy(snapshot_copy, model.rows(), deployments=captured)
                    captured_document = core.Document.open(snapshot_copy, None, script, registry, model.terrains)
                    try:
                        self.assertEqual(captured_document.model.deployment_at(free[0]).player, 1)
                    finally:
                        captured_document.close()
                finally:
                    document.close()
                self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), source_hash)

    def test_deployment_independent_validation_empty_zones_and_transaction_rollback(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        source = GREEN_SKIRMISH
        source_hash = hashlib.sha256(source.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                original = model.deployment_rows()
                first, other = original[:2]
                invalid = [None, {}, [None], [first, first], [{k: v for k, v in first.items() if k != "Turn"}]]
                for changes in ({"Counter": 2147483647}, {"Counter": True}, {"Counter": 2**32},
                                {"player": 0}, {"player": 5}, {"player": "1"}, {"Turn": 2},
                                {"PosX": -1}, {"PosY": model.size}, {"PosX": 1.25},
                                {"PosX": None}, {"unexpected": 1},
                                {"PosX": other["PosX"], "PosY": other["PosY"]}):
                    invalid.append([{**first, **changes}, *original[1:]])
                duplicate = {**first, "Counter": -1}
                invalid.append([*original, duplicate])
                for index, rows in enumerate(invalid):
                    # None is the save API's "use model" sentinel, so validate JSON null directly.
                    output = Path(work) / f"invalid-{index}.mdb"
                    if rows is None:
                        payload = Path(work) / "null-deployments.json"
                        payload.write_text("null", encoding="utf-8")
                        terrain_csv = document.directory / "map.csv"
                        with self.assertRaises(core.MapToolError):
                            core.run_map_tool(script, Action="Apply", Source=document.directory / "source.mdb",
                                              Output=output, Csv=terrain_csv, TerrainCsv=registry, DeploymentJson=payload)
                    else:
                        with self.assertRaises(core.MapToolError, msg=str(rows)):
                            document.save_copy(output, model.rows(), deployments=rows)
                    self.assertFalse(output.exists())
                bad_map = model.rows()
                bad_map[-1] = {**bad_map[-1], "Id": "2147483647"}
                model.begin_stroke()
                model.paint_deployment(first["PosX"], first["PosY"], 4)
                model.end_stroke()
                with self.assertRaises(core.MapToolError):
                    document.save_copy(Path(work) / "rollback.mdb", bad_map)
                self.assertFalse((Path(work) / "rollback.mdb").exists())
                model.begin_stroke()
                for item in list(model.deployments.values()):
                    model.paint_deployment(item.x, item.y, item.player, erase=True)
                formerly_reserved = next((row["PosX"], row["PosY"]) for row in original
                                         if (row["PosX"], row["PosY"]) not in model.blocked
                                         and not model.building_at((row["PosX"], row["PosY"]))
                                         and model.terrains[model.cells[row["PosX"], row["PosY"]].terrain.upper()].paintable)
                self.assertTrue(model.paint(*formerly_reserved, "NORD116", "trees"))
                model.end_stroke()
                empty_copy = Path(work) / "empty.mdb"
                document.save_copy(empty_copy, model.rows(), deployments=[])
                reopened = core.Document.open(empty_copy, None, script, registry, model.terrains)
                try:
                    self.assertEqual(reopened.model.deployment_rows(), [])
                    self.assertEqual(reopened.model.cells[formerly_reserved].terrain, "NORD116")
                finally:
                    reopened.close()
            finally:
                document.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), source_hash)

    def test_deployment_ai_and_nonzero_turn_rows_are_preserved_not_rewritten(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        with tempfile.TemporaryDirectory() as work:
            source = Path(work) / "special.mdb"
            core.shutil.copyfile(GREEN_SKIRMISH, source)
            initial = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                free = [key for key in initial.model.cells if key not in initial.model.deployment_positions][:2]
            finally:
                initial.close()
            powershell = Path(os.environ["SystemRoot"]) / "SysWOW64" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
            command = (
                "$ErrorActionPreference='Stop';$c=New-Object -ComObject ADODB.Connection;"
                "$b=New-Object System.Data.OleDb.OleDbConnectionStringBuilder;"
                "$b.Provider='Microsoft.Jet.OLEDB.4.0';$b['Mode']='Share Exclusive';"
                f"$b['Data Source']='{str(source).replace(chr(39), chr(39) * 2)}';"
                "$c.Open($b.ConnectionString);try {"
                f"$null=$c.Execute('INSERT INTO [Deployment] ([player],[PosX],[PosY],[Turn]) VALUES (2,{free[0][0]},{free[0][1]},7)');"
                f"$null=$c.Execute('INSERT INTO [Deployment] ([player],[PosX],[PosY],[Turn]) VALUES (0,{free[1][0]},{free[1][1]},0)');"
                "}finally{$c.Close()}"
            )
            result = core.subprocess.run([str(powershell), "-NoProfile", "-NonInteractive", "-Command", command],
                                         capture_output=True, timeout=120, creationflags=core.subprocess.CREATE_NO_WINDOW)
            self.assertEqual(result.returncode, 0, result.stderr.decode(errors="replace"))
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                preserved = [item for item in model.deployments.values() if not item.editable]
                self.assertEqual(len(preserved), 2)
                model.begin_stroke()
                item = next(item for item in model.deployments.values() if item.editable)
                model.paint_deployment(item.x, item.y, 4)
                model.end_stroke()
                output = Path(work) / "preserved.mdb"
                document.save_copy(output, model.rows())
                reopened = core.Document.open(output, None, script, registry, model.terrains)
                try:
                    for item in preserved:
                        self.assertEqual(reopened.model.deployments[item.counter], item)
                finally:
                    reopened.close()
                rows = model.deployment_rows()
                for item in preserved:
                    for remove in (True, False):
                        changed = [row for row in rows if row["Counter"] != item.counter]
                        if not remove:
                            changed.append({**item.row(), "Turn": 0, "player": 3})
                        invalid_copy = Path(work) / f"bad-preserved-{item.counter}-{remove}.mdb"
                        with self.assertRaises(core.MapToolError):
                            document.save_copy(invalid_copy, model.rows(), deployments=changed)
                        self.assertFalse(invalid_copy.exists())
            finally:
                document.close()

    def test_scenario_settings_archive_and_registered_skirmish_preservation(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        source, catalog = ROOT / "SCENS" / "SCEN1.ZIP", ROOT / "SCENS" / "SKIRMISH.MDB"
        hashes = [hashlib.sha256(path.read_bytes()).digest() for path in (source, catalog)]
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, "s1-3-1.mdb", script, registry, core.load_terrains(registry))
            try:
                model = document.model
                before = self.snapshot(document.directory / "source.mdb")
                settings = replace(model.scenario, title="Editor's r\u00e9sum\u00e9", turns=12, points=420,
                                   briefing="Editor's briefing##Objective: hold the line.",
                                   available=tuple((kind, kind in (2, 7, 56)) for kind, _ in model.scenario.available))
                model.update_scenario(settings)
                output = Path(work) / "scenario.zip"
                document.save_copy(output, model.rows())
                with zipfile.ZipFile(source) as original, zipfile.ZipFile(output) as saved:
                    self.assertEqual(original.namelist(), saved.namelist())
                    for name in original.namelist():
                        if name != document.member:
                            self.assertEqual(original.read(name), saved.read(name))
                reopened = core.Document.open(output, document.member, script, registry, model.terrains)
                try:
                    self.assertEqual(reopened.model.scenario, settings)
                    after = self.snapshot(reopened.directory / "source.mdb")
                    self.assertEqual(before.keys(), after.keys())
                    for table in before:
                        if table not in ("Scenario Info", "One Off Units Available"):
                            self.assertEqual(before[table], after[table], table)
                    changed_fields = {"TitleEnglish", "descriptionEnglish", "turn_qty", "One Off Deployment"}
                    for field, value in before["Scenario Info"][0].items():
                        if field not in changed_fields:
                            self.assertEqual(value, after["Scenario Info"][0][field], field)
                    old_roster = {row["Enum"]: row for row in before["One Off Units Available"]}
                    for row in after["One Off Units Available"]:
                        self.assertEqual(row["Counter"], old_roster[row["Enum"]]["Counter"])
                finally:
                    reopened.close()
                bundle = Path(work) / "bundle"
                document.export_skirmish(bundle, catalog, source, rows=model.rows(), buildings=model.building_rows(),
                                          next_id=model.next_building_id, surfaces=model.surface_rows(),
                                          units=model.unit_rows(), next_unit_id=model.next_unit_id, scenario=settings)
                catalog_before, catalog_after = self.snapshot(catalog), self.snapshot(bundle / "SCENS" / "SKIRMISH.MDB")
                new_rows = [row for row in catalog_after["Scenario Info"]
                            if row["Counter"] not in {row["Counter"] for row in catalog_before["Scenario Info"]}]
                self.assertEqual(len(new_rows), 1)
                entry = new_rows[0]
                self.assertEqual(entry["TitleEnglish"], settings.title)
                self.assertEqual(entry["descriptionEnglish"], settings.briefing)
                self.assertEqual(entry["One Off Deployment"], 420)
                self.assertEqual(entry["Size"], model.size)
                for row in catalog_before["Scenario Info"]:
                    self.assertIn(row, catalog_after["Scenario Info"])
                exported_archive = bundle / "SCENS" / "SCEN1.ZIP"
                with zipfile.ZipFile(source) as original, zipfile.ZipFile(exported_archive) as saved:
                    for name in original.namelist():
                        self.assertEqual(original.read(name), saved.read(name))
                    self.assertEqual(len(saved.namelist()), len(original.namelist()) + 1)
                exported = core.Document.open(exported_archive, entry["FileName"], script, registry, model.terrains)
                try:
                    self.assertEqual(exported.model.scenario, settings)
                finally:
                    exported.close()
                with self.assertRaises(FileExistsError):
                    document.export_skirmish(bundle, catalog, source, rows=model.rows(), buildings=[],
                                              next_id=model.next_building_id, surfaces={}, units={},
                                              next_unit_id=model.next_unit_id, scenario=settings)
            finally:
                document.close()
        self.assertEqual(hashes, [hashlib.sha256(path.read_bytes()).digest() for path in (source, catalog)])

    def test_scenario_bridge_validation_and_rollback(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        source = GREEN_SKIRMISH
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                model.add_unit(2, *self.unit_spot(model))
                good = model.scenario
                bad = [replace(good, title=" "), replace(good, turns=True), replace(good, points=-1),
                       replace(good, available=good.available + ((2, True),)),
                       replace(good, available=((2, True),)),
                       replace(good, available=good.available + ((40, True),))]
                for index, settings in enumerate(bad):
                    output = Path(work) / f"bad-{index}.mdb"
                    with self.subTest(index=index), self.assertRaises(core.MapToolError):
                        document.save_copy(output, model.rows(), scenario=settings)
                    self.assertFalse(output.exists())
            finally:
                document.close()

    @staticmethod
    def unit_spot(model):
        for x, y in model.cells:
            if (x, y) in model.unit_positions:
                continue
            candidate = core.Unit(model.next_unit_id, 2, x, y, armor=5, ap=80)
            try:
                model.validate_unit(candidate)
            except ValueError:
                continue
            return x, y
        raise AssertionError("No compatible unreserved infantry placement in the fixture.")

    def test_unit_later_campaign_roundtrip_timing_ids_orders_and_archive_preservation(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        source = ROOT / "SCENS" / "SCEN1.ZIP"
        source_hash = hashlib.sha256(source.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, "s4-20-1.mdb", script, registry, core.load_terrains(registry))
            try:
                model = document.model
                self.assertTrue(any(unit.layer == "reinforcements" and unit.arrival == 2 for unit in model.units.values()))
                original = dict(model.units)
                starting = next(unit for unit in original.values() if unit.layer == "units")
                self.assertEqual(starting.arrival, 0)
                x, y = self.unit_spot(model)
                initial = model.add_unit(2, x, y, owner=1, facing=7)
                later = model.add_unit(4, x, y, arrival=3, arrival_order=True)
                model.update_unit(later.identifier, arrival=5)
                model.update_unit(starting.identifier, facing=(starting.facing + 1) % 8)
                output = Path(work) / "units.zip"
                document.save_copy(output, model.rows())
                with zipfile.ZipFile(source) as before, zipfile.ZipFile(output) as after:
                    self.assertEqual(before.namelist(), after.namelist())
                    for name in before.namelist():
                        if name != document.member:
                            self.assertEqual(before.read(name), after.read(name), name)
                saved = core.Document.open(output, document.member, script, registry, model.terrains)
                try:
                    self.assertEqual(saved.model.next_unit_id, model.next_unit_id)
                    for identifier, unit in original.items():
                        expected = model.units[identifier]
                        self.assertEqual(saved.model.units[identifier], expected)
                        self.assertEqual(saved.model.units[identifier].order_turn, unit.order_turn)
                    self.assertEqual(saved.model.units[initial.identifier].owner, 1)
                    self.assertEqual(saved.model.units[initial.identifier].facing, 7)
                    self.assertEqual(saved.model.units[later.identifier].arrival, 5)
                    self.assertTrue(saved.model.units[later.identifier].arrival_order)
                    self.assertIsNotNone(saved.model.units[later.identifier].counter)
                    self.assertEqual(saved.model.units_at((x, y), 0)[0].identifier, initial.identifier)
                    self.assertEqual(saved.model.units_at((x, y), 5)[0].identifier, later.identifier)
                finally:
                    saved.close()
            finally:
                document.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), source_hash)

    def test_unit_conversion_references_malformed_input_and_transaction_rollback(self):
        script = TOOLS / "map_tool.ps1"
        registry = script.with_name("terrain-types.csv")
        source = GREEN_SKIRMISH
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                self.assertEqual(model.units[101].arrival, 0)
                self.assertEqual(model.units[101].order_turn, 2)
                with self.assertRaisesRegex(ValueError, "reference"):
                    model.delete_unit(100)
                x, y = self.unit_spot(model)
                added = model.add_unit(4, x, y)
                initial_copy = Path(work) / "initial.mdb"
                document.save_copy(initial_copy, model.rows())
                initial = core.Document.open(initial_copy, None, script, registry, model.terrains)
                try:
                    old = initial.model.units[added.identifier]
                    initial.model.update_unit(old.identifier, arrival=3)
                    converted_copy = Path(work) / "converted.mdb"
                    initial.save_copy(converted_copy, initial.model.rows())
                    converted = core.Document.open(converted_copy, None, script, registry, model.terrains)
                    try:
                        self.assertEqual(converted.model.units[old.identifier].layer, "reinforcements")
                        converted.model.update_unit(old.identifier, arrival=0)
                        restored_copy = Path(work) / "restored.mdb"
                        converted.save_copy(restored_copy, converted.model.rows())
                        restored = core.Document.open(restored_copy, None, script, registry, model.terrains)
                        try:
                            self.assertEqual(restored.model.units[old.identifier].layer, "units")
                            self.assertEqual(restored.model.units[old.identifier].identifier, old.identifier)
                        finally:
                            restored.close()
                    finally:
                        converted.close()
                finally:
                    initial.close()
                good = model.unit_rows()
                cases = [None, {}, {"units": [], "reinforcements": None}]
                missing_reference = json.loads(json.dumps(good))
                missing_reference["units"] = [row for row in missing_reference["units"] if row["ID"] != "100"]
                cases.append(missing_reference)
                invalid_health = json.loads(json.dumps(good))
                invalid_health["units"][-1]["currentAR"] = "999"
                cases.append(invalid_health)
                duplicate = json.loads(json.dumps(good))
                duplicate["reinforcements"].append(core.Unit(added.identifier, 4, x, y, arrival=3,
                                                               layer="reinforcements").row())
                cases.append(duplicate)
                invalid_turn = json.loads(json.dumps(good))
                invalid_turn["reinforcements"].append(core.Unit(model.next_unit_id, 4, x, y, arrival=101,
                                                                  layer="reinforcements").row())
                cases.append(invalid_turn)
                for index, units in enumerate(cases):
                    output = Path(work) / f"bad-unit-{index}.mdb"
                    if units is None:
                        unit_json = Path(work) / "null-units.json"
                        unit_json.write_text("null")
                        map_csv = Path(work) / "map.csv"
                        with map_csv.open("w", newline="") as stream:
                            writer = csv.DictWriter(stream, fieldnames=("Id", "PosX", "PosY", "TerrainImg"))
                            writer.writeheader()
                            writer.writerows(model.rows())
                        with self.assertRaises(core.MapToolError):
                            core.run_map_tool(script, Action="Apply", Source=document.directory / "source.mdb",
                                              Output=output, Csv=map_csv, TerrainCsv=registry, UnitJson=unit_json,
                                              NextUnitId=model.next_unit_id)
                    else:
                        with self.assertRaises(core.MapToolError):
                            document.save_copy(output, model.rows(), units=units)
                    self.assertFalse(output.exists())
            finally:
                document.close()

    def test_surface_roundtrip_and_independent_bridge_rejections(self):
        source = GREEN_SKIRMISH
        script, registry = TOOLS / "map_tool.ps1", TOOLS / "terrain-types.csv"
        original_hash = hashlib.sha256(source.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                metadata = json.loads((document.directory / "objects.json").read_text(encoding="utf-8-sig"))
                self.assertTrue(any(row["reason"] == "Roads On Map" for row in metadata["blocked"]))
                self.assertEqual((len(model.surfaces["roads"]), len(model.surfaces["walls"])), (18, 11))
                buildings = model.building_rows()
                road = next(iter(model.surfaces["roads"].values()))
                wall = next(iter(model.surfaces["walls"].values()))
                model.update_surface("roads", road.counter, kind="1", owner=2)
                model.update_surface("walls", wall.counter, owner=3, armor=30)
                model.begin_stroke()
                for x in (18, 19, 20):
                    model.paint_surface("roads", x, 0, "0")
                for x in (18, 19):
                    model.paint_surface("walls", x, 3, "1", owner=1)
                model.paint_surface("decors", 18, 6, "BBRUI000")
                model.paint_surface("decors", 19, 6, "UBRUI110")
                scenery = next(name for name, item in model.terrains.items() if item.scenery)
                model.paint(20, 6, scenery, "scenery")
                model.end_stroke()
                output = Path(work) / "layers.mdb"
                document.save_copy(output, model.rows())
                reopened = core.Document.open(output, None, script, registry, model.terrains)
                try:
                    self.assertEqual(reopened.model.rows(), model.rows())
                    self.assertEqual(reopened.model.building_rows(), buildings)
                    for layer, items in model.surfaces.items():
                        saved = {(item.x, item.y): item for item in reopened.model.surfaces[layer].values()}
                        self.assertEqual(len(saved), len(items))
                        for item in items.values():
                            actual = saved[item.x, item.y]
                            self.assertEqual(replace(actual, counter=item.counter), item)
                            self.assertGreaterEqual(actual.counter, 0)
                            if item.counter >= 0:
                                self.assertEqual(actual.counter, item.counter)
                            self.assertEqual(reopened.model.surface_sprite(layer, actual),
                                             model.surface_sprite(layer, item))
                finally:
                    reopened.close()
                for variant, layer, field, value, message in (
                        ("counter", "roads", "Counter", "2147483646", "counters"),
                        ("bounds", "roads", "PosX", "255", "outside the map"),
                        ("owner", "walls", "playerOwner", "5", "owner"),
                        ("armor", "walls", "currentAR", "0", "armor"),
                        ("decor", "decors", "id", "UNKNOWN", "type")):
                    bad = model.surface_rows()
                    bad[layer][0][field] = value
                    destination = Path(work) / f"bad-{variant}.mdb"
                    with self.assertRaisesRegex(core.MapToolError, message):
                        document.save_copy(destination, model.rows(), surfaces=bad)
                    self.assertFalse(destination.exists())
                bad = model.surface_rows()
                del bad["roads"]
                with self.assertRaisesRegex(core.MapToolError, "arrays"):
                    document.save_copy(Path(work) / "missing.mdb", model.rows(), surfaces=bad)
                bad = model.surface_rows()
                bad["roads"] = None
                with self.assertRaisesRegex(core.MapToolError, "array"):
                    document.save_copy(Path(work) / "null.mdb", model.rows(), surfaces=bad)
                for layer, items in model.surfaces.items():
                    for counter in list(items):
                        model.delete_surface(layer, counter)
                empty = Path(work) / "empty-layers.mdb"
                document.save_copy(empty, model.rows())
                reopened = core.Document.open(empty, None, script, registry, model.terrains)
                try:
                    self.assertTrue(all(not items for items in reopened.model.surfaces.values()))
                    self.assertEqual(reopened.model.building_rows(), buildings)
                finally:
                    reopened.close()
            finally:
                document.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), original_hash)

    def test_tree_and_building_roundtrip_and_bridge_rejections(self):
        source = GREEN_SKIRMISH
        script, registry = TOOLS / "map_tool.ps1", TOOLS / "terrain-types.csv"
        original_hash = hashlib.sha256(source.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as work:
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                model = document.model
                lab, pod = model.buildings[1000000], model.buildings[1000001]
                self.assertEqual((lab.kind, lab.x, lab.y), (237, 10, 14))
                self.assertEqual((pod.kind, pod.x, pod.y), (480, 8, 10))
                model.begin_stroke()
                model.paint(18, 0, "NORD116", "trees")
                model.end_stroke()
                valid = []
                for x, y in model.cells:
                    candidate = core.Building(model.next_building_id, 480, 0, 40, x, y, False)
                    try:
                        model.validate_building(candidate)
                    except ValueError:
                        continue
                    valid.append((x, y))
                self.assertGreater(len(valid), 2)
                building = model.add_building(480, 0, *valid[0])
                model.update_building(building.identifier, x=valid[1][0], y=valid[1][1], owner=2, armor=25,
                                      ai_target=True)
                existing = model.buildings[1000000]
                model.update_building(existing.identifier, kind=480, armor=35, owner=1)
                model.delete_building(1000001)
                output = Path(work) / "objects.mdb"
                document.save_copy(output, model.rows())
                reopened = core.Document.open(output, None, script, registry, core.load_terrains(registry))
                try:
                    self.assertEqual(reopened.model.rows(), model.rows())
                    self.assertEqual(set(reopened.model.buildings), set(model.buildings))
                    for identifier, item in model.buildings.items():
                        saved = reopened.model.buildings[identifier]
                        self.assertEqual(replace(saved, counter=item.counter), item)
                        if item.counter is None:
                            self.assertIsNotNone(saved.counter)
                    self.assertEqual(reopened.model.next_building_id, model.next_building_id)
                finally:
                    reopened.close()
                bad = model.building_rows()
                bad[-1]["PosX"] = "255"
                with self.assertRaisesRegex(core.MapToolError, "outside the map"):
                    document.save_copy(Path(work) / "bad.mdb", model.rows(), bad)
                self.assertFalse((Path(work) / "bad.mdb").exists())
                bad = model.building_rows()
                bad[-1]["Counter"] = "555"
                with self.assertRaisesRegex(core.MapToolError, "AutoNumber"):
                    document.save_copy(Path(work) / "bad-counter.mdb", model.rows(), bad)
                self.assertFalse((Path(work) / "bad-counter.mdb").exists())
            finally:
                document.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), original_hash)

    def test_empty_building_list_and_reference_deletion_protection(self):
        source = GREEN_SKIRMISH
        script, registry = TOOLS / "map_tool.ps1", TOOLS / "terrain-types.csv"
        with tempfile.TemporaryDirectory() as work:
            fixture = Path(work) / "referenced.mdb"
            fixture.write_bytes(source.read_bytes())
            powershell = Path(os.environ["SystemRoot"]) / "SysWOW64" / "WindowsPowerShell" / "v1.0" / "powershell.exe"
            command = (
                "$ErrorActionPreference='Stop'; $c=New-Object -ComObject ADODB.Connection; "
                "$b=New-Object System.Data.OleDb.OleDbConnectionStringBuilder; "
                "$b.Provider='Microsoft.Jet.OLEDB.4.0'; "
                f"$b['Data Source']='{str(fixture).replace(chr(39), chr(39) * 2)}'; "
                "$c.Open($b.ConnectionString); try { "
                "$null=$c.Execute('INSERT INTO [Objectives ID List] ([Id]) VALUES (1000000)'); "
                "$null=$c.Execute(\"INSERT INTO [Ruins On Map] ([id],[PosX],[PosY]) VALUES ('BBRUI000',18,0)\"); "
                "$null=$c.Execute('INSERT INTO [Automated Units] "
                "([AutomatedType],[StartX],[StartY],[DestX],[DestY],[TurnArrival]) VALUES (0,18,1,18,2,2)') "
                "} finally { $c.Close() }"
            )
            result = core.subprocess.run([str(powershell), "-NoProfile", "-NonInteractive", "-Command", command],
                                         capture_output=True, timeout=120, creationflags=core.subprocess.CREATE_NO_WINDOW)
            self.assertEqual(result.returncode, 0, result.stderr)
            document = core.Document.open(fixture, None, script, registry, core.load_terrains(registry))
            try:
                self.assertEqual(document.model.blocked[18, 0], "Ruins On Map")
                self.assertNotIn((18, 1), document.model.static_blocked)
                self.assertNotIn((18, 2), document.model.static_blocked)
                self.assertEqual((document.model.scripts.events[0].x, document.model.scripts.events[0].y), (18, 1))
                placed = document.model.add_unit(4, 18, 1)
                copy = Path(work) / "event-overlap.mdb"
                document.save_copy(copy, document.model.rows())
                reopened = core.Document.open(copy, None, script, registry, document.model.terrains)
                try:
                    self.assertEqual((reopened.model.units[placed.identifier].x,
                                      reopened.model.units[placed.identifier].y), (18, 1))
                finally:
                    reopened.close()
                with self.assertRaisesRegex(ValueError, "referenced"):
                    document.model.delete_building(1000000)
                forged = [row for row in document.model.building_rows() if row["id"] != "1000000"]
                with self.assertRaisesRegex(core.MapToolError, "referenced"):
                    document.save_copy(Path(work) / "broken.mdb", document.model.rows(), forged)
                self.assertFalse((Path(work) / "broken.mdb").exists())
            finally:
                document.close()
            document = core.Document.open(source, None, script, registry, core.load_terrains(registry))
            try:
                for identifier in list(document.model.buildings):
                    document.model.delete_building(identifier)
                output = Path(work) / "empty.mdb"
                document.save_copy(output, document.model.rows())
                reopened = core.Document.open(output, None, script, registry, core.load_terrains(registry))
                try:
                    self.assertEqual(reopened.model.buildings, {})
                finally:
                    reopened.close()
            finally:
                document.close()

    def test_process_only_policy_and_explicit_error(self):
        with patch.object(core.subprocess, "run") as run:
            run.return_value.returncode = 1
            run.return_value.stderr = b"Jet failure"
            run.return_value.stdout = b""
            with self.assertRaisesRegex(core.MapToolError, "Jet failure"):
                core.run_map_tool(TOOLS / "map_tool.ps1", Action="Export",
                                  Source=Path("scenario.mdb"), Output=Path("map.csv"))
            command = run.call_args.args[0]
            self.assertIn("SysWOW64", command[0])
            self.assertIn("-NonInteractive", command)
            self.assertEqual(command[command.index("-ExecutionPolicy") + 1], "Bypass")
            self.assertFalse(run.call_args.kwargs.get("shell", False))

    def test_open_archive_paint_save_and_readback_without_source_changes(self):
        source = ROOT / "SCENS" / "SCEN1.ZIP"
        script = TOOLS / "map_tool.ps1"
        registry = TOOLS / "terrain-types.csv"
        terrains = core.load_terrains(registry)
        original_hash = hashlib.sha256(source.read_bytes()).digest()
        self.assertIn("s1-3-1.mdb", core.archive_members(source))
        with tempfile.TemporaryDirectory() as work:
            output_mdb, output_zip = Path(work) / "edited.mdb", Path(work) / "edited.zip"
            document = core.Document.open(source, "s1-3-1.mdb", script, registry, terrains)
            try:
                before = document.model.rows()
                self.assertEqual(document.model.size, 24)
                document.model.begin_stroke()
                document.model.paint(0, 0, "BORE000")
                document.model.paint_surface("roads", 18, 0, "0")
                document.model.paint_surface("walls", 18, 3, "0")
                document.model.paint_surface("decors", 18, 6, "BBRUI000")
                document.model.end_stroke()
                document.save_copy(output_mdb, document.model.rows())
                document.save_copy(output_zip, document.model.rows())
                with self.assertRaises(FileExistsError):
                    document.save_copy(output_mdb, document.model.rows())
                with zipfile.ZipFile(source) as old, zipfile.ZipFile(output_zip) as new:
                    self.assertEqual(old.namelist(), new.namelist())
                    self.assertIsNone(new.testzip())
                    for member in old.namelist():
                        if member != "s1-3-1.mdb":
                            self.assertEqual(old.read(member), new.read(member))
                    self.assertEqual(new.read("s1-3-1.mdb"), output_mdb.read_bytes())
                reopened = core.Document.open(output_mdb, None, script, registry, terrains)
                try:
                    after = reopened.model.rows()
                    expected = [dict(row) for row in before]
                    expected[0]["TerrainImg"] = "BORE000"
                    self.assertEqual(after, expected)
                    for layer, items in document.model.surfaces.items():
                        actual = reopened.model.surfaces[layer]
                        self.assertEqual(len(actual), len(items))
                        for item in items.values():
                            saved = reopened.model.surface_at(layer, (item.x, item.y))
                            self.assertEqual(replace(saved, counter=item.counter), item)
                    self.assertFalse(reopened.model.dirty)
                finally:
                    reopened.close()
            finally:
                document.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), original_hash)


@slow
@unittest.skipUnless(os.name == "nt", "Desktop editor requires Windows")
class WindowsGuiTests(unittest.TestCase):
    def test_alternate_checkbox_preserves_independent_history_saves_selected_variant_and_closes_both(self):
        import map_editor as ui
        import tkinter as tk

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        archives = [ROOT / "SCENS" / "SCEN1.ZIP", ROOT / "SCENS" / "SCEN2.ZIP"]
        hashes = [hashlib.sha256(path.read_bytes()).digest() for path in archives]

        def finish():
            deadline = time.monotonic() + 30
            while app.busy and time.monotonic() < deadline:
                root.update()
                time.sleep(0.01)
            self.assertFalse(app.busy, "Variant database operation did not finish.")

        def paint(model):
            key = next(key for key, cell in model.cells.items() if model.terrains[cell.terrain.upper()].paintable)
            texture = "ARID000" if model.cells[key].terrain.upper() == "BORE000" else "BORE000"
            model.begin_stroke()
            self.assertTrue(model.paint(*key, texture))
            model.end_stroke()
            return key, texture

        documents = []
        try:
            with tempfile.TemporaryDirectory() as work, patch.object(ui.messagebox, "showerror") as errors:
                app.open_source(archives[0], "s1-2-1.mdb")
                finish()
                normal = app.document
                documents.append(normal)
                self.assertFalse(app.alternate_map.get())
                self.assertNotIn("disabled", app.alternate_control.state())
                self.assertEqual(len(normal.model.deployments), 211)
                normal_key, normal_texture = paint(normal.model)
                app.alternate_control.invoke()
                finish()
                alternate = app.document
                documents.append(alternate)
                self.assertIsNot(alternate, normal)
                self.assertTrue(app.alternate_map.get())
                self.assertEqual(alternate.source, archives[1])
                self.assertEqual(alternate.member, "s1-2-2.mdb")
                self.assertEqual(len(alternate.model.deployments), 193)
                self.assertTrue(normal.model.dirty)
                self.assertFalse(alternate.model.dirty)
                self.assertIn("Other variant has unsaved edits", app.heading.get())
                self.assertEqual(sum(u.layer == "units" for u in normal.model.units.values()), 23)
                self.assertEqual(sum(u.layer == "units" for u in alternate.model.units.values()), 25)
                alternate_key, alternate_texture = paint(alternate.model)
                app.alternate_control.invoke()
                self.assertFalse(app.busy, "Already loaded variants should not be reopened.")
                self.assertIs(app.document, normal)
                self.assertEqual(normal.model.cells[normal_key].terrain, normal_texture)
                self.assertEqual(len(normal.model.undo_stack), 1)
                self.assertEqual(len(alternate.model.undo_stack), 1)
                app.undo()
                self.assertFalse(normal.model.dirty)
                self.assertTrue(alternate.model.dirty)
                app.redo()
                self.assertTrue(normal.model.dirty)
                app.alternate_control.invoke()
                self.assertIs(app.document, alternate)
                self.assertEqual(alternate.model.cells[alternate_key].terrain, alternate_texture)
                with patch.object(ui.messagebox, "askyesno", return_value=False) as discard:
                    self.assertFalse(app.confirm_discard())
                    discard.assert_called_once()
                output = Path(work) / "alternate-edited.zip"
                with patch.object(ui.filedialog, "asksaveasfilename", return_value=str(output)):
                    app.save_dialog()
                    finish()
                self.assertFalse(alternate.model.dirty)
                self.assertTrue(normal.model.dirty)
                self.assertIn("*", root.title())
                saved = core.Document.open(output, alternate.member, app.script, app.registry, app.terrains)
                try:
                    self.assertEqual(saved.model.state(), alternate.model.state())
                    before = WindowsBridgeTests.snapshot(alternate.directory / "source.mdb")
                    after = WindowsBridgeTests.snapshot(saved.directory / "source.mdb")
                    for table in before:
                        if table != "Map":
                            self.assertEqual(before[table], after[table], table)
                finally:
                    saved.close()
                with zipfile.ZipFile(archives[1]) as original, zipfile.ZipFile(output) as edited:
                    self.assertEqual(original.namelist(), edited.namelist())
                    for member in original.namelist():
                        if member != alternate.member:
                            self.assertEqual(original.read(member), edited.read(member))
                bundle = Path(work) / "alternate-skirmish"
                with patch.object(ui.filedialog, "asksaveasfilename", return_value=str(bundle)):
                    app.export_skirmish()
                    finish()
                catalog = WindowsBridgeTests.snapshot(bundle / "SCENS" / "SKIRMISH.MDB")
                entry = next(row for row in catalog["Scenario Info"] if row["FileName"].startswith("edit"))
                exported = core.Document.open(bundle / "SCENS" / "SCEN1.ZIP", entry["FileName"],
                                              app.script, app.registry, app.terrains)
                try:
                    self.assertEqual(exported.model.state(), alternate.model.state())
                finally:
                    exported.close()
                with patch.object(ui.messagebox, "askyesno", return_value=False):
                    app.open_source(archives[1], "s1-2-2.mdb")
                self.assertIs(app.document, alternate)
                self.assertEqual(len(app.open_documents()), 2)
                with patch.object(ui.messagebox, "askyesno", return_value=True), \
                        patch.object(normal, "close", wraps=normal.close) as close_normal, \
                        patch.object(alternate, "close", wraps=alternate.close) as close_alternate:
                    app.open_source(archives[1], "s1-2-2.mdb")
                    finish()
                    close_normal.assert_called_once()
                    close_alternate.assert_called_once()
                self.assertTrue(app.alternate_map.get())
                self.assertEqual(len(app.open_documents()), 1)
                self.assertFalse(normal.directory.exists())
                self.assertFalse(alternate.directory.exists())
                errors.assert_not_called()
        finally:
            for document in app.open_documents():
                document.model.mark_saved()
            app.close()
            for document in documents:
                self.assertFalse(document.directory.exists())
        self.assertEqual(hashes, [hashlib.sha256(path.read_bytes()).digest() for path in archives])

    def test_alternate_load_failure_rolls_back_checkbox_and_unpaired_maps_disable_it(self):
        import map_editor as ui
        import tkinter as tk

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)

        def finish():
            deadline = time.monotonic() + 30
            while app.busy and time.monotonic() < deadline:
                root.update()
                time.sleep(0.01)
            self.assertFalse(app.busy)

        try:
            app.open_source(ROOT / "SCENS" / "SCEN1.ZIP", "s1-2-1.mdb")
            finish()
            normal = app.document
            state = normal.model.state()
            with patch.object(app, "load_document", side_effect=core.MapToolError("Alternate test load failed")), \
                    patch.object(ui.messagebox, "showerror") as errors:
                app.alternate_control.invoke()
                finish()
                errors.assert_called_once()
            self.assertIs(app.document, normal)
            self.assertFalse(app.alternate_map.get())
            self.assertEqual(normal.model.state(), state)
            self.assertEqual(list(app.variant_documents), [1])
            app.open_source(ROOT / "SCENS" / "SCEN1.ZIP", "s1-1-1.mdb")
            finish()
            self.assertIn("disabled", app.alternate_control.state())
            self.assertFalse(app.alternate_map.get())
            self.assertFalse(normal.directory.exists())
            # Directly opening variant 2 initializes the checkbox correctly.
            app.open_source(ROOT / "SCENS" / "SCEN2.ZIP", "s1-2-2.mdb")
            finish()
            self.assertTrue(app.alternate_map.get())
            self.assertNotIn("disabled", app.alternate_control.state())
        finally:
            for document in app.open_documents():
                document.model.mark_saved()
            root.update_idletasks()
            app.close()

    def test_unit_gui_turn_selector_properties_history_layout_and_async_save(self):
        import map_editor as ui
        import tkinter as tk

        ui.ctypes.windll.user32.SetProcessDPIAware()
        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        try:
            with tempfile.TemporaryDirectory() as work:
                app.document = core.Document.open(ROOT / "SCENS" / "SCEN1.ZIP", "s4-20-1.mdb",
                                                   app.script, app.registry, app.terrains)
                model = app.document.model
                app.mode.set("Units")
                app.change_mode()
                app.render_map()
                self.assertEqual(len(app.palette_names), 62)
                self.assertEqual(len(app.canvas.find_withtag("unit")),
                                 sum(len(model.unit_footprint(unit)) for unit in model.units.values()
                                     if unit.arrival == 0))
                root.geometry("1100x900")
                root.update_idletasks()
                self.assertLessEqual(app.open_button.master.winfo_reqwidth(), 1100)
                self.assertLessEqual(app.unit_view_box.master.winfo_reqwidth(), 1100)
                sidebar = app.unit_controls.master
                self.assertLessEqual(app.unit_delete_button.winfo_rooty() + app.unit_delete_button.winfo_height(),
                                     sidebar.winfo_rooty() + sidebar.winfo_height())
                def requested_height(widget):
                    padding = widget.pack_info()["pady"]
                    pady = sum(map(int, padding)) if isinstance(padding, tuple) else int(padding) * 2
                    return widget.winfo_reqheight() + pady
                fixed_sidebar = sum(requested_height(widget) for widget in sidebar.winfo_children()
                                    if widget.winfo_manager() == "pack" and not widget.pack_info()["expand"])
                fixed_root = sum(requested_height(widget) for widget in root.winfo_children()
                                 if widget.winfo_manager() == "pack" and not widget.pack_info()["expand"])
                self.assertLessEqual(fixed_sidebar + fixed_root + 80, 900,
                                     "The minimum window must retain at least 80 pixels for the palette.")
                app.unit_view.set("Turn 3")
                app.change_unit_turn()
                self.assertEqual(app.unit_arrival.get(), "Turn 3")
                self.assertEqual(str(app.unit_property_widgets["Armor / health"].cget("state")), "disabled")
                app.unit_kind.set(app.unit_label(4))
                app.unit_type_changed()
                app.unit_action.set("Place")
                x, y = WindowsBridgeTests.unit_spot(model)
                with patch.object(app, "event_cell", return_value=(x, y)), patch.object(ui.messagebox, "showerror") as errors:
                    app.paint_start(SimpleNamespace())
                    errors.assert_not_called()
                identifier = app.selected_unit
                self.assertEqual(model.units[identifier].arrival, 3)
                self.assertIsNone(model.stroke)
                app.unit_arrival.set("Turn 5")
                app.unit_order.set(True)
                app.apply_unit()
                self.assertEqual(model.units[identifier].arrival, 5)
                self.assertEqual(app.unit_view.get(), "Turn 5")
                app.undo()
                self.assertEqual(app.unit_view.get(), "Turn 3")
                self.assertEqual(app.unit_arrival.get(), "Turn 3")
                app.redo()
                self.assertEqual(app.unit_view.get(), "Turn 5")
                app.unit_action.set("Move")
                moved_x, moved_y = WindowsBridgeTests.unit_spot(model)
                with patch.object(app, "event_cell", return_value=(moved_x, moved_y)), patch.object(ui.messagebox, "showerror") as errors:
                    app.paint_start(SimpleNamespace())
                    app.eyedropper(SimpleNamespace())
                    errors.assert_not_called()
                self.assertEqual(app.selected_unit, identifier)
                self.assertEqual((model.units[identifier].x, model.units[identifier].y), (moved_x, moved_y))
                output = Path(work) / "gui-units.mdb"
                with patch.object(ui.filedialog, "asksaveasfilename", return_value=str(output)):
                    app.save_dialog()
                    deadline = time.monotonic() + 120
                    while app.busy and time.monotonic() < deadline:
                        root.update()
                        time.sleep(0.02)
                self.assertFalse(app.busy)
                self.assertTrue(output.exists(), app.status.get())
                saved = core.Document.open(output, None, app.script, app.registry, app.terrains)
                try:
                    self.assertEqual(saved.model.units[identifier].arrival, 5)
                    self.assertTrue(saved.model.units[identifier].arrival_order)
                    self.assertEqual((saved.model.units[identifier].x, saved.model.units[identifier].y),
                                     (moved_x, moved_y))
                finally:
                    saved.close()
                with patch.object(ui.messagebox, "askyesno", return_value=True):
                    app.delete_selected()
                self.assertNotIn(identifier, model.units)
                app.undo()
                self.assertIn(identifier, model.units)
                app.unit_view.set("Start")
                app.change_unit_turn()
                self.assertEqual(str(app.unit_property_widgets["Armor / health"].cget("state")), "normal")
                native_zero = replace(model.units[identifier], arrival=0)
                model._set_unit(identifier, native_zero)
                app.select_unit(native_zero)
                self.assertEqual(str(app.unit_property_widgets["Armor / health"].cget("state")), "disabled")
                self.assertEqual(str(app.unit_order_control.cget("state")), "normal")
                self.assertNotIn("owner", app.unit_properties())
                app.unit_kind.set(app.unit_label(2))
                app.unit_type_changed()
                with patch.object(ui.messagebox, "showerror") as errors:
                    app.apply_unit()
                    errors.assert_not_called()
                self.assertEqual(model.units[identifier].layer, "reinforcements")
                self.assertEqual(model.units[identifier].arrival, 0)
                self.assertEqual(model.units[identifier].kind, 2)
                app.unit_action.set("Place")
                app.sync_unit_controls()
                self.assertEqual(str(app.unit_property_widgets["Armor / health"].cget("state")), "normal")
                self.assertIn("owner", app.unit_properties(placing=True))
        finally:
            if app.document:
                app.document.model.mark_saved()
            app.close()

    def test_intact_wall_preview_explanation_and_explicit_full_health_edit(self):
        import map_editor as ui
        import tkinter as tk

        ui.ctypes.windll.user32.SetProcessDPIAware()
        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        source = GREEN_SKIRMISH
        original_hash = hashlib.sha256(source.read_bytes()).digest()
        try:
            app.document = core.Document.open(source, None, app.script, app.registry, app.terrains)
            app.mode.set("Walls")
            app.change_mode()
            app.render_map()
            model = app.document.model
            wall = next(iter(model.surfaces["walls"].values()))
            before = model.state()
            app.select_surface(wall)
            self.assertFalse(app.surface_broken.get())
            self.assertIn("intact preview - health 10/60", app.wall_preview.get())
            self.assertIn("Fresh battles reset HP", app.wall_preview.get())
            self.assertEqual(model.surface_sprite("walls", wall)[5], "0")
            self.assertEqual(model.state(), before, "Intact previews must not repair stored HP.")
            with tempfile.TemporaryDirectory() as work:
                untouched = Path(work) / "intact-preview.mdb"
                app.document.save_copy(untouched, model.rows())
                self.assertEqual(WindowsBridgeTests.snapshot(app.document.directory / "source.mdb"),
                                 WindowsBridgeTests.snapshot(untouched),
                                 "Saving intact previews must preserve every stored table, including low HP.")
            app.surface_armor.set("invalid")
            self.assertIn("whole number", app.wall_preview.get())
            app.surface_armor.set("61")
            self.assertIn("Health must be from 1 to 60", app.wall_preview.get())
            app.surface_full_health_button.invoke()
            self.assertEqual(app.surface_armor.get(), "60")
            self.assertIn("intact preview - health 60/60", app.wall_preview.get())
            self.assertFalse(app.surface_broken.get())
            self.assertEqual(model.state(), before, "Filling the inspector must not silently repair scenario data.")
            root.geometry("1100x900")
            root.update_idletasks()
            sidebar = app.surface_controls.master
            self.assertLessEqual(app.surface_delete_button.winfo_rooty() + app.surface_delete_button.winfo_height(),
                                 sidebar.winfo_rooty() + sidebar.winfo_height())
            self.assertLessEqual(app.surface_full_health_button.winfo_rootx()
                                 + app.surface_full_health_button.winfo_width(),
                                 sidebar.winfo_rootx() + sidebar.winfo_width())
            with patch.object(ui.messagebox, "showerror") as errors:
                app.apply_surface()
                errors.assert_not_called()
            repaired = model.surfaces["walls"][wall.counter]
            self.assertEqual(repaired, replace(wall, armor=60))
            self.assertEqual(model.surface_sprite("walls", repaired)[5], "0")
            with tempfile.TemporaryDirectory() as work:
                output = Path(work) / "repaired-wall.mdb"
                app.document.save_copy(output, model.rows())
                saved = core.Document.open(output, None, app.script, app.registry, app.terrains)
                try:
                    self.assertEqual(saved.model.surfaces["walls"][wall.counter], repaired)
                    self.assertEqual(saved.model.surface_sprite("walls", repaired), model.surface_sprite("walls", repaired))
                finally:
                    saved.close()
            app.undo()
            self.assertEqual(model.state(), before)
            self.assertIn("intact preview - health 10/60", app.wall_preview.get())
            app.redo()
            self.assertIn("intact preview - health 60/60", app.wall_preview.get())
        finally:
            if app.document:
                app.document.model.mark_saved()
            app.close()
        self.assertEqual(hashlib.sha256(source.read_bytes()).digest(), original_hash)

    def test_surface_gui_strokes_inspector_move_delete_history_and_async_save(self):
        import map_editor as ui
        import tkinter as tk

        ui.ctypes.windll.user32.SetProcessDPIAware()
        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        with tempfile.TemporaryDirectory() as work:
            try:
                app.document = core.Document.open(GREEN_SKIRMISH,
                                                  None, app.script, app.registry, app.terrains)
                def event(x, y):
                    px, py = app.center(x, y)
                    return SimpleNamespace(x=round(px - app.canvas.canvasx(0)),
                                           y=round(py - app.canvas.canvasy(0)))

                with patch.object(ui.messagebox, "showerror") as errors:
                    for mode, start in (("Roads", (18, 0)), ("Walls", (18, 3)), ("Decors", (18, 6))):
                        app.mode.set(mode)
                        app.change_mode()
                        app.render_map()
                        layer = app.current_layer()
                        before = len(app.document.model.undo_stack)
                        app.paint_start(event(*start))
                        app.paint_drag(event(start[0] + 1, start[1] + (1 if layer == "roads" else 0)))
                        app.paint_end()
                        self.assertEqual(len(app.document.model.undo_stack), before + 1)
                        self.assertEqual(len(app.canvas.find_withtag(layer)), len(app.document.model.surfaces[layer]))
                        if layer == "roads":
                            corner = app.document.model.surface_at(layer, (19, 0))
                            self.assertEqual(app.document.model.surface_sprite(layer, corner), "ROALG015")
                        item = app.document.model.surface_at(layer, start)
                        app.eyedropper(event(*start))
                        self.assertEqual(app.selected_surface, (layer, item.counter))
                        if layer != "decors":
                            app.surface_owner.set("2")
                        if layer == "walls":
                            app.surface_armor.set("30")
                        app.apply_surface()
                        app.surface_action.set("Move")
                        app.paint_start(event(start[0] + 2, start[1] + 1))
                        moved = app.document.model.surfaces[layer][item.counter]
                        self.assertEqual((moved.x, moved.y), (start[0] + 2, start[1] + 1))
                        with patch.object(ui.messagebox, "askyesno", return_value=True):
                            app.delete_selected()
                        self.assertNotIn(item.counter, app.document.model.surfaces[layer])
                        app.undo()
                        self.assertIn(item.counter, app.document.model.surfaces[layer])
                        app.redo()
                        self.assertNotIn(item.counter, app.document.model.surfaces[layer])
                        app.undo()
                    app.mode.set("Scenery")
                    app.change_mode()
                    app.erase_trees.set(False)
                    app.paint_start(event(20, 6))
                    app.paint_end()
                    self.assertTrue(app.terrains[app.document.model.cells[20, 6].terrain].scenery)
                    app.eyedropper(event(20, 6))
                    self.assertEqual(app.mode.get(), "Scenery")
                    for mode in ui.SURFACE_MODES:
                        app.mode.set(mode)
                        app.change_mode()
                        root.geometry("1100x900")
                        root.update_idletasks()
                        sidebar = app.surface_controls.master
                        self.assertLessEqual(
                            app.surface_delete_button.winfo_rooty() + app.surface_delete_button.winfo_height(),
                            sidebar.winfo_rooty() + sidebar.winfo_height())
                    output = Path(work) / "gui-surfaces.mdb"
                    expected = app.document.model.surface_rows()
                    with patch.object(ui.filedialog, "asksaveasfilename", return_value=str(output)):
                        app.save_dialog()
                        deadline = time.monotonic() + 30
                        while app.busy and time.monotonic() < deadline:
                            root.update()
                            time.sleep(0.01)
                        self.assertFalse(app.busy)
                    errors.assert_not_called()
                self.assertTrue(output.exists())
                saved = core.Document.open(output, None, app.script, app.registry, app.terrains)
                try:
                    for layer, rows in expected.items():
                        saved_rows = {(int(row["PosX"] if "PosX" in row else row["posX"]),
                                       int(row["PosY"] if "PosY" in row else row["posY"])): row
                                      for row in saved.model.surface_rows()[layer]}
                        for row in rows:
                            key = int(row.get("PosX", row.get("posX"))), int(row.get("PosY", row.get("posY")))
                            actual = dict(saved_rows[key], Counter=row["Counter"])
                            self.assertEqual(actual, row)
                    self.assertEqual(saved.model.rows(), app.document.model.rows())
                finally:
                    saved.close()
            finally:
                if app.document:
                    app.document.model.mark_saved()
                app.close()

    def test_building_gui_handlers_and_async_save(self):
        import map_editor as ui
        import tkinter as tk

        ui.ctypes.windll.user32.SetProcessDPIAware()
        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        with tempfile.TemporaryDirectory() as work:
            try:
                app.document = core.Document.open(
                    GREEN_SKIRMISH, None,
                    app.script, app.registry, app.terrains,
                )
                app.mode.set("Buildings")
                app.change_mode()
                app.render_map()

                def click(x, y):
                    px, py = app.center(x, y)
                    app.building_click(SimpleNamespace(
                        x=round(px - app.canvas.canvasx(0)), y=round(py - app.canvas.canvasy(0))))

                app.building_action.set("Place")
                click(18, 1)
                identifier = app.selected_building
                self.assertIsNotNone(identifier)
                app.building_action.set("Move")
                click(19, 2)
                app.building_kind.set(app.type_label(237))
                app.type_changed()
                app.owner.set("3")
                app.armor.set("42")
                app.ai_target.set(True)
                app.apply_building()
                item = app.document.model.buildings[identifier]
                self.assertEqual((item.kind, item.owner, item.armor, item.x, item.y, item.ai_target),
                                 (237, 3, 42, 19, 2, True))
                output = Path(work) / "gui.mdb"
                with patch.object(ui.filedialog, "asksaveasfilename", return_value=str(output)), \
                        patch.object(ui.messagebox, "showerror") as errors:
                    app.save_dialog()
                    deadline = time.monotonic() + 30
                    while app.busy and time.monotonic() < deadline:
                        root.update()
                        time.sleep(0.01)
                    self.assertFalse(app.busy, "GUI save did not finish")
                    errors.assert_not_called()
                self.assertTrue(output.exists())
                self.assertFalse(app.document.model.dirty)
                saved = core.Document.open(output, None, app.script, app.registry, app.terrains)
                try:
                    self.assertEqual(replace(saved.model.buildings[identifier], counter=item.counter), item)
                finally:
                    saved.close()
                with patch.object(ui.messagebox, "askyesno", return_value=True):
                    app.delete_building()
                self.assertNotIn(identifier, app.document.model.buildings)
                app.undo()
                self.assertIn(identifier, app.document.model.buildings)
                app.redo()
                self.assertNotIn(identifier, app.document.model.buildings)
            finally:
                if app.document:
                    app.document.model.mark_saved()
                app.close()

    def test_all_low_hp_building_previews_are_intact_without_mutating_properties(self):
        import map_editor as ui
        import tkinter as tk

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        try:
            app.mode.set("Buildings")
            for kind in app.building_types.values():
                with self.subTest(kind=kind.identifier):
                    model = DeploymentTests.model(size=max(kind.width, kind.height) + 2)
                    model.building_types = app.building_types
                    building = core.Building(1000000, kind.identifier, 2, 1, 1, 1, False, counter=99)
                    model.buildings[building.identifier] = building
                    model.mark_saved()
                    before = model.state()
                    app.document = SimpleNamespace(model=model, source=Path("preview.mdb"), member=None, close=Mock())
                    app.selected_building = building.identifier
                    with patch.object(app, "image", wraps=app.image) as images:
                        app.render_map()
                    rendered = [call.args[0] for call in images.call_args_list if call.kwargs.get("building")]
                    expected = [kind.sprite(x, y, level=app.building_levels[kind.identifier])
                                for x in range(kind.width) for y in range(kind.height)]
                    self.assertCountEqual(rendered, expected)
                    self.assertEqual(app.inspected.get(), "Building 1000000 - owner 2")
                    self.assertEqual(model.state(), before)
                    self.assertEqual(model.building_rows()[0]["currentAR"], "1")
                    self.assertEqual(model.building_rows()[0]["Counter"], "99")
                    self.assertFalse(model.dirty)
        finally:
            root.update_idletasks()
            app.close()

    def test_building_controls_fit_minimum_window_and_all_previews_load(self):
        import map_editor as ui
        import tkinter as tk

        ui.ctypes.windll.user32.SetProcessDPIAware()
        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        try:
            app.mode.set("Buildings")
            app.change_mode()
            self.assertEqual(len(app.palette_names), 81)
            for height in (900, 1100):
                root.geometry(f"1100x{height}")
                root.update_idletasks()
                sidebar = app.building_controls.master
                self.assertLessEqual(app.delete_button.winfo_rooty() + app.delete_button.winfo_height(),
                                     sidebar.winfo_rooty() + sidebar.winfo_height())
            for kind in app.building_types.values():
                for x in range(kind.width):
                    for y in range(kind.height):
                        app.building_art.texture(kind.sprite(x, y, level=app.building_levels[kind.identifier]))
        finally:
            app.close()


if __name__ == "__main__":
    unittest.main()
