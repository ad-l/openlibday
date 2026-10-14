import copy
import csv
from dataclasses import replace
import hashlib
import json
import os
from pathlib import Path
import tempfile
import time
import unittest
from unittest.mock import patch
import zipfile

import map_editor_core as core
import scenario_scripts as scripts
import test_map_editor as existing
from scenario_script_editor import id_values, position_values


from testsupport import GREEN_SKIRMISH, ROOT, TOOLS, slow
SCRIPT = TOOLS / "map_tool.ps1"
REGISTRY = SCRIPT.with_name("terrain-types.csv")


class ScriptModelTests(unittest.TestCase):
    def model(self, data=None):
        data = data or scripts.ScriptData(objectives=(scripts.Objective(1, 0, 0),))
        unit = core.Unit(100, 4, 1, 1, armor=10, ap=120, counter=1, origin_layer="units")
        return existing.UnitTests().model({"units": [unit.row()], "nextUnitId": 200,
                                          "scripts": data.rows(), "fixedReferences": []})

    def test_registry_and_input_parsers(self):
        types = scripts.load_support_types(SCRIPT.with_name("support-types.csv"))
        self.assertEqual(set(types), set(range(27)))
        self.assertEqual(types[14].offsets, ((-1, -2), (2, 2), (-1, 1), (2, -2)))
        self.assertEqual(dict(types[14].properties)["unitIDDrop"], "4")
        self.assertIn("Human", types[4].label)
        self.assertIn("Tauran", types[11].label)
        self.assertIn("Shadamar", types[13].label)
        self.assertIn("150", types[4].description())
        self.assertEqual(id_values("1, 2\n3"), (1, 2, 3))
        self.assertEqual(position_values("1,2\n\n3 4"), ((1, 2), (3, 4)))
        for value in ("1.2,3", "1,2,3"):
            with self.assertRaises(ValueError):
                position_values(value)

    def test_reinforcement_order_derivation_matches_native_objective_dispatch(self):
        for objective, expected in ((0, 3), (1, 5), (2, 6), (3, 5), (4, 3), (5, 6),
                                    (6, 0), (7, 0), (8, 0), (9, 0), (10, 0)):
            data = scripts.ScriptData(objectives=(scripts.Objective(1, objective, 0),),
                                      objective_ids=(scripts.IdTarget(1, 100),),
                                      objective_positions=(scripts.PosTarget(1, 2, 3),))
            with self.subTest(objective=objective):
                self.assertEqual(scripts.reinforcement_order(data, True),
                                 (expected, (100,) if expected in (3, 6) else (),
                                  ((2, 3),) if expected == 5 else ()))
                self.assertEqual(scripts.reinforcement_order(data, False), (0, (), ()))
        data = replace(data, objectives=(scripts.Objective(1, 0, 0),), objective_ids=())
        self.assertEqual(scripts.reinforcement_order(data, True), (5, (), ((2, 3),)))
        self.assertIsNone(scripts.reinforcement_order(scripts.ScriptData(), True))
        self.assertEqual(scripts.reinforcement_order(None, False), (0, (), ()))

    def test_lossless_anomalies_enumeration_and_counter_reconciliation(self):
        data = scripts.ScriptData(
            objectives=(scripts.Objective(7, 0, 0),),
            objective_ids=(scripts.IdTarget(10, 100), scripts.IdTarget(3, 100), scripts.IdTarget(6, 101)),
            order_positions=(scripts.PosTarget(8, 0, 0, 999),))
        self.assertEqual(scripts.ScriptData.from_rows(data.rows()), data)
        self.assertEqual(data.targets("objective_ids", (101, 100, 100)), data)
        updated = data.targets("objective_ids", (100, 102))
        self.assertEqual(updated.objective_ids, (data.objective_ids[0], scripts.IdTarget(-1, 102)))
        self.assertEqual(updated.order_positions, data.order_positions)
        self.assertEqual(updated.next_counter("objective_ids"), -2)
        rows = data.rows()
        rows["Objective"].append(rows["Objective"][0])
        with self.assertRaises(ValueError):
            scripts.ScriptData.from_rows(rows)

    def test_support_validation_overlap_and_history(self):
        model = self.model()
        before = model.state()
        event = model.add_support(4, 1, 1, 3)
        self.assertEqual((event.x, event.y), (model.units[100].x, model.units[100].y))
        model.update_support(event.counter, turn=4)
        model.delete_support(event.counter)
        for _ in range(3):
            model.undo()
        self.assertEqual(model.state(), before)
        model.redo()
        for kind, turn, x, destination in ((15, 1, 1, 1), (14, 0, 1, 1), (14, 100, 1, 1),
                                           (4, 1, 8, 8), (4, 1, 1, 2)):
            with self.subTest(kind=kind, turn=turn, x=x), self.assertRaises(ValueError):
                model.add_support(kind, x, 1, turn, destination, 1)
        transport = model.add_support(0, 1, 1, 2, 4, 5)
        self.assertEqual((transport.dest_x, transport.dest_y), (4, 5))
        with self.assertRaises(ValueError):
            model.update_support(10000, turn=2)

    def test_atomic_orders_references_and_category_conversion(self):
        model = self.model()
        target = model.add_unit(4, 3, 3)
        before = model.state()
        history = len(model.undo_stack)
        model.set_unit_order(100, 6, 12, 50, 4, (target.identifier,), ())
        self.assertEqual(len(model.undo_stack), history + 1)
        self.assertEqual(model.units[100].order_id, 6)
        with self.assertRaises(ValueError):
            model.delete_unit(target.identifier)
        model.undo()
        self.assertEqual(model.state(), before)
        model.redo()
        model.set_unit_order(100, 0, 12, 100, 65535, (), ())
        self.assertNotIn(target.identifier, model.unit_references)
        model.update_unit(100, arrival=3)
        model.undo()
        model.set_unit_order(100, 1, 5, 100, 65535, (), ((1, 2),))
        with self.assertRaises(ValueError):
            model.update_unit(100, arrival=3)
        for probability in (-1, 101, True):
            with self.assertRaises(ValueError):
                model.set_unit_order(100, 1, 5, probability, 65535, (), ((1, 2),))
        with self.assertRaises(ValueError):
            model.set_unit_order(100, 1, 5, 100, 65535, (target.identifier,), ())

    def test_objectives_noop_and_preserve_only_modes(self):
        model = self.model()
        before = model.state()
        model.set_objective(0, 0, (), ())
        self.assertEqual(model.state(), before)
        self.assertEqual(len(model.undo_stack), 0)
        model.set_objective(1, 3, (), ((2, 2),))
        self.assertIn("Raw hold threshold", [row[1] for row in scripts.timeline(model)])
        model.set_objective(5, 3, (100,), ((2, 2),))
        with self.assertRaises(ValueError):
            model.delete_unit(100)
        with self.assertRaises(ValueError):
            model.set_objective(5, 4, (100,), ((2, 2),))
        for kind in (7, 8, 9, 10):
            advanced = self.model(scripts.ScriptData(objectives=(scripts.Objective(1, kind, 0),)))
            with self.assertRaises(ValueError):
                advanced.set_objective(0, 0, (), ())
            self.assertTrue(any("preserve-only" in warning for warning in scripts.diagnostics(advanced)))

    def test_new_targets_reject_duplicates_and_absent_ids(self):
        model = self.model()
        for ids, positions in (((100, 100), ()), ((999,), ()), ((), ((1, 1), (1, 1))),
                               ((), ((-1, 1),))):
            with self.subTest(ids=ids, positions=positions), self.assertRaises(ValueError):
                model.set_objective(0, 0, ids, positions)
        original = scripts.ScriptData(objectives=(scripts.Objective(1, 0, 0),),
                                      events=(scripts.SupportEvent(1, 15, 0, 0, 0, 0, 1),))
        model = self.model(original)
        with self.assertRaises(ValueError):
            model.delete_support(1)
        model.add_support(4, 1, 1, 2)
        self.assertEqual(model.scripts.events[0], original.events[0])


@slow
@unittest.skipUnless(os.name == "nt", "Requires Windows Jet and Tk")
class ScriptPersistenceTests(unittest.TestCase):
    def open(self, member="s1-2-1.mdb", archive="SCEN1.ZIP"):
        document = core.Document.open(ROOT / "SCENS" / archive, member, SCRIPT, REGISTRY,
                                      core.load_terrains(REGISTRY))
        self.addCleanup(document.close)
        return document

    def test_native_objective_overlay_artwork_footprints_toggle_zoom_and_history(self):
        import tkinter as tk
        import map_editor as ui
        from types import SimpleNamespace

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        root.update_idletasks()
        try:
            document = self.open()
            model = document.model
            app.activate_document(document)
            unit = next(unit for unit in model.units.values() if unit.layer == "units")
            building = next(iter(model.buildings.values()))
            multi_type = next(kind for (kind, tech), definition in model.unit_types.items()
                              if tech == 0 and definition.width == definition.height == 3)
            later = replace(unit, identifier=model.next_unit_id, kind=multi_type, arrival=3,
                            layer="reinforcements", x=3, y=3)
            model._set_unit(later.identifier, later)
            target_ids = (unit.identifier, building.identifier, later.identifier)
            tiles = ((1, 1), (2, 1))
            initial = model.state()
            model.set_objective(0, model.scripts.objectives[0].turn, target_ids, tiles)
            app.render_map()
            self.assertFalse(app.canvas.find_withtag("scripting"))
            app.show_scripts.set(True)
            app.render_scripts()
            self.assertEqual(len(app.canvas.find_withtag("objective_tile")), 2)
            self.assertEqual(len(app.canvas.find_withtag("objective_id")), 2)
            for zoom, (multiply, divide) in ui.ZOOMS.items():
                app.zoom.set(zoom)
                app.render_map()
                scale = multiply / divide
                tile = app.canvas.find_withtag("objective_tile")[0]
                self.assertEqual(app.canvas.type(tile), "image")
                self.assertEqual(app.canvas.itemcget(tile, "image"),
                                 str(app.image("OBJT000", zoom, layer="deployment")))
                for entity, footprint in ((unit, model.unit_footprint(unit)),
                                          (building, model.footprint(building))):
                    marker = app.canvas.find_withtag(f"objective_id_{entity.identifier}")[0]
                    cx, cy = app.center(
                        (min(x for x, _ in footprint) + max(x for x, _ in footprint)) / 2,
                        (min(y for _, y in footprint) + max(y for _, y in footprint)) / 2)
                    self.assertEqual(app.canvas.coords(marker), [cx - 27 * scale, cy - 42 * scale])
                    self.assertEqual(app.canvas.itemcget(marker, "image"),
                                     str(app.image("OBJT001", zoom, layer="deployment")))
                    self.assertGreater(marker, max(app.items.values()))
            app.unit_view.set("Turn 3")
            app.render_map()
            self.assertFalse(app.canvas.find_withtag(f"objective_id_{unit.identifier}"))
            self.assertTrue(app.canvas.find_withtag(f"objective_id_{later.identifier}"))
            self.assertTrue(app.canvas.find_withtag(f"objective_id_{building.identifier}"))
            app.undo()
            self.assertEqual(model.state(), initial)
            app.redo()
            self.assertEqual(len(app.canvas.find_withtag("objective_tile")), 2)
            before = model.state()
            app.show_scripts.set(False)
            app.render_scripts()
            self.assertFalse(app.canvas.find_withtag("scripting"))
            app.script_dialog = SimpleNamespace(picking="tiles", selected_order=None,
                                               selected_event=None, preview_tiles=lambda: ((4, 4),),
                                               active_tab="Timeline / diagnostics")
            app.render_scripts()
            self.assertTrue(app.canvas.find_withtag("scripting"))
            self.assertFalse(app.canvas.find_withtag("objective_tile"))
            self.assertFalse(app.canvas.find_withtag("objective_id"))
            app.script_dialog = None
            self.assertEqual(model.state(), before)
        finally:
            app.script_dialog = None
            app.document = None
            app.executor.shutdown(wait=True)
            root.destroy()

    def test_native_noop_preserves_every_table_and_blob_including_anomalies(self):
        for member, archive in (("s1-2-1.mdb", "SCEN1.ZIP"), ("s1-4-1.mdb", "SCEN1.ZIP"),
                                ("s4-15-2.mdb", "SCEN2.ZIP"), ("dmc3-12.mdb", "DEATHM.ZIP")):
            with self.subTest(member=member), tempfile.TemporaryDirectory() as work:
                document = self.open(member, archive)
                before = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
                output = Path(work) / "noop.mdb"
                document.save_copy(output, document.model.rows())
                self.assertEqual(existing.WindowsBridgeTests.snapshot(output), before)

    def test_active_script_tab_filters_targets_and_stops_stale_picking(self):
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
            units = [unit for unit in model.units.values() if unit.layer == "units"]
            selected, other = units[:2]
            model.set_objective(0, model.scripts.objectives[0].turn, (selected.identifier,), ((1, 1),))
            model.set_unit_order(selected.identifier, 1, 99, 100, 65535, (), ((2, 2),))
            model.set_unit_order(other.identifier, 1, 99, 100, 65535, (), ((3, 3),))
            event = model.add_support(14, 8, 8, 4)
            app.show_scripts.set(True)
            app.edit_scripts()
            dialog = app.script_dialog
            dialog.selected_order = selected.identifier
            dialog.selected_event = event.counter
            root.update()
            original = model.state()

            def select(tab):
                dialog.notebook.select(dialog.pages[tab])
                root.update()

            def counts():
                return tuple(bool(app.canvas.find_withtag(tag)) for tag in
                             ("objective_id", "objective_tile", "support_target", "order_target"))

            select("Objective")
            self.assertEqual(counts(), (True, True, False, False))
            select("Scheduled support")
            self.assertEqual(counts(), (False, False, True, False))
            self.assertGreater(len(app.canvas.find_withtag("support_target")), len(model.scripts.events) * 2)
            select("AI orders")
            self.assertEqual(counts(), (False, False, False, True))
            sources = app.canvas.find_withtag("order_source")
            self.assertEqual(len(sources), len(model.unit_footprint(selected)))
            self.assertEqual({tuple(app.canvas.coords(item)) for item in sources},
                             {app.diamond(*key) for key in model.unit_footprint(selected)})
            self.assertTrue(all(app.canvas.itemcget(item, "outline") == "#70df93" for item in sources))
            markers = app.canvas.find_withtag("order_target")
            self.assertEqual(len(markers), 1)
            self.assertEqual(app.canvas.coords(markers[0]), list(app.diamond(2, 2)))
            dialog.order_unit.set(dialog.order_label(other))
            dialog.choose_order_unit()
            self.assertEqual({tuple(app.canvas.coords(item)) for item in app.canvas.find_withtag("order_source")},
                             {app.diamond(*key) for key in model.unit_footprint(other)})
            marker = app.canvas.find_withtag("order_target")[0]
            self.assertEqual(app.canvas.coords(marker), list(app.diamond(3, 3)))
            model.set_unit_order(other.identifier, 6, 99, 100, 65535, (selected.identifier,), ())
            app.render_scripts()
            markers = app.canvas.find_withtag("order_target")
            self.assertEqual(len(markers), 2)
            self.assertEqual(app.canvas.itemcget(markers[1], "text"), f"ID {selected.identifier}")
            dialog.selected_order = selected.identifier
            dialog.fill_order()
            dialog.start_pick("order_tiles")
            self.assertEqual(dialog.picking, "order_tiles")
            select("Objective")
            self.assertIsNone(dialog.picking)
            self.assertFalse(app.canvas.find_withtag("order_source"))
            self.assertEqual(counts(), (True, True, False, False))
            select("Timeline / diagnostics")
            self.assertEqual(counts(), (True, True, True, True))
            select("AI orders")
            dialog.selected_order = None
            app.render_scripts()
            self.assertEqual(counts(), (False, False, False, False))
            self.assertFalse(app.canvas.find_withtag("order_source"))
            app.show_scripts.set(False)
            select("Objective")
            self.assertFalse(app.canvas.find_withtag("scripting"))
            app.show_scripts.set(True)
            select("Scheduled support")
            dialog.close()
            self.assertEqual(counts()[:3], (True, True, True))
            model.undo()
            self.assertEqual(model.state(), original)
        finally:
            if app.script_dialog:
                app.script_dialog.close()
            app.document = None
            app.executor.shutdown(wait=True)
            root.destroy()

    def test_reinforcement_orders_gui_preview_flags_history_overlay_and_native_save(self):
        import tkinter as tk
        import map_editor as ui

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        root.update_idletasks()
        try:
            document = self.open()
            model = document.model
            app.activate_document(document)
            original = model.state()
            before = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
            reinforcement = next(unit for unit in model.units.values()
                                 if unit.layer == "reinforcements" and unit.arrival_order)
            app.show_scripts.set(True)
            app.edit_scripts()
            dialog = app.script_dialog
            root.update()
            dialog.notebook.select(dialog.pages["AI orders"])
            root.update()
            dialog.order_unit.set(dialog.order_label(reinforcement))
            dialog.choose_order_unit()
            self.assertIn("reinforcement turn", dialog.order_unit.get())
            self.assertEqual(app.arrival_value(app.unit_view.get()), reinforcement.arrival)
            self.assertEqual(dialog.selected_order, reinforcement.identifier)
            self.assertTrue(app.canvas.find_withtag("order_source"))
            derived = scripts.reinforcement_order(model.scripts, True)
            self.assertEqual(dialog.order_kind.get(), f"{derived[0]}: {scripts.ORDERS[derived[0]]}")
            self.assertEqual(id_values(dialog.value(dialog.order_ids)), derived[1])
            self.assertEqual(position_values(dialog.value(dialog.order_positions)), derived[2])
            self.assertEqual(len(app.canvas.find_withtag("order_target")),
                             sum((x, y) in model.cells for x, y in derived[2]) +
                             2 * sum(identifier in model.units or identifier in model.buildings
                                     for identifier in derived[1]))
            for widget in (dialog.order_kind_box, dialog.order_turn_box, dialog.order_probability_box,
                           dialog.order_enemy_box, dialog.order_ids, dialog.order_positions):
                self.assertEqual(str(widget.cget("state")), "disabled")
            for name in ("Pick target IDs", "Paint target tiles", "Clear targets"):
                self.assertIn("disabled", dialog.order_buttons[name].state())
            self.assertNotIn("disabled", dialog.order_buttons["Apply order"].state())
            self.assertEqual((dialog.order_turn.get(), dialog.order_probability.get()), ("99", "100"))
            self.assertEqual(model.state(), original)
            with self.assertRaises(ValueError):
                model.set_unit_order(reinforcement.identifier, 1, 99, 100, 65535, (), ((2, 2),))
            dialog.reinforcement_order.set(False)
            dialog.preview_reinforcement_order()
            self.assertEqual(dialog.order_kind.get(), "0: No Order")
            self.assertEqual(model.state(), original)
            dialog.apply_order()
            self.assertFalse(model.units[reinforcement.identifier].arrival_order)
            self.assertFalse(app.canvas.find_withtag("order_target"))
            self.assertTrue(app.canvas.find_withtag("order_source"))
            app.undo()
            self.assertEqual(model.state(), original)
            self.assertTrue(dialog.reinforcement_order.get())
            app.redo()
            self.assertFalse(dialog.reinforcement_order.get())
            with tempfile.TemporaryDirectory() as work:
                output = Path(work) / "reinforcement-order.mdb"
                document.save_copy(output, model.rows())
                saved = core.Document.open(output, None, SCRIPT, REGISTRY, model.terrains)
                try:
                    self.assertFalse(saved.model.units[reinforcement.identifier].arrival_order)
                    after = existing.WindowsBridgeTests.snapshot(output)
                    self.assertTrue(all(after[table] == rows for table, rows in before.items()
                                        if table != "Reinforcement"))
                    old_rows = before["Reinforcement"]
                    new_rows = after["Reinforcement"]
                    self.assertEqual(len(old_rows), len(new_rows))
                    for old, new in zip(old_rows, new_rows):
                        expected = dict(old)
                        if old["UnitID"] == reinforcement.identifier:
                            expected["order"] = False
                        self.assertEqual(new, expected)
                finally:
                    saved.close()
                model.update_unit(reinforcement.identifier, arrival_order=True)
                enabled = Path(work) / "reenabled-order.mdb"
                document.save_copy(enabled, model.rows())
                self.assertEqual(existing.WindowsBridgeTests.snapshot(enabled), before)
            initial = next(unit for unit in model.units.values() if unit.layer == "units")
            dialog.order_unit.set(dialog.order_label(initial))
            dialog.choose_order_unit()
            self.assertEqual(str(dialog.order_kind_box.cget("state")), "readonly")
            self.assertFalse(dialog.reinforcement_controls.winfo_manager())
            dialog.start_pick("unit")
            app.unit_view.set(f"Turn {reinforcement.arrival}")
            dialog.map_start((reinforcement.x, reinforcement.y))
            self.assertEqual(dialog.selected_order, reinforcement.identifier)
        finally:
            if app.script_dialog:
                app.script_dialog.close()
            app.document = None
            app.executor.shutdown(wait=True)
            root.destroy()

    def test_script_edits_new_entity_references_mdb_zip_and_registered_export(self):
        document = self.open()
        model = document.model
        before = existing.WindowsBridgeTests.snapshot(document.directory / "source.mdb")
        target = model.add_unit(2, *existing.WindowsBridgeTests.unit_spot(model))
        building_type = next(kind for kind in model.building_types.values()
                             if kind.width == kind.height == 1 and kind.armor > 0)
        for x, y in model.cells:
            candidate = core.Building(model.next_building_id, building_type.identifier, 0,
                                      building_type.armor, x, y, building_type.ai_target)
            try:
                model.validate_building(candidate)
            except ValueError:
                continue
            building = model.add_building(building_type.identifier, 0, x, y)
            break
        else:
            self.fail("No free building tile")
        event = model.add_support(4, building.x, building.y, 3)
        ordered = next(unit for unit in model.units.values() if unit.layer == "units" and unit.order_id == 0)
        model.set_unit_order(ordered.identifier, 1, 12, 50, 4, (), ((0, 0), (1, 1)))
        old_goal = model.scripts.objectives[0]
        model.set_objective(5, old_goal.turn, (target.identifier, building.identifier),
                            tuple((row.x, row.y) for row in model.scripts.objective_positions))
        source_hash = hashlib.sha256(document.source.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as work:
            folder = Path(work)
            database = folder / "edited.mdb"
            document.save_copy(database, model.rows())
            after = existing.WindowsBridgeTests.snapshot(database)
            permitted = {"Automated Units", "Objective", "Objectives ID List", "Unit Order Pos List",
                         "Units On Map", "Buildings On Map", "Scenario Info"}
            self.assertTrue(all(after[table] == rows for table, rows in before.items() if table not in permitted))
            self.assertIn(target.identifier, [row["Id"] for row in after["Objectives ID List"]])
            self.assertIn(building.identifier, [row["Id"] for row in after["Objectives ID List"]])
            added = [row for row in after["Automated Units"] if row not in before["Automated Units"]]
            self.assertEqual(len(added), 1)
            self.assertEqual(added[0]["StartX"], event.x)
            changed = next(row for row in after["Units On Map"] if row["ID"] == ordered.identifier)
            self.assertEqual((changed["OrderID"], changed["Turn"], changed["Probability"], changed["EnemyUnitType"]),
                             (1, 12, 50, 4))
            archive = folder / "edited.zip"
            document.save_copy(archive, model.rows())
            with zipfile.ZipFile(document.source) as original, zipfile.ZipFile(archive) as saved:
                for name in original.namelist():
                    if name != document.member:
                        self.assertEqual(original.read(name), saved.read(name))
            bundle = folder / "skirmish"
            document.export_skirmish(
                bundle, ROOT / "SCENS" / "SKIRMISH.MDB", document.source,
                rows=model.rows(), buildings=model.building_rows(), next_id=model.next_building_id,
                surfaces=model.surface_rows(), units=model.unit_rows(), next_unit_id=model.next_unit_id,
                scenario=model.scenario, deployments=model.deployment_rows(), scripts=model.scripts)
            entries = bundle / "catalog.json"
            core.run_map_tool(SCRIPT, Action="ExportCatalog", Source=bundle / "SCENS" / "SKIRMISH.MDB",
                              Output=entries)
            member = next(row["FileName"] for row in json.loads(entries.read_text(encoding="utf-8-sig"))
                          if row["FileName"].startswith("edit"))
            reopened = core.Document.open(bundle / "SCENS" / "SCEN1.ZIP", member, SCRIPT, REGISTRY, model.terrains)
            try:
                self.assertEqual(reopened.model.scripts.events[-1].kind, 4)
                self.assertEqual(reopened.model.scripts.objectives[0].kind, 5)
                self.assertEqual({row.identifier for row in reopened.model.scripts.objective_ids},
                                 {target.identifier, building.identifier})
            finally:
                reopened.close()
        self.assertEqual(hashlib.sha256(document.source.read_bytes()).digest(), source_hash)

    def test_native_bridge_malformed_scripts_roll_back_output(self):
        document = self.open()
        rows = document.model.scripts.rows()
        malformed = []
        for field, value in (("Counter", 999999), ("TurnArrival", 0), ("AutomatedType", 15),
                             ("StartX", -1), ("StartY", True), ("DestX", 2147483648)):
            candidate = copy.deepcopy(rows)
            candidate["Automated Units"][0][field] = value
            malformed.append(candidate)
        candidate = copy.deepcopy(rows)
        candidate["Objectives ID List"].append({"Counter": -1, "Id": 999999})
        malformed.append(candidate)
        candidate = copy.deepcopy(rows)
        candidate["Objective"] = []
        malformed.append(candidate)
        candidate = copy.deepcopy(rows)
        candidate["Extra"] = []
        malformed.append(candidate)
        with tempfile.TemporaryDirectory() as work:
            folder = Path(work)
            terrain = folder / "terrain.csv"
            with terrain.open("w", newline="", encoding="utf-8-sig") as stream:
                writer = csv.DictWriter(stream, fieldnames=("Id", "PosX", "PosY", "TerrainImg"))
                writer.writeheader()
                writer.writerows(document.model.rows())
            for index, data in enumerate(malformed):
                path = folder / "scripts.json"
                path.write_text(json.dumps(data), encoding="utf-8")
                output = folder / f"bad-{index}.mdb"
                with self.subTest(index=index), self.assertRaises(core.MapToolError):
                    core.run_map_tool(SCRIPT, Action="Apply", Source=document.directory / "source.mdb",
                                      Output=output, Csv=terrain, TerrainCsv=REGISTRY, ScriptJson=path)
                self.assertFalse(output.exists())

    def test_gui_map_picking_history_busy_snapshot_and_alternate_lifecycle(self):
        import tkinter as tk
        import map_editor as ui

        root = tk.Tk()
        root.withdraw()
        app = ui.MapEditor(root, ROOT)
        def finish():
            deadline = time.monotonic() + 40
            while app.busy and time.monotonic() < deadline:
                root.update()
                time.sleep(.01)
            self.assertFalse(app.busy)
        try:
            with patch.object(ui.messagebox, "showerror") as errors:
                app.open_source(ROOT / "SCENS" / "SCEN1.ZIP", "s1-2-1.mdb")
                finish()
                self.assertFalse(app.show_scripts.get())
                app.edit_scripts()
                dialog, model = app.script_dialog, app.document.model
                self.assertEqual(len(dialog.notebook.tabs()), 4)
                self.assertFalse(app.canvas.find_withtag("scripting"))
                terrain = tuple(cell.terrain for cell in model.cells.values())
                before = model.state()
                dialog.new_event()
                dialog.start_pick("source")
                dialog.map_start((5, 5))
                self.assertGreaterEqual(len(app.canvas.find_withtag("scripting")), 5)
                app.paint_end()
                dialog.apply_event()
                self.assertEqual(len(model.scripts.events), len(before[-1].events) + 1)
                app.undo()
                self.assertEqual(model.state(), before)
                app.redo()
                dialog.selected_order = next(unit.identifier for unit in model.units.values()
                                             if unit.layer == "units" and unit.order_id == 0)
                dialog.fill_order()
                dialog.order_kind.set("1: Goto")
                dialog.put(dialog.order_ids, "")
                dialog.put(dialog.order_positions, "")
                dialog.start_pick("order_tiles")
                with patch.object(app, "event_cell", return_value=(5, 5)):
                    app.paint_start(type("Event", (), {})())
                with patch.object(app, "event_cell", return_value=(7, 5)):
                    app.paint_drag(type("Event", (), {})())
                app.paint_end()
                self.assertEqual(position_values(dialog.value(dialog.order_positions)), ((5, 5), (6, 5), (7, 5)))
                dialog.apply_order()
                self.assertEqual(model.units[dialog.selected_order].order_id, 1)
                dialog.goal_kind.set("1: Hold Position")
                dialog.goal_turn.set("3")
                dialog.put(dialog.goal_ids, "")
                dialog.put(dialog.goal_positions, "3,3")
                dialog.apply_goal()
                self.assertEqual(model.scripts.objectives[0].kind, 1)
                self.assertEqual(tuple(cell.terrain for cell in model.cells.values()), terrain)
                app.busy = True
                state = model.state()
                dialog.apply_goal()
                self.assertEqual(model.state(), state)
                self.assertEqual(errors.call_count, 1)
                errors.reset_mock()
                app.busy = False
                with patch.object(ui.filedialog, "asksaveasfilename", return_value="unused.mdb"), \
                        patch.object(app, "_job") as job, patch.object(app.document, "save_copy") as save:
                    app.save_dialog()
                    saved_scripts = model.scripts
                    model.add_support(4, 2, 2, 9)
                    job.call_args.args[1]()
                    self.assertEqual(save.call_args.args[-1], saved_scripts)
                alternate = core.Document.open(ROOT / "SCENS" / "SCEN2.ZIP", "s1-2-2.mdb",
                                               SCRIPT, REGISTRY, model.terrains)
                app.variant_documents[2] = alternate
                app.activate_document(alternate)
                self.assertIsNone(app.script_dialog)
                self.assertFalse(dialog.winfo_exists())
                self.assertFalse(app.show_scripts.get())
                self.assertEqual(model.scripts.objectives[0].kind, 1)
                self.assertNotEqual(alternate.model.scripts, model.scripts)
                errors.assert_not_called()
        finally:
            app.close_documents()
            app.executor.shutdown(wait=True)
            root.destroy()


if __name__ == "__main__":
    unittest.main()
