import os
from pathlib import Path
import tempfile
import time
from dataclasses import replace
from unittest.mock import patch
import unittest
import zipfile

import map_editor_core as core
import test_map_editor as existing


from testsupport import GREEN_SKIRMISH, ROOT, TOOLS, slow
SCRIPT = TOOLS / "map_tool.ps1"
REGISTRY = SCRIPT.with_name("terrain-types.csv")


class ModelTests(unittest.TestCase):
    def model(self, objects=None):
        model = existing.UnitTests().model(objects)
        model.terrains.update(DEEP000=core.Terrain("DEEP000", 1, False),
                              BANK000=core.Terrain("BANK000", 2, False),
                              SHAL000=core.Terrain("SHAL000", 0, False))
        return model

    def channel(self, model, y=2, vertical=False):
        points = ((1, y), (2, y), (3, y), (4, y))
        if vertical:
            points = tuple((y, x) for x, y in points)
        for key, name in zip(points, ("BANK000", "DEEP000", "DEEP000", "BANK000")):
            model.cells[key].terrain = name
        return core.BridgeSpan(points[0], points[-1])

    def test_mine_flags_sprites_history_and_dynamic_occupancy(self):
        model = self.model()
        initial = model.state()
        model.begin_stroke()
        model.paint_surface("mines", 1, 1, "0")
        flags = dict(zip(core.MINE_FLAGS, (False, True, False, True, False, True)))
        model.paint_surface("mines", 2, 1, "1", owner=3, mine_flags=flags)
        model.end_stroke()
        normal, grav = model.surface_at("mines", (1, 1)), model.surface_at("mines", (2, 1))
        self.assertEqual(model.surface_sprite("mines", normal), "MINE000")
        self.assertEqual(model.surface_sprite("mines", grav), "GMINE000")
        persisted = replace(grav, counter=100)
        self.assertEqual(core.Surface.from_row("mines", persisted.row("mines")), persisted)
        for name, value in flags.items():
            self.assertEqual(getattr(grav, name), value)
        model.undo()
        self.assertEqual(model.state(), initial)
        model.redo()
        with self.assertRaisesRegex(ValueError, "mines"):
            model.add_unit(0, 1, 1)
        with self.assertRaises(ValueError):
            model.update_surface("mines", normal.counter, armed=1)
        model.begin_stroke()
        model.paint_surface("roads", 1, 1, "0")
        model.paint_surface("mines", 1, 1, "0", erase=True)
        model.end_stroke()
        unit = model.add_unit(0, 1, 1)
        self.assertEqual((unit.x, unit.y), (1, 1))
        model.undo()
        model.undo()
        self.assertEqual(model.blocked[1, 1], "Mines On Map")
        model.update_surface("mines", grav.counter, x=4, y=4, owner=4)
        self.assertIsNone(model.surface_at("mines", (2, 1)))
        self.assertEqual(model.surface_at("mines", (4, 4)).hidden_3, False)

    def test_legacy_duplicates_and_visibility_flags_are_preserved(self):
        row = core.Surface(100, "1", 2, 2, armed=False, hidden_AI=True, hidden_4=False).row("mines")
        second = dict(row, Counter="101")
        model = self.model({"mines": [row, second],
                            "blocked": [{"x": 2, "y": 2, "reason": "Mines On Map"}]})
        self.assertEqual(model.surface_rows()["mines"], [row, second])
        model.delete_surface("mines", 100)
        self.assertIn((2, 2), model.blocked)
        model.delete_surface("mines", 101)
        self.assertNotIn((2, 2), model.blocked)
        model.undo()
        self.assertEqual(model.surface_rows()["mines"], [second])

    def test_bridge_axes_borders_endpoints_pruning_and_existing_roads(self):
        for vertical in (False, True):
            model = self.model()
            span = self.channel(model, vertical=vertical)
            self.assertEqual(model.bridge_spans(), (span,))
            model.cells[span.tiles[1]].terrain = "SHAL000"
            self.assertEqual(model.bridge_spans(), ())
        model = self.model()
        first = self.channel(model, 0)
        self.channel(model, 1)
        third = self.channel(model, 2)
        self.assertEqual(model.bridge_spans(), (first, third))
        model.begin_stroke()
        model.paint_surface("roads", *first.start, "1")
        model.end_stroke()
        self.assertEqual(model.bridge_spans(), (first, third))
        model.cells[0, 0].terrain = "DEEP000"
        self.assertEqual(model.bridge_spans(), (core.BridgeSpan((1, 1), (4, 1)),))
        edge = self.model()
        self.channel(edge, 7)
        self.assertEqual(edge.bridge_spans(), ())
        edge = self.model()
        span = self.channel(edge)
        edge.cells[5, 2].terrain = "BANK000"
        self.assertEqual(edge.bridge_spans(), ())

    def test_bank_edit_is_explicit_restricted_occupied_and_undoable(self):
        model = self.model()
        span = self.channel(model)
        initial = model.state()
        model.begin_stroke()
        with self.assertRaises(ValueError):
            model.paint(1, 2, "DEEP000")
        self.assertTrue(model.paint(1, 2, "DEEP000", "erase_bridge"))
        self.assertEqual(model.bridge_spans(), ())
        self.assertTrue(model.paint(1, 2, "BANK000", "bridge"))
        self.assertEqual(model.bridge_spans(), (span,))
        with self.assertRaises(ValueError):
            model.paint(0, 0, "BANK000", "bridge")
        self.assertFalse(model.paint(0, 0, "DEEP000", "erase_bridge"))
        model.end_stroke()
        self.assertEqual(model.state(), initial)
        model.begin_stroke()
        model.paint(1, 2, "DEEP000", "erase_bridge")
        model.end_stroke()
        model.undo()
        self.assertEqual(model.state(), initial)
        model.redo()
        model.begin_stroke()
        model.paint_surface("roads", 1, 2, "1")
        with self.assertRaisesRegex(ValueError, "overlap"):
            model.paint(1, 2, "BANK000", "bridge")
        model.end_stroke()

    def test_native_crossing_overlap_x_preference_and_adjacent_banks(self):
        model = self.model()
        for x in range(1, 6):
            model.cells[x, 3].terrain = "BANK000" if x in (1, 5) else "DEEP000"
        for y in range(1, 6):
            model.cells[3, y].terrain = "BANK000" if y in (1, 5) else "DEEP000"
        self.assertEqual(model.bridge_spans(), (core.BridgeSpan((3, 1), (3, 5)),))
        model = self.model()
        model.cells[1, 1].terrain = model.cells[2, 1].terrain = "BANK000"
        self.assertEqual(model.bridge_spans(), (core.BridgeSpan((1, 1), (2, 1)),))
        model.cells[3, 1].terrain = "SHAL000"
        model.cells[1, 2].terrain = "DEEP000"
        model.cells[1, 3].terrain = "BANK000"
        self.assertEqual(model.bridge_spans(), ())


@slow
@unittest.skipUnless(os.name == "nt", "Native Jet and Tk checks require Windows")
class NativeTests(unittest.TestCase):
    def open(self, member="s1-5-1.mdb"):
        for archive in (ROOT / "SCENS").glob("SCEN*.ZIP"):
            with zipfile.ZipFile(archive) as package:
                if member in package.namelist():
                    return core.Document.open(archive, member, SCRIPT, REGISTRY, core.load_terrains(REGISTRY))
        self.fail(f"No installed archive contains {member}.")

    def free(self, model):
        return next(key for key, cell in model.cells.items()
                    if model.terrains[cell.terrain.upper()].paintable
                    and key not in model.blocked and not model.building_at(key))

    def test_native_noop_both_types_flags_atomic_removal_and_rollback(self):
        with tempfile.TemporaryDirectory() as work:
            folder = Path(work)
            for member in ("s1-5-1.mdb", "s3-10-1.mdb"):
                document = self.open(member)
                try:
                    original = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
                    noop = folder / f"{member}-noop.mdb"
                    document.save_copy(noop, document.model.rows())
                    self.assertEqual(existing.WindowsBridgeTests.snapshot(noop), original)
                finally:
                    document.close()
            document = self.open()
            try:
                model = document.model
                mine = next(iter(model.surfaces["mines"].values()))
                flags = dict(zip(core.MINE_FLAGS, (False, True, False, True, False, True)))
                model.update_surface("mines", mine.counter, kind="1", owner=3, **flags)
                key = self.free(model)
                model.begin_stroke()
                model.paint_surface("mines", *key, "0")
                model.end_stroke()
                output = folder / "edited.zip"
                document.save_copy(output, model.rows())
                saved = core.Document.open(output, document.member, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertEqual(saved.model.surfaces["mines"][mine.counter],
                                     model.surfaces["mines"][mine.counter])
                    self.assertGreaterEqual(saved.model.surface_at("mines", key).counter, 0)
                    untouched = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
                    changed = existing.WindowsBridgeTests.snapshot(saved.directory / "source.mdb")
                    del untouched["Mines On Map"], changed["Mines On Map"]
                    self.assertEqual(changed, untouched)
                finally:
                    saved.close()
                for field, value in (("armed", "2"), ("hidden_1", "true"), ("playerOwner", "5"),
                                     ("Counter", "2147483646"), ("typeID", "2"), ("PosX", "255")):
                    bad = model.surface_rows()
                    bad["mines"][-1][field] = value
                    failed = folder / f"bad-{field}.mdb"
                    with self.subTest(field=field), self.assertRaises(core.MapToolError):
                        document.save_copy(failed, model.rows(), surfaces=bad)
                    self.assertFalse(failed.exists())
                model.delete_surface("mines", mine.counter)
                unit = model.add_unit(0, mine.x, mine.y)
                atomic = folder / "atomic.mdb"
                document.save_copy(atomic, model.rows())
                saved = core.Document.open(atomic, None, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertNotIn(mine.counter, saved.model.surfaces["mines"])
                    self.assertEqual((saved.model.units[unit.identifier].x, saved.model.units[unit.identifier].y),
                                     (mine.x, mine.y))
                finally:
                    saved.close()
            finally:
                document.close()

    def test_native_bank_terrain_roundtrip_and_rejection(self):
        document = self.open()
        try:
            model = document.model
            key = next(key for key, cell in model.cells.items()
                       if model.terrains[cell.terrain.upper()].kind == 2
                       and key not in model.blocked and not model.building_at(key)
                       and all(not model.surface_at("roads", (key[0] + dx, key[1] + dy))
                               for dx, dy, _ in core.NEIGHBORS))
            model.begin_stroke()
            model.paint(*key, "EAU000", "erase_bridge")
            model.end_stroke()
            with tempfile.TemporaryDirectory() as work:
                output = Path(work) / "bank.mdb"
                document.save_copy(output, model.rows())
                saved = core.Document.open(output, None, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertEqual(saved.model.rows(), model.rows())
                    self.assertEqual(saved.model.bridge_spans(), model.bridge_spans())
                    original = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
                    modified = existing.WindowsBridgeTests.snapshot(output)
                    del original["Map"], modified["Map"]
                    self.assertEqual(modified, original)
                finally:
                    saved.close()
                bad = model.rows()
                cell = self.free(model)
                next(row for row in bad if (int(row["PosX"]), int(row["PosY"])) == cell)["TerrainImg"] = "EAU000"
                failed = Path(work) / "invalid-water.mdb"
                with self.assertRaisesRegex(core.MapToolError, "restricted"):
                    document.save_copy(failed, bad)
                self.assertFalse(failed.exists())
                occupied = model.rows()
                mine = next(iter(model.surfaces["mines"].values()))
                next(row for row in occupied
                     if (int(row["PosX"]), int(row["PosY"])) == (mine.x, mine.y))["TerrainImg"] = "NORD841"
                failed = Path(work) / "occupied-bank.mdb"
                with self.assertRaises(core.MapToolError):
                    document.save_copy(failed, occupied)
                self.assertFalse(failed.exists())
                model.begin_stroke()
                model.paint_surface("roads", *key, "1")
                model.end_stroke()
                kind = next(kind for (kind, tech), definition in model.unit_types.items()
                            if tech == 0 and definition.movement in (3, 9, 5)
                            and definition.width == definition.height == 1)
                unit = model.add_unit(kind, *key)
                combined = Path(work) / "banks-road-unit.mdb"
                document.save_copy(combined, model.rows())
                saved = core.Document.open(combined, None, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertEqual(saved.model.surface_at("roads", key).kind, "1")
                    self.assertEqual((saved.model.units[unit.identifier].x, saved.model.units[unit.identifier].y), key)
                finally:
                    saved.close()
        finally:
            document.close()

    def test_registered_export_includes_mines_and_banks(self):
        document = self.open()
        try:
            model = document.model
            key = self.free(model)
            model.begin_stroke()
            model.paint_surface("mines", *key, "1", owner=2)
            bank = next(key for key, cell in model.cells.items()
                        if model.terrains[cell.terrain.upper()].kind == 2
                        and key not in model.blocked and not model.building_at(key))
            model.paint(*bank, "EAU000", "erase_bridge")
            model.end_stroke()
            with tempfile.TemporaryDirectory() as work:
                bundle = Path(work) / "skirmish"
                document.export_skirmish(
                    bundle, ROOT / "SCENS" / "SKIRMISH.MDB", ROOT / "SCENS" / "SCEN1.ZIP",
                    rows=model.rows(), buildings=model.building_rows(), next_id=model.next_building_id,
                    surfaces=model.surface_rows(), units=model.unit_rows(), next_unit_id=model.next_unit_id,
                    scenario=model.scenario, deployments=model.deployment_rows(), scripts=model.scripts)
                archive = bundle / "SCENS" / "SCEN1.ZIP"
                with zipfile.ZipFile(archive) as package:
                    member = next(name for name in package.namelist() if name.startswith("edit"))
                saved = core.Document.open(archive, member, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertEqual(saved.model.surface_at("mines", key).kind, "1")
                    self.assertEqual(saved.model.surface_at("mines", key).owner, 2)
                    self.assertEqual(saved.model.cells[bank].terrain, "EAU000")
                finally:
                    saved.close()
        finally:
            document.close()

    def test_gui_modes_native_artwork_flags_preview_history_and_snapshots(self):
        import tkinter as tk
        import map_editor as ui

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        try:
            app.open_source(ROOT / "SCENS" / "SCEN1.ZIP", "s1-5-1.mdb")
            deadline = time.monotonic() + 40
            while app.busy and time.monotonic() < deadline:
                root.update()
                time.sleep(.01)
            self.assertFalse(app.busy)
            model = app.document.model
            self.assertFalse(app.canvas.find_withtag("bridges"))
            self.assertEqual(len(app.canvas.find_withtag("mines")), 16)
            app.mode.set("Mines")
            app.change_mode()
            app.select_surface(next(iter(model.surfaces["mines"].values())))
            counter = app.selected_surface[1]
            app.mine_flags["armed"].set(False)
            app.mine_flags["hidden_2"].set(False)
            app.apply_surface()
            self.assertFalse(model.surfaces["mines"][counter].armed)
            app.undo()
            self.assertTrue(app.mine_flags["armed"].get())
            app.redo()
            self.assertFalse(app.mine_flags["hidden_2"].get())
            app.brush.set("1x1")
            app.surface_kind.set(app.surface_label("mines", "1"))
            app.surface_type_changed()
            app.surface_action.set("Paint")
            target = self.free(model)
            with patch.object(app, "event_cell", return_value=target):
                app.paint_start(type("Event", (), {})())
                app.paint_end()
            self.assertEqual(model.surface_at("mines", target).kind, "1")
            self.assertFalse(model.surface_at("mines", target).armed)
            app.mode.set("Bridges")
            app.change_mode()
            self.assertTrue(app.bridge_controls.winfo_manager())
            self.assertEqual(len(app.canvas.find_withtag("bridges")),
                             sum(len(span.tiles) for span in model.bridge_spans()))
            self.assertTrue(all(model.terrains[name].kind == 2 for name in app.palette_names))
            bank = next(key for key, cell in model.cells.items()
                        if model.terrains[cell.terrain.upper()].kind == 2
                        and key not in model.blocked and not model.building_at(key))
            old_terrain = model.cells[bank].terrain
            app.bridge_action.set("Remove banks")
            with patch.object(app, "event_cell", return_value=bank):
                app.paint_start(type("Event", (), {})())
                app.paint_end()
            self.assertEqual(model.cells[bank].terrain, "EAU000")
            self.assertEqual(len(app.canvas.find_withtag("bridges")),
                             sum(len(span.tiles) for span in model.bridge_spans()))
            app.undo()
            self.assertEqual(model.cells[bank].terrain, old_terrain)
            app.redo()
            self.assertEqual(model.cells[bank].terrain, "EAU000")
            app.mode.set("Ground")
            app.change_mode()
            self.assertFalse(app.canvas.find_withtag("bridges"))
            with patch.object(ui.filedialog, "asksaveasfilename", return_value="unused.mdb"), \
                    patch.object(app, "_job") as job, patch.object(app.document, "save_copy") as save:
                app.save_dialog()
                captured = model.surface_rows()
                model.update_surface("mines", counter, armed=True)
                job.call_args.args[1]()
                self.assertEqual(save.call_args.args[4], captured)
        finally:
            app.close_documents()
            root.destroy()


if __name__ == "__main__":
    unittest.main()
