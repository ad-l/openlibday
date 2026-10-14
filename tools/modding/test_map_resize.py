import hashlib
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import map_editor_core as core
import scenario_scripts as scripts
import test_map_editor as existing


from testsupport import GREEN_SKIRMISH, ROOT, TOOLS, slow
SCRIPT = TOOLS / "map_tool.ps1"
REGISTRY = SCRIPT.with_name("terrain-types.csv")


class ResizeModelTests(unittest.TestCase):
    def model(self, objects=None):
        return existing.UnitTests().model(objects)

    def test_grow_shrink_history_and_original_coordinate_ids(self):
        model = self.model()
        before = model.state()
        original = model.rows()
        self.assertTrue(model.resize(10, "BORE000"))
        self.assertEqual((model.size, len(model.cells)), (10, 100))
        self.assertEqual(len({cell.identifier for cell in model.cells.values()}), 100)
        self.assertEqual(model.cells[9, 9].terrain, "BORE000")
        self.assertEqual([row for row in model.rows() if int(row["PosX"]) < 8 and int(row["PosY"]) < 8], original)
        model.begin_stroke()
        model.paint(9, 9, "NORD000")
        model.end_stroke()
        model.undo()
        self.assertEqual(model.cells[9, 9].terrain, "BORE000")
        model.undo()
        self.assertEqual(model.state(), before)
        self.assertFalse(model.dirty)
        model.redo()
        self.assertEqual(model.size, 10)
        self.assertTrue(model.resize(6, "NORD000"))
        self.assertEqual(len(model.cells), 36)
        model.resize(8, "BORE000")
        self.assertEqual({key: cell.identifier for key, cell in model.cells.items()}, model.original_cell_ids)
        history = len(model.undo_stack)
        self.assertFalse(model.resize(8, "BORE000"))
        self.assertEqual(len(model.undo_stack), history)
        for size in (0, 257, True, 1.5):
            with self.assertRaises(ValueError):
                model.resize(size, "NORD000")
        with self.assertRaises(ValueError):
            model.resize(9, "NORD116")

    def test_shrink_blocks_every_editable_and_preserved_spatial_surface(self):
        unit = core.Unit(100, 0, 7, 7, counter=1, origin_layer="units").row()
        reinforcement = core.Unit(101, 0, 7, 7, arrival=3, layer="reinforcements", counter=2,
                                  origin_layer="reinforcements").row()
        cases = [
            {"units": [unit]},
            {"reinforcements": [reinforcement]},
            {"mines": [core.Surface(1, "1", 7, 7).row("mines")]},
            {"roads": [core.Surface(1, "0", 7, 7).row("roads")]},
            {"walls": [core.Surface(1, "0", 7, 7, armor=60).row("walls")]},
            {"decors": [core.Surface(1, "BBRUI000", 7, 7).row("decors")]},
            {"deployments": [dict(Counter=1, player=0, PosX=7, PosY=7, Turn=4)]},
            {"blocked": [dict(x=7, y=7, reason="Unknown reservation")]},
            {"resizeReservations": [dict(table="HQ on Map", counter=1, x=7, y=7)]},
            {"resizeReservations": [dict(table="Counter Attack", counter=1, x=7, y=7)]},
            {"resizeReservations": [dict(table="Taurans On Map", counter=1, x=0, y=0)]},
        ]
        for data in (
                scripts.ScriptData(events=(scripts.SupportEvent(1, 4, 7, 7, 0, 0, 1),)),
                scripts.ScriptData(events=(scripts.SupportEvent(1, 0, 0, 0, 7, 7, 1),)),
                scripts.ScriptData(objective_positions=(scripts.PosTarget(1, 7, 7),)),
                scripts.ScriptData(order_positions=(scripts.PosTarget(1, 7, 7, 100),))):
            cases.append({"scripts": data.rows()})
        for objects in cases:
            with self.subTest(objects=objects):
                model = self.model(objects)
                before = model.state()
                with self.assertRaisesRegex(ValueError, "Cannot shrink"):
                    model.resize(7, "NORD000")
                self.assertEqual(model.state(), before)
                self.assertFalse(model.undo_stack)
        model = self.model()
        definition = next(kind for kind in model.building_types.values() if kind.width > 1)
        building = core.Building(1000001, definition.identifier, 0, definition.armor, 6, 6, False)
        model.buildings[building.identifier] = building
        with self.assertRaisesRegex(ValueError, "Building"):
            model.resize(7, "NORD000")

    def test_multitile_unit_cannot_be_cut_even_when_origin_is_inside(self):
        model = self.model()
        kind = next(kind for (kind, tech), definition in model.unit_types.items()
                    if tech == 0 and definition.width == definition.height == 3)
        unit = core.Unit(100, kind, 5, 5)
        model._set_unit(unit.identifier, unit)
        with self.assertRaisesRegex(ValueError, "Unit/reinforcement"):
            model.resize(7, "NORD000")

    def test_legacy_off_map_reservations_are_preserved_but_active_hq_blocks_crop(self):
        rows = [dict(table="HQ on Map", counter=1, x=20, y=2),
                dict(table="HQ on Map", counter=2, x=6, y=6)]
        model = self.model({"resizeReservations": rows})
        model.resize(10, "NORD000")
        model.resize(8, "NORD000")
        self.assertEqual(model.resize_reservations, rows)
        with self.assertRaisesRegex(ValueError, "HQ on Map row 2"):
            model.resize(6, "NORD000")


@slow
@unittest.skipUnless(os.name == "nt", "Requires native Jet and Tk")
class NativeResizeTests(unittest.TestCase):
    def open(self):
        document = core.Document.open(ROOT / "SCENS" / "SCEN1.ZIP", "s1-2-1.mdb",
                                      SCRIPT, REGISTRY, core.load_terrains(REGISTRY))
        self.addCleanup(document.close)
        return document

    def test_native_grow_crop_zip_noop_and_unrelated_table_preservation(self):
        document = self.open()
        model = document.model
        old_size = model.size
        digest = hashlib.sha256(document.source.read_bytes()).digest()
        before = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
        model.resize(old_size + 2, "BORE000")
        model.begin_stroke()
        model.paint(old_size + 1, old_size + 1, "NORD116", "trees")
        model.end_stroke()
        with tempfile.TemporaryDirectory() as work:
            folder = Path(work)
            for name in ("grown.mdb", "grown.zip"):
                output = folder / name
                document.save_copy(output, model.rows())
                saved = core.Document.open(output, document.member if output.suffix == ".zip" else None,
                                           SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertEqual(saved.model.size, old_size + 2)
                    self.assertEqual(saved.model.rows(), model.rows())
                    after = existing.WindowsBridgeTests.snapshot(saved.directory / "source.mdb")
                    self.assertTrue(all(after[table] == rows for table, rows in before.items() if table != "Map"))
                finally:
                    saved.close()
            model.resize(old_size + 1, "BORE000")
            cropped = folder / "cropped.mdb"
            document.save_copy(cropped, model.rows())
            saved = core.Document.open(cropped, None, SCRIPT, REGISTRY, model.terrains)
            try:
                self.assertEqual(saved.model.rows(), model.rows())
            finally:
                saved.close()
            model.resize(old_size, "NORD000")
            restored = folder / "restored.mdb"
            document.save_copy(restored, model.rows())
            self.assertEqual(existing.WindowsBridgeTests.snapshot(restored), before)
        self.assertEqual(hashlib.sha256(document.source.read_bytes()).digest(), digest)

    def test_forged_crop_rejected_by_native_validator_and_output_rolled_back(self):
        document = self.open()
        model = document.model
        original_size = model.size
        model.resize(original_size + 2, "BORE000")
        model.begin_stroke()
        model.paint_surface("mines", original_size + 1, original_size + 1, "0")
        model.end_stroke()
        rows = [row for row in model.rows()
                if int(row["PosX"]) < original_size + 1 and int(row["PosY"]) < original_size + 1]
        with tempfile.TemporaryDirectory() as work:
            output = Path(work) / "bad.mdb"
            with self.assertRaises(core.MapToolError):
                document.save_copy(output, rows)
            self.assertFalse(output.exists())
            rows = [row for row in model.rows() if int(row["PosX"]) < 1 and int(row["PosY"]) < 1]
            with self.assertRaises(core.MapToolError):
                document.save_copy(output, rows)
            self.assertFalse(output.exists())

    def test_registered_export_resized_catalog_and_objects_on_added_tiles(self):
        document = self.open()
        model = document.model
        catalog = ROOT / "SCENS" / "SKIRMISH.MDB"
        original = existing.WindowsBridgeTests.snapshot(catalog)
        digest = hashlib.sha256(catalog.read_bytes()).digest()
        size = model.size + 1
        model.resize(size, "NORD000")
        key = (size - 1, size - 1)
        model.begin_stroke()
        model.paint_surface("roads", *key, "0")
        model.paint_surface("mines", *key, "1", owner=2)
        model.end_stroke()
        with tempfile.TemporaryDirectory() as work:
            bundle = Path(work) / "resized"
            document.export_skirmish(
                bundle, catalog, document.source, rows=model.rows(), buildings=model.building_rows(),
                next_id=model.next_building_id, surfaces=model.surface_rows(), units=model.unit_rows(),
                next_unit_id=model.next_unit_id, scenario=model.scenario,
                deployments=model.deployment_rows(), scripts=model.scripts)
            exported = existing.WindowsBridgeTests.snapshot(bundle / "SCENS" / "SKIRMISH.MDB")
            old_ids = {row["Counter"] for row in original["Scenario Info"]}
            entries = [row for row in exported["Scenario Info"] if row["Counter"] not in old_ids]
            self.assertEqual(len(entries), 1)
            self.assertEqual(entries[0]["Size"], size)
            saved = core.Document.open(bundle / "SCENS" / "SCEN1.ZIP", entries[0]["FileName"],
                                       SCRIPT, REGISTRY, model.terrains)
            try:
                self.assertEqual(saved.model.rows(), model.rows())
                self.assertEqual(saved.model.surface_at("roads", key).kind, "0")
                self.assertEqual(saved.model.surface_at("mines", key).kind, "1")
            finally:
                saved.close()
        self.assertEqual(hashlib.sha256(catalog.read_bytes()).digest(), digest)

    def test_gui_terrain_view_and_resize_dialog_history_and_snapshot(self):
        import tkinter as tk
        import map_editor as ui

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        root.update_idletasks()
        try:
            document = self.open()
            app.activate_document(document)
            model = document.model
            original = model.state()
            app.terrain_only.set(True)
            app.render_map()
            self.assertFalse(app.canvas.find_withtag("building"))
            self.assertFalse(app.canvas.find_withtag("surface"))
            self.assertFalse(app.canvas.find_withtag("unit"))
            self.assertEqual(len(app.items), len(model.cells))
            tree = next(key for key, cell in model.cells.items() if model.terrains[cell.terrain.upper()].tree)
            self.assertTrue(app.terrains[app.display_terrain(tree)].paintable)
            self.assertEqual(app.canvas.itemcget(app.items[tree], "image"),
                             str(app.image(app.display_terrain(tree), app.zoom.get())))
            self.assertEqual(model.state(), original)
            app.mode.set("Units")
            app.change_mode()
            self.assertTrue(app.canvas.find_withtag("unit"))
            app.mode.set("Ground")
            app.change_mode()
            self.assertFalse(app.canvas.find_withtag("unit"))
            app.edit_scripts()
            app.script_dialog.notebook.select(app.script_dialog.pages["Objective"])
            root.update()
            app.script_dialog.start_pick("goal_tiles")
            self.assertFalse(app.terrain_view)
            self.assertTrue(app.canvas.find_withtag("unit"))
            app.terrain_only.set(True)
            app.change_terrain_view()
            self.assertIsNone(app.script_dialog.picking)
            self.assertFalse(app.canvas.find_withtag("unit"))
            app.script_dialog.close()
            size = model.size
            dialog = ui.ResizeDialog(app)
            dialog.size.set(str(size + 1))
            dialog.terrain.set("BORE000")
            dialog.apply()
            self.assertEqual(model.size, size + 1)
            self.assertEqual(len(app.items), (size + 1) ** 2)
            self.assertIn(f"{size + 1}x{size + 1}", app.heading.get())
            app.undo()
            self.assertEqual(model.state(), original)
            self.assertEqual(len(app.items), size ** 2)
            app.redo()
            with patch.object(ui.filedialog, "asksaveasfilename", return_value="unused.mdb"), \
                    patch.object(app, "_job") as job, patch.object(document, "save_copy") as save:
                app.save_dialog()
                captured = model.rows()
                model.resize(size + 2, "NORD000")
                job.call_args.args[1]()
                self.assertEqual(save.call_args.args[1], captured)
            dialog = ui.ResizeDialog(app)
            dialog.size.set("1")
            with patch.object(ui.messagebox, "showerror") as error:
                dialog.apply()
                error.assert_called_once()
            self.assertTrue(dialog.winfo_exists())
            dialog.destroy()
        finally:
            app.document = None
            app.executor.shutdown(wait=True)
            root.destroy()


if __name__ == "__main__":
    unittest.main()
