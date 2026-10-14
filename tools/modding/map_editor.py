"""Windows desktop scenario editor. Launch with: python tools\\modding\\map_editor.py --game-root <install>."""

from __future__ import annotations

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from dataclasses import replace
import csv
import ctypes
import math
import os
from pathlib import Path
import subprocess
import tkinter as tk
from tkinter import filedialog, messagebox, ttk
import zipfile

from map_editor_core import (
    MINE_FLAGS,
    Building, Document, MapToolError, ScenarioSettings, SKIRMISH_DIFFICULTIES, TerrainArt, Texture, Unit,
    alternate_map_source, archive_members, campaign_map_variant, line_cells, skirmish_difficulty, skirmish_unit_types,
    load_building_types, load_surface_types, load_terrains, load_unit_types, tile_at,
)
from scenario_script_editor import ScriptDialog, integer
from scenario_scripts import reinforcement_order
import registries


ERRORS = (OSError, ValueError, KeyError, csv.Error, zipfile.BadZipFile,
          zipfile.LargeZipFile, MapToolError, subprocess.TimeoutExpired, tk.TclError)
ZOOMS = {"50%": (1, 2), "100%": (1, 1), "200%": (2, 1), "300%": (3, 1)}
SURFACE_MODES = {"Roads": "roads", "Walls": "walls", "Decors": "decors", "Mines": "mines"}
ARRIVAL_TURNS = ("Start", *(f"Turn {turn}" for turn in range(1, 101)))


class ArchivePicker(tk.Toplevel):
    def __init__(self, parent: tk.Tk, source: Path, members: list[str]):
        super().__init__(parent)
        self.title(f"Choose scenario - {source.name}")
        self.geometry("640x620")
        self.transient(parent)
        self.result: str | None = None
        self.members = members
        self.filtered = members
        self.search = tk.StringVar()
        ttk.Label(self, text="Choose an MDB scenario from this archive.").pack(padx=16, pady=12)
        entry = ttk.Entry(self, textvariable=self.search)
        entry.pack(fill="x", padx=16)
        frame = ttk.Frame(self)
        frame.pack(fill="both", expand=True, padx=16, pady=12)
        self.listbox = tk.Listbox(frame, exportselection=False)
        scroll = ttk.Scrollbar(frame, command=self.listbox.yview)
        self.listbox.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        self.listbox.pack(fill="both", expand=True)
        controls = ttk.Frame(self)
        controls.pack(fill="x", padx=16, pady=(0, 16))
        ttk.Button(controls, text="Cancel", command=self.destroy).pack(side="right", padx=6)
        ttk.Button(controls, text="Open scenario", command=self.accept).pack(side="right")
        self.search.trace_add("write", self.refresh)
        self.listbox.bind("<Double-Button-1>", lambda _: self.accept())
        self.bind("<Return>", lambda _: self.accept())
        self.bind("<Escape>", lambda _: self.destroy())
        self.refresh()
        self.grab_set()
        entry.focus_set()

    def refresh(self, *_):
        query = self.search.get().strip().lower()
        self.filtered = [name for name in self.members if query in name.lower()]
        self.listbox.delete(0, "end")
        for name in self.filtered:
            self.listbox.insert("end", name)
        if self.filtered:
            self.listbox.selection_set(0)

    def accept(self):
        selection = self.listbox.curselection()
        if selection:
            self.result = self.filtered[selection[0]]
            self.destroy()


class ScenarioDialog(tk.Toplevel):
    def __init__(self, editor):
        super().__init__(editor.root)
        self.editor = editor
        self.original = editor.document.model.scenario
        self.title("Scenario settings")
        self.geometry("760x740")
        self.minsize(650, 620)
        self.transient(editor.root)
        self.name = tk.StringVar(value=self.original.title)
        self.turns = tk.StringVar(value=str(self.original.turns))
        self.points = tk.StringVar(value=str(self.original.points))
        self.difficulty = tk.StringVar()
        self.search = tk.StringVar()
        self.enabled = {kind for kind, enabled in self.original.available if enabled}
        self.visible = []
        body = ttk.Frame(self, padding=16)
        body.pack(fill="both", expand=True)
        fields = ttk.Frame(body)
        fields.pack(fill="x")
        fields.columnconfigure(1, weight=1)
        for row, (caption, variable) in enumerate((("Scenario name (English)", self.name),
                                                  ("Turn limit (1..100)", self.turns),
                                                  ("Deployment budget (native points)", self.points))):
            ttk.Label(fields, text=caption).grid(row=row, column=0, sticky="w", padx=(0, 12), pady=5)
            ttk.Entry(fields, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=5)
        ttk.Label(fields, text="Difficulty preset").grid(row=3, column=0, sticky="w", pady=5)
        self.difficulty_box = ttk.Combobox(fields, textvariable=self.difficulty,
                                           values=list(SKIRMISH_DIFFICULTIES), state="readonly")
        self.difficulty_box.grid(row=3, column=1, sticky="ew", pady=5)
        self.difficulty_box.bind("<<ComboboxSelected>>", self.choose_difficulty)
        ttk.Label(body, text="The game derives difficulty from points: Easy <=300, Average <=500, "
                  "Hard <=1000, Very hard >1000.\nA preset changes the deployment budget. "
                  "The game derives a separate technology allocation from this value.",
                  wraplength=700).pack(anchor="w", pady=(6, 12))
        notebook = ttk.Notebook(body)
        notebook.pack(fill="both", expand=True)
        briefing_page = ttk.Frame(notebook, padding=10)
        roster_page = ttk.Frame(notebook, padding=10)
        notebook.add(briefing_page, text="Briefing / objective text")
        notebook.add(roster_page, text="Available army units")
        ttk.Label(briefing_page, text="English briefing and objective text only. "
                  "Actual win-condition rules and other languages are preserved.",
                  wraplength=660).pack(anchor="w", pady=(0, 8))
        text_frame = ttk.Frame(briefing_page)
        text_frame.pack(fill="both", expand=True)
        self.briefing = tk.Text(text_frame, wrap="word", undo=True, height=10)
        text_scroll = ttk.Scrollbar(text_frame, command=self.briefing.yview)
        self.briefing.configure(yscrollcommand=text_scroll.set)
        text_scroll.pack(side="right", fill="y")
        self.briefing.pack(fill="both", expand=True)
        self.initial_briefing = self.original.briefing.replace("##", "\n")
        self.briefing.insert("1.0", self.initial_briefing)
        ttk.Label(roster_page, text="Click to toggle availability. These are choices in the game's "
                  "army setup, not units placed on the map.", wraplength=660).pack(anchor="w", pady=(0, 8))
        ttk.Entry(roster_page, textvariable=self.search).pack(fill="x", pady=(0, 8))
        list_frame = ttk.Frame(roster_page)
        list_frame.pack(fill="both", expand=True)
        self.roster = tk.Listbox(list_frame, selectmode="multiple", exportselection=False, height=10)
        roster_scroll = ttk.Scrollbar(list_frame, command=self.roster.yview)
        self.roster.configure(yscrollcommand=roster_scroll.set)
        roster_scroll.pack(side="right", fill="y")
        self.roster.pack(fill="both", expand=True)
        self.roster.bind("<<ListboxSelect>>", self.remember_selection)
        self.count = tk.StringVar()
        ttk.Label(roster_page, textvariable=self.count).pack(anchor="w", pady=(6, 0))
        ttk.Label(body, text="Save copy preserves the source. Skirmish export also creates matching "
                  "SCENS\\SKIRMISH.MDB and SCEN1.ZIP copies; nothing is installed automatically.",
                  wraplength=700).pack(anchor="w", pady=10)
        controls = ttk.Frame(body)
        controls.pack(fill="x")
        ttk.Button(controls, text="Cancel", command=self.destroy).pack(side="right", padx=(8, 0))
        ttk.Button(controls, text="Apply", command=self.accept).pack(side="right", padx=(8, 0))
        ttk.Button(controls, text="Apply and export skirmish...",
                   command=lambda: self.accept(export=True)).pack(side="left")
        self.points.trace_add("write", self.refresh_difficulty)
        self.search.trace_add("write", self.refresh_roster)
        self.refresh_difficulty()
        self.refresh_roster()
        self.bind("<Escape>", lambda _: self.destroy())
        self.grab_set()

    def choose_difficulty(self, _=None):
        self.points.set(str(SKIRMISH_DIFFICULTIES[self.difficulty.get()]))

    def refresh_difficulty(self, *_):
        value = self.points.get()
        self.difficulty.set(skirmish_difficulty(int(value)) if value.isascii() and value.isdecimal()
                            and int(value) <= 32767 else "Invalid points")

    def remember_selection(self, _=None):
        selected = {self.visible[index] for index in self.roster.curselection()}
        self.enabled.difference_update(self.visible)
        self.enabled.update(selected)
        self.count.set(f"{len(self.enabled)} army unit types available. Automated/support availability is unchanged.")

    def refresh_roster(self, *_):
        self.remember_selection()
        query = self.search.get().strip().lower()
        kinds = skirmish_unit_types(self.editor.unit_types)
        labels = {kind: self.editor.unit_label(kind) for kind in kinds}
        self.visible = sorted((kind for kind in kinds if query in labels[kind].lower()), key=labels.get)
        self.roster.delete(0, "end")
        for index, kind in enumerate(self.visible):
            self.roster.insert("end", labels[kind])
            if kind in self.enabled:
                self.roster.selection_set(index)

    def settings(self) -> ScenarioSettings:
        self.remember_selection()
        if any(not value.isascii() or not value.isdecimal() for value in (self.turns.get(), self.points.get())):
            raise ValueError("Turn limit and points must be nonnegative integers.")
        text = self.briefing.get("1.0", "end-1c")
        briefing = self.original.briefing if text == self.initial_briefing else text.replace("\n", "##")
        kinds = {kind for kind, _ in self.original.available} | self.enabled
        return ScenarioSettings(self.name.get(), briefing, int(self.turns.get()), int(self.points.get()),
                                tuple((kind, kind in self.enabled) for kind in sorted(kinds)))

    def accept(self, *, export=False):
        try:
            self.editor.document.model.update_scenario(self.settings())
        except ERRORS as error:
            messagebox.showerror("Cannot apply scenario settings", str(error), parent=self)
            return
        self.editor.update_controls()
        if self.editor.script_dialog:
            self.editor.script_dialog.refresh_inspection()
        self.editor.status.set("Scenario settings applied. Save a copy or export the skirmish to keep them.")
        self.destroy()
        if export:
            self.editor.export_skirmish()


class ResizeDialog(tk.Toplevel):
    def __init__(self, editor):
        super().__init__(editor.root)
        self.editor = editor
        self.document = editor.document
        self.title("Resize map")
        self.transient(editor.root)
        self.size = tk.StringVar(value=str(self.document.model.size))
        self.terrain = tk.StringVar(value=editor.ground_texture.get())
        body = ttk.Frame(self, padding=12)
        body.pack(fill="both", expand=True)
        ttk.Label(body, text=f"Current map: {self.document.model.size} x {self.document.model.size}").pack(anchor="w")
        ttk.Label(body, text="New square side length (1..256):").pack(anchor="w", pady=(8, 0))
        ttk.Entry(body, textvariable=self.size).pack(fill="x")
        ttk.Label(body, text="Ground for added tiles:").pack(anchor="w", pady=(8, 0))
        ttk.Combobox(body, textvariable=self.terrain, state="readonly",
                     values=sorted(name for name, item in editor.terrains.items() if item.paintable)).pack(fill="x")
        ttk.Label(body, text="Top-left coordinates stay fixed. Tiles are added/removed along the right and bottom. "
                  "Shrinking is blocked if it would cut off objects, reinforcements, deployment or script targets. "
                  "Resize is undoable; installed files remain unchanged.",
                  wraplength=420).pack(anchor="w", pady=10)
        ttk.Button(body, text="Resize", command=self.apply).pack(side="left")
        ttk.Button(body, text="Cancel", command=self.destroy).pack(side="right")
        self.grab_set()

    def apply(self):
        try:
            if self.editor.busy or self.editor.document is not self.document:
                raise ValueError("Wait for the current operation and reopen Resize for the active map.")
            self.editor.paint_end()
            changed = self.document.model.resize(integer(self.size.get()), self.terrain.get())
            if changed:
                if self.editor.script_dialog:
                    self.editor.script_dialog.stop_pick()
                self.editor.render_map(reset_view=True)
                self.editor.refresh_inspector()
                self.editor.status.set(f"Resized map to {self.document.model.size}x{self.document.model.size}. "
                                       "Undo is available; Save Copy to persist.")
            self.destroy()
        except (ValueError, tk.TclError) as error:
            messagebox.showerror("Cannot resize map", str(error), parent=self)


class MapEditor:
    def __init__(self, root: tk.Tk, game_root: Path, initial: Path | None = None,
                 member: str | None = None):
        self.root = root
        self.game_root = game_root.resolve()
        registries.ensure(self.game_root)
        self.script = Path(__file__).with_name("map_tool.ps1")
        self.registry = Path(__file__).with_name("terrain-types.csv")
        self.terrains = load_terrains(self.registry)
        self.art = TerrainArt(self.game_root / "IMGS" / "TERRAINS.DBI")
        self.building_art = TerrainArt(self.game_root / "IMGS" / "UB.DBI")
        self.building_types = load_building_types(self.script.with_name("building-types.csv"))
        self.unit_types = load_unit_types(self.script.with_name("unit-types.csv"))
        self.unit_frames = {}
        for (identifier, tech), kind in self.unit_types.items():
            if tech:
                continue
            frames = [frame for frame in range(10)
                      if all(kind.sprite(x, y, facing, frame) in self.building_art.records
                             for facing in range(8 if kind.facings else 1)
                             for x in range(kind.width) for y in range(kind.height))]
            if not frames:
                raise ValueError(f"No complete unit artwork for {identifier}: {kind.name}")
            self.unit_frames[identifier] = frames[0]
        self.surface_types = load_surface_types(self.script.parent)
        self.surface_art = {layer: TerrainArt(self.game_root / "IMGS" / filename)
                            for layer, filename in (("roads", "ROADS.DBI"), ("walls", "WALLS.DBI"),
                                                    ("decors", "RUINS.DBI"))}
        self.surface_art["deployment"] = TerrainArt(self.game_root / "IMGS" / "SYMBOLS.DBI")
        self.surface_art["mines"] = self.building_art
        for player in range(1, 5):
            for shape in range(16):
                self.surface_art["deployment"].texture(f"DEPL{player}{shape:02d}")
        self.building_levels = {}
        for kind in self.building_types.values():
            levels = [level for level in range(5)
                      if all(kind.sprite(x, y, level=level) in self.building_art.records
                             for x in range(kind.width) for y in range(kind.height))]
            if not levels:
                raise ValueError(f"No complete building artwork for {kind.identifier}: {kind.name}")
            preferred = {131: 1, 173: 2, 174: 3, 175: 4}.get(kind.identifier, levels[0])
            if preferred not in levels:
                raise ValueError(f"Missing deployment artwork level {preferred}: {kind.identifier}")
            self.building_levels[kind.identifier] = preferred
        missing = sorted(set(self.terrains) - set(self.art.records))
        if missing:
            raise ValueError(f"The terrain registry has no matching artwork for: {', '.join(missing)}")
        self.document: Document | None = None
        self.script_dialog: ScriptDialog | None = None
        self.variant_documents: dict[int, Document] = {}
        self.variant_sources: dict[int, tuple[Path, str | None]] = {}
        self.executor = ThreadPoolExecutor(max_workers=1)
        self.busy = False
        self.images: dict[tuple[str, str], tk.PhotoImage] = {}
        self.items: dict[tuple[int, int], int] = {}
        self.palette_names: list[str] = []
        self.origin_x, self.origin_y = 0.0, 0.0
        self.scale = 1.0
        self.stroke_last: tuple[int, int] | None = None
        self.skipped: set[tuple[int, int]] = set()
        self.selected = tk.StringVar(value="BORE000")
        self.ground_texture = tk.StringVar(value="BORE000")
        self.tree_texture = tk.StringVar(value="NORD116")
        self.mode = tk.StringVar(value="Ground")
        self.erase_trees = tk.BooleanVar(value=False)
        self.scenery_texture = tk.StringVar(value=next(name for name, item in self.terrains.items() if item.scenery))
        self.surface_action = tk.StringVar(value="Paint")
        self.surface_kind = tk.StringVar(value="0")
        self.surface_brushes = {"roads": "0", "walls": "0", "decors": "BBRUI000", "mines": "0"}
        self.mine_flags = {name: tk.BooleanVar(value=name not in ("hidden_AI",))
                           for name in MINE_FLAGS}
        self.bridge_action = tk.StringVar(value="Paint banks")
        self.bridge_bank = tk.StringVar(value=next(name for name, item in self.terrains.items()
                                                   if item.kind == 2 and not item.overlapping))
        self.bridge_water = tk.StringVar(value="EAU000")
        self.bridge_counts = tk.StringVar()
        self.surface_owner = tk.StringVar(value="0")
        self.surface_armor = tk.StringVar(value="60")
        self.surface_x = tk.StringVar(value="0")
        self.surface_y = tk.StringVar(value="0")
        self.surface_broken = tk.BooleanVar(value=False)
        self.surface_inspected = tk.StringVar(value="No object selected")
        self.wall_preview = tk.StringVar()
        self.selected_surface: tuple[str, int] | None = None
        self.selected_unit: int | None = None
        self.unit_action = tk.StringVar(value="Select")
        self.unit_view = tk.StringVar(value="Start")
        self.unit_arrival = tk.StringVar(value="Start")
        self.unit_kind = tk.StringVar(value=self.unit_label(2))
        self.unit_owner = tk.StringVar(value="0")
        self.unit_tech = tk.StringVar(value="0")
        self.unit_facing = tk.StringVar(value="0")
        self.unit_armor = tk.StringVar(value="5")
        self.unit_ap = tk.StringVar(value="80")
        self.unit_x = tk.StringVar(value="0")
        self.unit_y = tk.StringVar(value="0")
        self.unit_order = tk.BooleanVar(value=False)
        self.unit_inspected = tk.StringVar(value="No unit selected")
        self.building_action = tk.StringVar(value="Select")
        self.selected_building: int | None = None
        self.building_kind = tk.StringVar(value=self.type_label(480))
        self.owner = tk.StringVar(value="0")
        self.armor = tk.StringVar(value="40")
        self.building_x = tk.StringVar(value="0")
        self.building_y = tk.StringVar(value="0")
        self.ai_target = tk.BooleanVar(value=False)
        self.inspected = tk.StringVar(value="No building selected")
        self.search = tk.StringVar()
        self.theme = tk.StringVar(value="All")
        self.zoom = tk.StringVar(value="100%")
        self.brush = tk.StringVar(value="1x1")
        self.grid = tk.BooleanVar(value=False)
        self.alternate_map = tk.BooleanVar(value=False)
        self.show_deployment = tk.BooleanVar(value=False)
        self.show_scripts = tk.BooleanVar(value=False)
        self.terrain_only = tk.BooleanVar(value=False)
        self.deployment_player = tk.StringVar(value="1")
        self.deployment_action = tk.StringVar(value="Paint")
        self.deployment_counts = tk.StringVar()
        self.status = tk.StringVar(value="Open a scenario MDB or ZIP archive to start.")
        self.heading = tk.StringVar(value="No scenario open")
        self.root.title("LIBDAY Map Editor")
        self.root.geometry("1800x1100")
        self.root.minsize(1100, 900)
        self.root.protocol("WM_DELETE_WINDOW", self.close)
        self._build_ui()
        self.refresh_palette()
        self.root.bind("<Control-o>", lambda _: self.open_dialog())
        self.root.bind("<Control-s>", lambda _: self.save_dialog())
        self.root.bind("<Control-z>", lambda _: self.undo())
        self.root.bind("<Control-y>", lambda _: self.redo())
        self.root.bind("<Escape>", lambda _: self.script_dialog.stop_pick() if self.script_dialog else None)
        self.canvas.bind("<Delete>", lambda _: self.delete_selected())
        if initial:
            self.root.after(150, lambda: self.open_source(initial, member))

    def _build_ui(self):
        toolbar = ttk.Frame(self.root, padding=(8, 8, 8, 0))
        toolbar.pack(fill="x")
        self.open_button = ttk.Button(toolbar, text="Open scenario...", command=self.open_dialog)
        self.open_button.pack(side="left", padx=4)
        self.save_button = ttk.Button(toolbar, text="Save copy...", command=self.save_dialog, state="disabled")
        self.save_button.pack(side="left", padx=4)
        self.undo_button = ttk.Button(toolbar, text="Undo", command=self.undo, state="disabled")
        self.undo_button.pack(side="left", padx=4)
        self.redo_button = ttk.Button(toolbar, text="Redo", command=self.redo, state="disabled")
        self.redo_button.pack(side="left", padx=4)
        self.scenario_button = ttk.Button(toolbar, text="Scenario...", command=self.edit_scenario, state="disabled")
        self.scenario_button.pack(side="left", padx=4)
        self.script_button = ttk.Button(toolbar, text="Scripts...", command=self.edit_scripts, state="disabled")
        self.script_button.pack(side="left", padx=4)
        self.alternate_control = ttk.Checkbutton(toolbar, text="Alternate map", variable=self.alternate_map,
                                                 command=self.toggle_alternate, state="disabled")
        self.alternate_control.pack(side="left", padx=4)
        self.help_button = ttk.Button(toolbar, text="Controls / limits", command=self.help)
        self.help_button.pack(side="right", padx=4)
        viewbar = ttk.Frame(self.root, padding=(12, 4, 12, 0))
        viewbar.pack(fill="x")
        ttk.Label(viewbar, text="Zoom").pack(side="left", padx=(0, 4))
        self.zoom_box = ttk.Combobox(viewbar, textvariable=self.zoom, values=list(ZOOMS),
                                     width=7, state="readonly")
        self.zoom_box.pack(side="left")
        self.zoom_box.bind("<<ComboboxSelected>>", lambda _: self.render_map())
        ttk.Label(viewbar, text="  Brush").pack(side="left", padx=4)
        ttk.Combobox(viewbar, textvariable=self.brush, values=("1x1", "3x3", "5x5"),
                     width=6, state="readonly").pack(side="left")
        ttk.Checkbutton(viewbar, text="Grid", variable=self.grid, command=self.render_map).pack(side="left", padx=12)
        self.deployment_toggle = ttk.Checkbutton(viewbar, text="Deployment zones", variable=self.show_deployment,
                                                 command=self.toggle_deployment)
        self.deployment_toggle.pack(side="left", padx=(0, 12))
        self.script_toggle = ttk.Checkbutton(viewbar, text="Script targets", variable=self.show_scripts,
                                             command=self.render_scripts)
        self.script_toggle.pack(side="left", padx=(0, 8))
        ttk.Label(viewbar, text="Units at").pack(side="left")
        self.unit_view_box = ttk.Combobox(viewbar, textvariable=self.unit_view, values=ARRIVAL_TURNS,
                                           width=9, state="readonly")
        self.unit_view_box.pack(side="left", padx=4)
        self.unit_view_box.bind("<<ComboboxSelected>>", self.change_unit_turn)
        ttk.Label(self.root, textvariable=self.heading, padding=(12, 4)).pack(fill="x")
        body = ttk.Frame(self.root)
        body.pack(fill="both", expand=True, padx=8)
        sidebar = ttk.Frame(body, width=370)
        sidebar.pack(side="left", fill="y", padx=(0, 8))
        sidebar.pack_propagate(False)
        self.mode_box = ttk.Combobox(sidebar, textvariable=self.mode,
                                     values=("Ground", "Trees", "Scenery", "Buildings", *SURFACE_MODES, "Bridges", "Units", "Deployment"),
                                     state="readonly")
        self.mode_box.pack(fill="x", pady=4)
        self.mode_box.bind("<<ComboboxSelected>>", self.change_mode)
        ttk.Label(sidebar, text="Search artwork ID / object name").pack(anchor="w")
        ttk.Entry(sidebar, textvariable=self.search).pack(fill="x", pady=(2, 8))
        themes = sorted({re_prefix(name) for name, terrain in self.terrains.items() if terrain.paintable})
        self.theme_box = ttk.Combobox(sidebar, textvariable=self.theme, values=["All", *themes], state="readonly")
        self.theme_box.pack(fill="x", pady=(0, 8))
        palette_frame = ttk.Frame(sidebar)
        self.palette_frame = palette_frame
        palette_frame.pack(fill="both", expand=True)
        self.palette = tk.Canvas(palette_frame, bg="#252932", highlightthickness=0, width=340, height=1)
        palette_scroll = ttk.Scrollbar(palette_frame, orient="vertical", command=self.palette.yview)
        self.palette.configure(yscrollcommand=palette_scroll.set)
        palette_scroll.pack(side="right", fill="y")
        self.palette.pack(fill="both", expand=True)
        self.palette.bind("<Button-1>", self.palette_click)
        self.palette.bind("<MouseWheel>", lambda event: self.palette.yview_scroll(-int(event.delta / 120), "units"))
        self.palette_count = ttk.Label(sidebar)
        self.palette_count.pack(anchor="w", pady=4)
        self.brush_label = ttk.Label(sidebar, text="Selected brush / type:")
        self.brush_label.pack(anchor="w", pady=(4, 0))
        self.brush_name_label = ttk.Label(sidebar, textvariable=self.selected, font=("Consolas", 12, "bold"))
        self.brush_name_label.pack(anchor="w", pady=(0, 6))
        self.ground_controls = ttk.Frame(sidebar)
        ttk.Checkbutton(self.ground_controls, text="Terrain mode (hide objects and vegetation)",
                        variable=self.terrain_only, command=self.change_terrain_view).pack(anchor="w")
        ttk.Label(self.ground_controls, text="View only: trees/scenery use plain-ground placeholders. "
                  "Their editing protections remain; use their erase tools to replace them.",
                  wraplength=350).pack(anchor="w", pady=4)
        self.resize_button = ttk.Button(self.ground_controls, text="Resize map...", command=self.resize_dialog)
        self.resize_button.pack(anchor="w", pady=4)
        self.ground_controls.pack(fill="x")
        self.tree_controls = ttk.Frame(sidebar)
        self.erase_label = tk.StringVar(value="Remove trees instead of planting")
        ttk.Checkbutton(self.tree_controls, textvariable=self.erase_label,
                        variable=self.erase_trees).pack(anchor="w")
        ttk.Label(self.tree_controls, text="Replacement ground (trees have no separate ground layer):",
                  wraplength=350).pack(anchor="w")
        ttk.Combobox(self.tree_controls, textvariable=self.ground_texture,
                     values=sorted(name for name, item in self.terrains.items() if item.paintable),
                     state="readonly").pack(fill="x", pady=4)
        self.building_controls = ttk.Frame(sidebar)
        actions = ttk.Frame(self.building_controls)
        actions.pack(fill="x")
        for action in ("Select", "Place", "Move"):
            ttk.Radiobutton(actions, text=action, variable=self.building_action, value=action).pack(side="left")
        ttk.Label(self.building_controls, textvariable=self.inspected, wraplength=350).pack(anchor="w", pady=4)
        self.type_box = ttk.Combobox(
            self.building_controls, textvariable=self.building_kind, state="readonly",
            values=[self.type_label(key) for key in sorted(self.building_types)],
        )
        self.type_box.pack(fill="x")
        self.type_box.bind("<<ComboboxSelected>>", self.type_changed)
        properties = ttk.Frame(self.building_controls)
        properties.pack(fill="x", pady=4)
        for label, variable, values in (
                ("Owner (0=AI, 1..4=players)", self.owner, tuple(map(str, range(5)))),
                ("Armor / health", self.armor, None), ("X", self.building_x, None), ("Y", self.building_y, None)):
            row = ttk.Frame(properties)
            row.pack(fill="x")
            ttk.Label(row, text=label, width=26).pack(side="left")
            widget = (ttk.Combobox(row, textvariable=variable, values=values, width=8, state="readonly")
                      if values else ttk.Entry(row, textvariable=variable, width=10))
            widget.pack(side="left")
        ttk.Checkbutton(self.building_controls, text="AI target", variable=self.ai_target).pack(anchor="w")
        buttons = ttk.Frame(self.building_controls)
        buttons.pack(fill="x", pady=4)
        self.apply_button = ttk.Button(buttons, text="Apply properties", command=self.apply_building, state="disabled")
        self.apply_button.pack(side="left")
        self.delete_button = ttk.Button(buttons, text="Delete building", command=self.delete_building, state="disabled")
        self.delete_button.pack(side="left", padx=4)
        ttk.Label(self.building_controls, text="Palette -> Place -> click map. Select a building to edit.\n"
                  "Move -> click a new footprint origin. Delete removes it.", wraplength=350).pack(anchor="w", pady=4)
        self.unit_controls = ttk.Frame(sidebar)
        actions = ttk.Frame(self.unit_controls)
        actions.pack(fill="x")
        for action in ("Select", "Place", "Move"):
            ttk.Radiobutton(actions, text=action, variable=self.unit_action, value=action).pack(side="left")
        ttk.Label(self.unit_controls, textvariable=self.unit_inspected, wraplength=350).pack(anchor="w", pady=4)
        self.unit_type_box = ttk.Combobox(self.unit_controls, textvariable=self.unit_kind, state="readonly",
                                         values=[self.unit_label(key) for key in sorted(self.unit_frames)])
        self.unit_type_box.pack(fill="x")
        self.unit_type_box.bind("<<ComboboxSelected>>", self.unit_type_changed)
        self.unit_property_widgets = {}
        properties = ttk.Frame(self.unit_controls)
        properties.pack(fill="x", pady=4)
        for index, (label, caption, variable, values, width) in enumerate((
                ("Arrival turn", "Arrival", self.unit_arrival, ARRIVAL_TURNS, 8),
                ("Owner (0=AI, 1..4=players)", "Owner", self.unit_owner, tuple(map(str, range(5))), 4),
                ("Technology level", "Tech", self.unit_tech, ("0",), 8),
                ("Facing (0..7)", "Facing", self.unit_facing, tuple(map(str, range(8))), 4),
                ("Armor / health", "Health", self.unit_armor, None, 8),
                ("Action points", "AP", self.unit_ap, None, 4),
                ("X", "X", self.unit_x, None, 8), ("Y", "Y", self.unit_y, None, 4))):
            row, column = index // 2, (index % 2) * 2
            ttk.Label(properties, text=caption).grid(row=row, column=column, sticky="w", padx=(0, 4), pady=1)
            widget = (ttk.Combobox(properties, textvariable=variable, values=values, width=width, state="readonly")
                      if values else ttk.Entry(properties, textvariable=variable, width=width + 2))
            widget.grid(row=row, column=column + 1, sticky="w", padx=(0, 6), pady=1)
            self.unit_property_widgets[label] = widget
        self.unit_property_widgets["Technology level"].bind("<<ComboboxSelected>>", self.unit_stats_changed)
        self.unit_order_control = ttk.Checkbutton(self.unit_controls, text="Use objective-derived AI order",
                                                  variable=self.unit_order)
        self.unit_order_control.pack(anchor="w", pady=4)
        buttons = ttk.Frame(self.unit_controls)
        buttons.pack(fill="x", pady=4)
        self.unit_apply_button = ttk.Button(buttons, text="Apply properties", command=self.apply_unit, state="disabled")
        self.unit_apply_button.pack(side="left")
        self.unit_delete_button = ttk.Button(buttons, text="Delete unit", command=self.delete_unit, state="disabled")
        self.unit_delete_button.pack(side="left", padx=4)
        self.unit_script_button = ttk.Button(self.unit_controls, text="Edit unit AI order...",
                                             command=self.edit_selected_order, state="disabled")
        self.unit_script_button.pack(fill="x", pady=2)
        ttk.Label(self.unit_controls, text="Owner: 0=AI, 1..4=players. Later arrivals: AI only, game-assigned stats.\n"
                  "Units at shows placements, not movement.", wraplength=350).pack(anchor="w", pady=4)
        self.unit_arrival.trace_add("write", lambda *_: self.sync_unit_controls())
        self.deployment_controls = ttk.Frame(sidebar)
        row = ttk.Frame(self.deployment_controls)
        row.pack(fill="x", pady=4)
        ttk.Label(row, text="Player slot", width=16).pack(side="left")
        self.deployment_player_box = ttk.Combobox(row, textvariable=self.deployment_player,
                                                 values=("1", "2", "3", "4"), state="readonly", width=8)
        self.deployment_player_box.pack(side="left")
        self.deployment_player_box.bind("<<ComboboxSelected>>", self.deployment_player_changed)
        row = ttk.Frame(self.deployment_controls)
        row.pack(fill="x")
        for action in ("Paint", "Erase"):
            ttk.Radiobutton(row, text=action, variable=self.deployment_action, value=action,
                            command=self.paint_end).pack(side="left", padx=(0, 12))
        ttk.Label(self.deployment_controls, textvariable=self.deployment_counts, wraplength=350).pack(
            anchor="w", pady=8)
        ttk.Label(self.deployment_controls,
                  text="Drag to paint a player's starting zone (Turn 0).\n"
                  "Painting transfers tiles from other player slots.\n"
                  "Erase removes only the selected player's cells.\n"
                  "Right-click a zone to pick its player.\n\n"
                  "Zones may cover any terrain or objects; actual unit placement is checked by the game. "
                  "AI/unknown-player/nonzero-turn records are preserved, not edited. "
                  "The game merges these slots for fewer players.",
                  wraplength=350).pack(anchor="w", pady=4)
        self.surface_controls = ttk.Frame(sidebar)
        actions = ttk.Frame(self.surface_controls)
        actions.pack(fill="x")
        for action in ("Paint", "Select", "Move", "Erase"):
            ttk.Radiobutton(actions, text=action, variable=self.surface_action, value=action).pack(side="left")
        ttk.Label(self.surface_controls, textvariable=self.surface_inspected, wraplength=350).pack(anchor="w", pady=4)
        self.surface_type_box = ttk.Combobox(self.surface_controls, textvariable=self.surface_kind, state="readonly")
        self.surface_type_box.pack(fill="x")
        self.surface_type_box.bind("<<ComboboxSelected>>", self.surface_type_changed)
        properties = ttk.Frame(self.surface_controls)
        properties.pack(fill="x", pady=4)
        self.surface_property_rows = {}
        for label, variable, values in (
                ("Owner (0=AI, 1..4=players)", self.surface_owner, tuple(map(str, range(5)))),
                ("Armor / health", self.surface_armor, None), ("X", self.surface_x, None), ("Y", self.surface_y, None)):
            row = ttk.Frame(properties)
            row.pack(fill="x")
            self.surface_property_rows[label] = row
            ttk.Label(row, text=label, width=26).pack(side="left")
            widget = (ttk.Combobox(row, textvariable=variable, values=values, width=8, state="readonly")
                      if values else ttk.Entry(row, textvariable=variable, width=10))
            widget.pack(side="left")
        self.surface_full_health_button = ttk.Button(
            self.surface_property_rows["Armor / health"], text="Full health", command=self.fill_wall_health)
        self.surface_full_health_button.pack(side="left", padx=4)
        self.broken_control = ttk.Checkbutton(self.surface_controls, text="Broken concrete section (isDamaged)",
                                               variable=self.surface_broken)
        self.wall_preview_label = ttk.Label(self.surface_controls, textvariable=self.wall_preview, wraplength=350)
        self.mine_controls = ttk.Frame(self.surface_controls)
        ttk.Checkbutton(self.mine_controls, text="Armed", variable=self.mine_flags["armed"]).pack(anchor="w")
        hidden = ttk.Frame(self.mine_controls)
        hidden.pack(fill="x")
        ttk.Label(hidden, text="Hidden from:").pack(side="left")
        for name, label in zip(MINE_FLAGS[1:], ("AI", "P1", "P2", "P3", "P4")):
            ttk.Checkbutton(hidden, text=label, variable=self.mine_flags[name]).pack(side="left")
        ttk.Label(self.mine_controls, text="All mines are visible in the editor, regardless of these flags.",
                  wraplength=350).pack(anchor="w")
        buttons = ttk.Frame(self.surface_controls)
        buttons.pack(fill="x", pady=4)
        self.surface_apply_button = ttk.Button(buttons, text="Apply properties", command=self.apply_surface,
                                                state="disabled")
        self.surface_apply_button.pack(side="left")
        self.surface_delete_button = ttk.Button(buttons, text="Delete object", command=self.delete_surface,
                                                 state="disabled")
        self.surface_delete_button.pack(side="left", padx=4)
        ttk.Label(self.surface_controls, text="Paint/Erase: drag a brush. Right-click to inspect.\n"
                  "Select -> edit -> Apply. Move -> click destination.\n"
                  "Low-HP damage previews are disabled; decors are debris/wreckage.",
                  wraplength=350).pack(anchor="w", pady=4)
        self.bridge_controls = ttk.Frame(sidebar)
        actions = ttk.Frame(self.bridge_controls)
        actions.pack(fill="x")
        for action in ("Paint banks", "Remove banks"):
            ttk.Radiobutton(actions, text=action, variable=self.bridge_action, value=action).pack(side="left")
        ttk.Label(self.bridge_controls, text="Removal replacement (deep water):").pack(anchor="w", pady=(4, 0))
        ttk.Combobox(self.bridge_controls, textvariable=self.bridge_water, state="readonly",
                     values=sorted(name for name, item in self.terrains.items()
                                   if item.kind == 1 and not item.overlapping)).pack(fill="x")
        ttk.Label(self.bridge_controls, textvariable=self.bridge_counts, wraplength=350).pack(anchor="w", pady=4)
        ttk.Label(self.bridge_controls,
                  text="Paint native riverbanks on deep-water/bank tiles; remove banks to deep water. "
                  "A crossing needs plain land behind both banks and deep water between them. "
                  "This changes terrain, not permission flags.\n\n"
                  "Markers show terrain-generated spans only in this mode. "
                  "The game still checks construction eligibility; existing bridges remain Roads.",
                  wraplength=350).pack(anchor="w")
        self.search.trace_add("write", lambda *_: self.refresh_palette())
        self.theme.trace_add("write", lambda *_: self.refresh_palette())
        self.surface_armor.trace_add("write", self.refresh_wall_preview)
        self.surface_kind.trace_add("write", self.refresh_wall_preview)
        map_frame = ttk.Frame(body)
        map_frame.pack(fill="both", expand=True)
        self.canvas = tk.Canvas(map_frame, bg="#1a1e26", highlightthickness=0, cursor="crosshair")
        horizontal = ttk.Scrollbar(map_frame, orient="horizontal", command=self.canvas.xview)
        vertical = ttk.Scrollbar(map_frame, orient="vertical", command=self.canvas.yview)
        self.canvas.configure(xscrollcommand=horizontal.set, yscrollcommand=vertical.set)
        self.canvas.grid(row=0, column=0, sticky="nsew")
        vertical.grid(row=0, column=1, sticky="ns")
        horizontal.grid(row=1, column=0, sticky="ew")
        map_frame.rowconfigure(0, weight=1)
        map_frame.columnconfigure(0, weight=1)
        self.canvas.bind("<ButtonPress-1>", self.paint_start)
        self.canvas.bind("<B1-Motion>", self.paint_drag)
        self.canvas.bind("<ButtonRelease-1>", self.paint_end)
        self.canvas.bind("<Motion>", self.hover)
        self.canvas.bind("<Leave>", lambda _: self.canvas.delete("hover"))
        self.canvas.bind("<Button-3>", self.eyedropper)
        self.canvas.bind("<MouseWheel>", self.wheel)
        self.canvas.bind("<ButtonPress-2>", self.pan_start)
        self.canvas.bind("<B2-Motion>", self.pan_drag)
        self.canvas.bind("<Shift-ButtonPress-1>", self.pan_start)
        self.canvas.bind("<Shift-B1-Motion>", self.pan_drag)
        ttk.Label(self.root, text="Edit terrain, objects, units, deployment zones and native scenario scripts. "
                  "Script forms must be applied before saving.", padding=(12, 6)).pack(fill="x")
        self.progress = ttk.Progressbar(self.root, mode="indeterminate")
        self.progress.pack(fill="x", padx=8)
        ttk.Label(self.root, textvariable=self.status, padding=(12, 8)).pack(fill="x")

    def image(self, name: str, zoom: str = "100%", building: bool = False,
              layer: str | None = None) -> tk.PhotoImage:
        key = (layer + ":" if layer else "building:" if building else "") + name, zoom
        if key not in self.images:
            art = self.surface_art[layer] if layer else self.building_art if building else self.art
            image = tk.PhotoImage(master=self.root, data=base64.b64encode(art.texture(name).png()))
            multiply, divide = ZOOMS[zoom]
            if multiply != 1:
                image = image.zoom(multiply)
            if divide != 1:
                image = image.subsample(divide)
            self.images[key] = image
        return self.images[key]

    def type_label(self, identifier: int) -> str:
        item = self.building_types[identifier]
        return f"{identifier} {item.name} ({item.width}x{item.height})"

    def current_type(self) -> int:
        return int(self.building_kind.get().split(" ", 1)[0])

    def current_layer(self) -> str | None:
        return SURFACE_MODES.get(self.mode.get())

    def surface_label(self, layer: str, kind: str) -> str:
        return f"{kind} {self.surface_types[layer][kind.upper()].name}"

    def current_surface_type(self) -> str:
        return self.surface_kind.get().split(" ", 1)[0]

    def surface_type_changed(self, _event=None):
        layer, kind = self.current_layer(), self.current_surface_type()
        definition = self.surface_types[layer][kind.upper()]
        self.surface_brushes[layer] = kind
        self.surface_armor.set(str(definition.armor))
        if kind != "0":
            self.surface_broken.set(False)
        self.selected.set(self.surface_label(layer, kind))
        self.refresh_palette()

    def refresh_wall_preview(self, *_):
        if self.current_layer() != "walls":
            self.wall_preview.set("")
            return
        definition = self.surface_types["walls"].get(self.current_surface_type())
        if definition is None:
            self.wall_preview.set("Select a wall type to inspect its stored health.")
            return
        try:
            armor = int(self.surface_armor.get())
        except ValueError:
            self.wall_preview.set(f"Health must be a whole number from 1 to {definition.armor}.")
            return
        if not 1 <= armor <= definition.armor:
            self.wall_preview.set(f"Health must be from 1 to {definition.armor}.")
            return
        self.wall_preview.set(
            f"After Apply: intact preview - health {armor}/{definition.armor}.\n"
            "Fresh battles reset HP; low-HP artwork is hidden.\n"
            "isDamaged controls broken concrete end caps.")

    def fill_wall_health(self):
        if self.busy or self.current_layer() != "walls":
            return
        definition = self.surface_types["walls"][self.current_surface_type()]
        self.surface_armor.set(str(definition.armor))
        self.status.set("Full health set in the inspector. Click Apply properties to update the selected wall."
                        if self.selected_surface else "Full health set for the wall painting brush.")

    def object_thumbnail(self, namespace: str, identifier: int, kind, sprite) -> tk.PhotoImage:
        key = f"thumbnail:{namespace}:{identifier}", "100%"
        if key not in self.images:
            width, height = (kind.width + kind.height) * 27, (kind.width + kind.height - 2) * 14 + 56
            rgba = bytearray(width * height * 4)
            for dx, dy in sorted(((x, y) for x in range(kind.width) for y in range(kind.height)),
                                 key=lambda point: (sum(point), point[0])):
                texture = self.building_art.texture(sprite(dx, dy))
                left, top = (dx - dy + kind.height - 1) * 27, (dx + dy) * 14
                for y in range(56):
                    for x in range(54):
                        source = (y * 54 + x) * 4
                        if texture.rgba[source + 3]:
                            target = ((top + y) * width + left + x) * 4
                            rgba[target:target + 4] = texture.rgba[source:source + 4]
            image = tk.PhotoImage(master=self.root, data=base64.b64encode(Texture(width, height, bytes(rgba)).png()))
            image = image.subsample(max(1, math.ceil(max(width / 100, height / 58))))
            self.images[key] = image
        return self.images[key]

    def building_thumbnail(self, identifier: int) -> tk.PhotoImage:
        kind = self.building_types[identifier]
        return self.object_thumbnail("building", identifier, kind,
                                     lambda x, y: kind.sprite(x, y, level=self.building_levels[identifier]))

    def unit_thumbnail(self, identifier: int) -> tk.PhotoImage:
        kind = self.unit_types[identifier, 0]
        return self.object_thumbnail("unit", identifier, kind,
                                     lambda x, y: kind.sprite(x, y, frame=self.unit_frames[identifier]))

    def unit_label(self, identifier: int) -> str:
        return f"{identifier} {self.unit_types[identifier, 0].name}"

    def current_unit_type(self) -> int:
        return int(self.unit_kind.get().split(" ", 1)[0])

    @staticmethod
    def arrival_value(value: str) -> int:
        if value not in ARRIVAL_TURNS:
            raise ValueError("Choose Start or a reinforcement turn from 1 to 100.")
        return ARRIVAL_TURNS.index(value)

    def unit_type_changed(self, _event=None):
        identifier = self.current_unit_type()
        levels = tuple(str(tech) for kind, tech in self.unit_types if kind == identifier)
        self.unit_property_widgets["Technology level"].configure(values=levels)
        self.unit_tech.set("0")
        self.unit_stats_changed()
        self.selected.set(self.unit_kind.get())
        self.refresh_palette()

    def unit_stats_changed(self, _event=None):
        definition = self.unit_types[self.current_unit_type(), int(self.unit_tech.get())]
        self.unit_armor.set(str(definition.armor))
        self.unit_ap.set(str(definition.ap))

    def unit_is_reinforcement(self, *, placing=False):
        if self.unit_arrival.get() != "Start":
            return True
        unit = self.document.model.units.get(self.selected_unit) if self.document else None
        return not placing and unit is not None and unit.layer == "reinforcements" and unit.arrival == 0

    def sync_unit_controls(self):
        reinforcement = self.unit_is_reinforcement(placing=self.unit_action.get() == "Place")
        for label, widget in self.unit_property_widgets.items():
            disabled = self.busy or (reinforcement and label in (
                "Owner (0=AI, 1..4=players)", "Technology level", "Facing (0..7)", "Armor / health", "Action points"))
            widget.configure(state="disabled" if disabled else "readonly" if isinstance(widget, ttk.Combobox) else "normal")
        self.unit_order_control.configure(state="normal" if reinforcement and not self.busy else "disabled")
        self.unit_type_box.configure(state="disabled" if self.busy else "readonly")

    def change_unit_turn(self, _event=None):
        if self.busy:
            return
        self.paint_end()
        self.selected_unit = None
        self.unit_arrival.set(self.unit_view.get())
        if self.unit_view.get() != "Start":
            self.unit_owner.set("0")
            self.unit_tech.set("0")
            self.unit_stats_changed()
        self.render_map()
        self.update_controls()
        self.status.set(f"Showing units scheduled at {self.unit_view.get()}; new placements use this arrival turn.")

    def change_mode(self, _event=None):
        self.paint_end()
        self.tree_controls.pack_forget()
        self.building_controls.pack_forget()
        self.surface_controls.pack_forget()
        self.unit_controls.pack_forget()
        self.deployment_controls.pack_forget()
        self.bridge_controls.pack_forget()
        self.ground_controls.pack_forget()
        mode = self.mode.get()
        layer = self.current_layer()
        if mode in ("Trees", "Scenery"):
            self.selected.set((self.tree_texture if mode == "Trees" else self.scenery_texture).get())
            self.erase_label.set("Remove trees instead of planting" if mode == "Trees" else "Erase scenery to ground")
            self.tree_controls.pack(fill="x")
        elif mode == "Buildings":
            self.selected.set(self.building_kind.get())
            self.building_controls.pack(fill="x")
        elif mode == "Units":
            self.selected.set(self.unit_kind.get())
            self.unit_controls.pack(fill="x")
            if self.selected_unit is None:
                self.unit_arrival.set(self.unit_view.get())
        elif mode == "Deployment":
            self.selected.set(f"Player {self.deployment_player.get()} zone")
            self.deployment_controls.pack(fill="x")
        elif mode == "Bridges":
            self.selected.set(self.bridge_bank.get())
            self.bridge_controls.pack(fill="x")
        elif layer:
            self.selected_surface = None
            self.surface_inspected.set("No object selected")
            self.surface_kind.set(self.surface_label(layer, self.surface_brushes[layer]))
            self.surface_action.set("Paint")
            self.surface_type_box.configure(values=[self.surface_label(layer, kind)
                                                     for kind in self.surface_types[layer]])
            for label in ("Owner (0=AI, 1..4=players)", "Armor / health"):
                self.surface_property_rows[label].pack_forget()
            if layer != "decors":
                self.surface_property_rows["Owner (0=AI, 1..4=players)"].pack(
                    fill="x", before=self.surface_property_rows["X"])
            if layer == "walls":
                self.surface_property_rows["Armor / health"].pack(fill="x", before=self.surface_property_rows["X"])
            self.broken_control.pack_forget()
            self.wall_preview_label.pack_forget()
            self.mine_controls.pack_forget()
            if layer == "walls":
                self.wall_preview_label.pack(fill="x", pady=4, before=self.surface_apply_button.master)
                self.broken_control.pack(anchor="w", before=self.surface_apply_button.master)
            elif layer == "mines":
                self.mine_controls.pack(fill="x", pady=4, before=self.surface_apply_button.master)
            self.surface_controls.pack(fill="x")
            self.surface_type_changed()
        else:
            self.selected.set(self.ground_texture.get())
            self.ground_controls.pack(fill="x")
        self.refresh_wall_preview()
        self.theme_box.configure(state="disabled" if mode in ("Buildings", "Units", "Deployment") or layer else "readonly")
        if mode in ("Units", "Deployment"):
            self.theme_box.pack_forget()
            self.brush_label.pack_forget()
            self.brush_name_label.pack_forget()
        elif not self.theme_box.winfo_manager():
            self.theme_box.pack(fill="x", pady=(0, 8), before=self.palette_frame)
            self.brush_label.pack(anchor="w", pady=(4, 0), after=self.palette_count)
            self.brush_name_label.pack(anchor="w", pady=(0, 6), after=self.brush_label)
        eligible = lambda item: item.kind == 2 and not item.overlapping if mode == "Bridges" else \
            item.tree if mode == "Trees" else item.scenery if mode == "Scenery" else item.paintable
        self.theme_box.configure(values=["All", *sorted({re_prefix(name) for name, item in self.terrains.items()
                                                         if eligible(item)})])
        self.theme.set("All")
        self.search.set("")
        self.refresh_palette()
        self.palette.yview_moveto(0)
        self.draw_selection()
        self.update_controls()
        self.show_deployment.set(mode == "Deployment")
        self.render_deployment()
        self.render_bridges()
        self.render_map()

    def render_bridges(self):
        self.canvas.delete("bridges")
        if not self.document:
            self.bridge_counts.set("No scenario open.")
            return
        if self.mode.get() != "Bridges":
            return
        spans = self.document.model.bridge_spans()
        self.bridge_counts.set(f"{len(spans)} terrain-generated spans; "
                               f"{sum(len(span.tiles) for span in spans)} marked tiles.")
        for span in spans:
            for key in span.tiles:
                cx, cy = self.center(*key)
                self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                                         image=self.image("BRIDG001", self.zoom.get(), layer="deployment"),
                                         tags="bridges")
        for tag in ("grid", "selection", "hover"):
            self.canvas.tag_raise(tag)

    def deployment_player_changed(self, _event=None):
        self.paint_end()
        self.selected.set(f"Player {self.deployment_player.get()} zone")
        self.refresh_palette()

    def toggle_deployment(self):
        self.paint_end()
        self.render_deployment()

    def render_deployment(self):
        self.canvas.delete("deployment")
        if not self.document:
            self.deployment_counts.set("No scenario open.")
            return
        model = self.document.model
        counts = [len({(item.x, item.y) for item in model.deployments.values()
                       if item.editable and item.player == player}) for player in range(1, 5)]
        preserved = sum(not item.editable for item in model.deployments.values())
        self.deployment_counts.set("Starting tiles: " + " / ".join(
            f"P{player}: {count}" for player, count in enumerate(counts, 1))
            + (f"\n{preserved} AI/unknown-player/nonzero-turn records preserved." if preserved else ""))
        if self.show_deployment.get() and not self.terrain_view:
            for key in sorted(model.deployment_positions.keys() & model.cells.keys()):
                item = model.deployment_at(key)
                if item:
                    cx, cy = self.center(*key)
                    self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                                             image=self.image(model.deployment_sprite(item), self.zoom.get(),
                                                              layer="deployment"), tags="deployment")
        for tag in ("grid", "selection", "hover"):
            self.canvas.tag_raise(tag)

    def type_changed(self, _event=None):
        kind = self.building_types[self.current_type()]
        self.armor.set(str(kind.armor))
        if self.selected_building is None:
            self.ai_target.set(kind.ai_target)
        self.selected.set(self.building_kind.get())
        self.refresh_palette()

    def refresh_palette(self):
        self.palette.delete("all")
        query, theme = self.search.get().strip().upper(), self.theme.get()
        buildings = self.mode.get() == "Buildings"
        units = self.mode.get() == "Units"
        deployment = self.mode.get() == "Deployment"
        layer = self.current_layer()
        if deployment:
            self.palette_names = [str(player) for player in range(1, 5) if query in f"PLAYER {player}"]
        elif buildings:
            self.palette_names = [str(key) for key, kind in sorted(self.building_types.items())
                                  if query in f"{key} {kind.name}".upper()]
        elif units:
            self.palette_names = [str(key) for key in sorted(self.unit_frames)
                                  if query in f"{key} {self.unit_types[key, 0].name}".upper()]
        elif layer:
            self.palette_names = [key for key, kind in self.surface_types[layer].items()
                                  if query in f"{key} {kind.name}".upper()]
        else:
            self.palette_names = sorted(name for name, terrain in self.terrains.items()
                                        if (terrain.kind == 2 and not terrain.overlapping
                                            if self.mode.get() == "Bridges" else
                                            terrain.tree if self.mode.get() == "Trees" else
                                            terrain.scenery if self.mode.get() == "Scenery" else terrain.paintable)
                                        and query in name.upper() and (theme == "All" or re_prefix(name) == theme))
        for index, name in enumerate(self.palette_names):
            x, y = (index % 3) * 112, (index // 3) * 96
            selected = (name == self.deployment_player.get() if deployment else
                        int(name) == self.current_type() if buildings else
                        int(name) == self.current_unit_type() if units else
                        name == self.current_surface_type() if layer else name == self.selected.get())
            self.palette.create_rectangle(x + 3, y + 3, x + 109, y + 93,
                                          fill="#334155" if selected else "#252932",
                                          outline="#ffd166" if selected else "#4b5563", width=2 if selected else 1)
            if deployment:
                image = self.image(f"DEPL{name}05", layer="deployment")
            elif layer:
                definition = self.surface_types[layer][name]
                sprite = definition.image + ("G006" if layer == "roads" else "006" if layer == "walls" else
                                              "000" if layer == "mines" else "")
                image = self.image(sprite, layer=layer)
            else:
                image = (self.building_thumbnail(int(name)) if buildings else
                         self.unit_thumbnail(int(name)) if units else self.image(name))
            self.palette.create_image(x + 56, y + 5, anchor="n", image=image)
            label = (f"Player {name}" if deployment else
                     f"{name} {self.building_types[int(name)].name}" if buildings else
                     self.unit_label(int(name)) if units else
                     self.surface_label(layer, name) if layer else name)
            self.palette.create_text(x + 56, y + 77, text=label, fill="#eeeeee",
                                     font=("Segoe UI", 8) if buildings or units or layer else ("Consolas", 9), width=105)
        self.palette.configure(scrollregion=(0, 0, 336, max(96, ((len(self.palette_names) + 2) // 3) * 96)))
        self.palette_count.configure(text=f"{len(self.palette_names)} {self.mode.get().lower()} choices")

    def palette_click(self, event):
        if self.busy:
            return
        x, y = self.palette.canvasx(event.x), self.palette.canvasy(event.y)
        column, row = int(x // 112), int(y // 96)
        index = row * 3 + column
        if 0 <= column < 3 and 0 <= index < len(self.palette_names):
            name = self.palette_names[index]
            if self.mode.get() == "Deployment":
                self.paint_end()
                self.deployment_player.set(name)
                self.deployment_player_changed()
                self.deployment_action.set("Paint")
            elif self.mode.get() == "Buildings":
                self.building_kind.set(self.type_label(int(name)))
                self.selected_building = None
                self.inspected.set("New building - click a valid footprint to place")
                self.building_action.set("Place")
                self.type_changed()
                self.update_controls()
            elif self.mode.get() == "Units":
                self.selected_unit = None
                self.unit_kind.set(self.unit_label(int(name)))
                self.unit_arrival.set(self.unit_view.get())
                self.unit_action.set("Place")
                self.unit_order.set(False)
                self.unit_type_changed()
                self.draw_selection()
                self.update_controls()
            elif self.current_layer():
                layer = self.current_layer()
                self.selected_surface = None
                self.surface_inspected.set("New brush - drag on clear terrain")
                self.surface_kind.set(self.surface_label(layer, name))
                self.surface_action.set("Paint")
                self.surface_broken.set(False)
                self.surface_type_changed()
                self.draw_selection()
                self.update_controls()
            else:
                self.selected.set(name)
                (self.bridge_bank if self.mode.get() == "Bridges" else
                 self.tree_texture if self.mode.get() == "Trees" else
                 self.scenery_texture if self.mode.get() == "Scenery" else self.ground_texture).set(name)
            self.refresh_palette()
            if self.terrain_view:
                self.render_map()
            self.status.set(f"Selected {self.selected.get()}. Left-drag to paint; right-click to pick "
                            + ("a player." if self.mode.get() == "Deployment" else "a texture."))

    def _job(self, description, action, success):
        self.busy = True
        self.progress.start(12)
        self.status.set(description)
        self.update_controls()
        future = self.executor.submit(action)

        def finish():
            if not future.done():
                self.root.after(60, finish)
                return
            self.busy = False
            self.progress.stop()
            try:
                result = future.result()
                success(result)
            except ERRORS as error:
                self.status.set(f"Operation failed: {error}")
                messagebox.showerror("Map editor error", str(error), parent=self.root)
            finally:
                self.update_controls()

        self.root.after(60, finish)

    def confirm_discard(self) -> bool:
        return not any(document.model.dirty for document in self.open_documents()) or messagebox.askyesno(
            "Unsaved map edits", "Discard the unsaved edits to the open map and its loaded variant?", parent=self.root)

    def open_documents(self) -> list[Document]:
        documents = list(self.variant_documents.values())
        if self.document is not None and all(document is not self.document for document in documents):
            documents.append(self.document)
        return documents

    def close_documents(self):
        for document in self.open_documents():
            document.close()
        self.document = None
        self.variant_documents.clear()
        self.variant_sources.clear()

    def load_document(self, source: Path, member: str | None) -> Document:
        document = Document.open(source, member, self.script, self.registry, self.terrains)
        complete = False
        try:
            for name in {cell.terrain for cell in document.model.cells.values()}:
                self.art.texture(name)
            for layer, items in document.model.surfaces.items():
                for item in items.values():
                    if (item.x, item.y) in document.model.cells:
                        self.surface_art[layer].texture(document.model.surface_sprite(layer, item))
            for unit in document.model.units.values():
                kind = self.unit_types[unit.kind, 0]
                for dx in range(kind.width):
                    for dy in range(kind.height):
                        self.building_art.texture(kind.sprite(dx, dy, unit.facing, self.unit_frames[unit.kind]))
            complete = True
            return document
        finally:
            if not complete:
                document.close()

    def activate_document(self, document: Document):
        if self.script_dialog:
            self.script_dialog.close()
        self.document = document
        self.stroke_last = None
        self.selected_building = None
        self.selected_surface = None
        self.selected_unit = None
        self.unit_view.set("Start")
        self.unit_arrival.set("Start")
        self.inspected.set("No building selected")
        self.show_deployment.set(self.mode.get() == "Deployment")
        self.show_scripts.set(False)
        self.render_map(reset_view=True)
        label = "alternate" if campaign_map_variant(document.source, document.member) == 2 else "normal"
        variant_note = (f" Editing the {label} variant; switching keeps each variant's unsaved edits."
                        if len(self.variant_sources) == 2 else "")
        self.status.set(f"Opened {self.display_source()} - {document.model.size}x{document.model.size}."
                        + variant_note + " Original files will not be overwritten.")

    def toggle_alternate(self):
        if self.busy or not self.document:
            self.update_controls()
            return
        self.paint_end()
        variant = 2 if self.alternate_map.get() else 1
        if variant not in self.variant_sources:
            self.update_controls()
            return
        if variant in self.variant_documents:
            self.activate_document(self.variant_documents[variant])
            return
        source, member = self.variant_sources[variant]

        def opened(document: Document):
            self.variant_documents[variant] = document
            self.activate_document(document)

        self._job("Opening alternate map..." if variant == 2 else "Opening normal map...",
                  lambda: self.load_document(source, member), opened)

    def open_dialog(self):
        if self.busy:
            return
        source = filedialog.askopenfilename(parent=self.root, title="Open scenario",
                                           initialdir=self.game_root / "SCENS",
                                           filetypes=[("Scenarios", "*.mdb *.zip"), ("Map database", "*.mdb"),
                                                      ("Scenario archive", "*.zip")])
        if source:
            self.open_source(Path(source))

    def open_source(self, source: Path, member: str | None = None):
        if self.busy or not self.confirm_discard():
            return
        try:
            if source.suffix.lower() == ".zip":
                members = archive_members(source)
                if member is None:
                    picker = ArchivePicker(self.root, source, members)
                    self.root.wait_window(picker)
                    member = picker.result
                    if member is None:
                        return
                elif member not in members:
                    raise ValueError(f"Scenario is not in this archive: {member}")
        except ERRORS as error:
            messagebox.showerror("Cannot open scenario", str(error), parent=self.root)
            return

        def load():
            document = self.load_document(source, member)
            other, warning = None, None
            try:
                other = alternate_map_source(document.source, document.member)
            except ERRORS as error:
                warning = str(error)
            return document, other, warning

        def opened(result):
            document, other, warning = result
            self.close_documents()
            variant = campaign_map_variant(document.source, document.member)
            if variant is not None:
                self.variant_documents[variant] = document
                self.variant_sources[variant] = document.source, document.member
                if other is not None:
                    self.variant_sources[3 - variant] = other
            self.activate_document(document)
            if warning:
                self.status.set(self.status.get() + f" Alternate map unavailable: {warning}")
                messagebox.showwarning("Alternate map unavailable", warning, parent=self.root)

        self._job("Opening scenario database...", load, opened)

    def display_source(self) -> str:
        if not self.document:
            return ""
        return self.document.source.name + (f" / {self.document.member}" if self.document.member else "")

    def render_map(self, reset_view: bool = False, finish_stroke: bool = True):
        if not self.document or self.busy:
            return
        model = self.document.model
        if self.mode.get() == "Units" and self.selected_unit in model.units:
            self.unit_view.set(ARRIVAL_TURNS[model.units[self.selected_unit].arrival])
        if finish_stroke:
            model.end_stroke()
        old_x, old_y = self.canvas.xview()[0], self.canvas.yview()[0]
        self.canvas.delete("all")
        self.items.clear()
        multiply, divide = ZOOMS[self.zoom.get()]
        self.scale = multiply / divide
        padding = 48
        self.origin_x = padding + model.size * 27 * self.scale
        self.origin_y = padding + 42 * self.scale
        self.canvas.configure(scrollregion=(0, 0, padding * 2 + model.size * 54 * self.scale,
                                           padding * 2 + (model.size * 28 + 56) * self.scale))
        parts = {}
        unit_parts = {}
        overlays = {}
        arrival = self.arrival_value(self.unit_view.get())
        for unit in model.units.values():
            if unit.arrival != arrival:
                continue
            kind = self.unit_types[unit.kind, 0]
            for dx in range(kind.width):
                for dy in range(kind.height):
                    unit_parts.setdefault((unit.x + dx, unit.y + dy), []).append(
                        kind.sprite(dx, dy, unit.facing, self.unit_frames[unit.kind]))
        for layer in ("roads", "decors", "walls", "mines"):
            for item in model.surfaces[layer].values():
                if (item.x, item.y) in model.cells:
                    overlays.setdefault((item.x, item.y), []).append((layer, model.surface_sprite(layer, item)))
        for building in model.buildings.values():
            kind = model.building_types[building.kind]
            for dx in range(kind.width):
                for dy in range(kind.height):
                    parts.setdefault((building.x + dx, building.y + dy), []).append(
                        kind.sprite(dx, dy, level=self.building_levels[building.kind]))
        if self.terrain_view:
            parts.clear()
            unit_parts.clear()
            overlays.clear()
        for key, cell in sorted(model.cells.items(), key=lambda item: (sum(item[0]), item[0][0])):
            cx, cy = self.center(*key)
            self.items[key] = self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale,
                                                       anchor="nw", image=self.image(self.display_terrain(key), self.zoom.get()))
            for layer, name in overlays.get(key, []):
                self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                                         image=self.image(name, self.zoom.get(), layer=layer),
                                         tags=("surface", layer))
            for name in parts.get(key, []):
                self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                                         image=self.image(name, self.zoom.get(), building=True), tags="building")
            for name in unit_parts.get(key, []):
                self.canvas.create_image(cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                                         image=self.image(name, self.zoom.get(), building=True), tags="unit")
        if self.grid.get():
            for key in model.cells:
                self.canvas.create_polygon(self.diamond(*key), fill="", outline="#607080",
                                           width=1, tags="grid")
        self.render_deployment()
        self.render_scripts()
        self.render_bridges()
        self.canvas.xview_moveto(0 if reset_view else old_x)
        self.canvas.yview_moveto(0 if reset_view else old_y)
        self.draw_selection()
        self.update_controls()

    def render_scripts(self):
        self.canvas.delete("scripting")
        if not self.document or self.busy or self.terrain_view or self.document.model.scripts is None:
            return
        dialog = self.script_dialog
        if not self.show_scripts.get() and not (dialog and dialog.picking):
            return
        model = self.document.model
        tab = dialog.active_tab if dialog else "Timeline / diagnostics"
        show_objective = tab in ("Timeline / diagnostics", "Objective")
        show_support = tab in ("Timeline / diagnostics", "Scheduled support")
        show_orders = tab in ("Timeline / diagnostics", "AI orders")

        def marker(key, color, label="", category="scripting"):
            if key not in model.cells:
                return
            tags = ("scripting", category)
            self.canvas.create_polygon(self.diamond(*key), fill="", outline=color, width=2, tags=tags)
            if label:
                cx, cy = self.center(*key)
                self.canvas.create_text(cx, cy, text=label, fill=color, tags=tags,
                                        font=("Consolas", 8))

        def landing(kind, x, y):
            support = model.support_types.get(kind)
            if support and support.category == 4:
                unit_type = model.unit_types[int(dict(support.properties)["unitIDDrop"]), 0]
                for dx, dy in support.offsets:
                    for px in range(unit_type.width):
                        for py in range(unit_type.height):
                            marker((x + dx + px, y + dy + py), "#b994ff", category="support_target")

        if self.show_scripts.get() and show_objective:
            for key in {(target.x, target.y) for target in model.scripts.objective_positions} & model.cells.keys():
                cx, cy = self.center(*key)
                self.canvas.create_image(
                    cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                    image=self.image("OBJT000", self.zoom.get(), layer="deployment"),
                    tags=("scripting", "objective_tile"))
            for target in {row.identifier for row in model.scripts.objective_ids}:
                unit = model.units.get(target)
                entity = unit or model.buildings.get(target)
                if entity is None or (unit and unit.arrival != self.arrival_value(self.unit_view.get())):
                    continue
                footprint = model.unit_footprint(unit) if unit else model.footprint(entity)
                if not footprint & model.cells.keys():
                    continue
                cx, cy = self.center(
                    (min(x for x, _ in footprint) + max(x for x, _ in footprint)) / 2,
                    (min(y for _, y in footprint) + max(y for _, y in footprint)) / 2)
                self.canvas.create_image(
                    cx - 27 * self.scale, cy - 42 * self.scale, anchor="nw",
                    image=self.image("OBJT001", self.zoom.get(), layer="deployment"),
                    tags=("scripting", "objective_id", f"objective_id_{target}"))
        for event in model.scripts.events if show_support else ():
            marker((event.x, event.y), "#76cfff", f"S{event.kind} T{event.turn}", "support_target")
            if event.kind in (0, 7) and (event.x, event.y) in model.cells and (
                    event.dest_x, event.dest_y) in model.cells:
                marker((event.dest_x, event.dest_y), "#76cfff", category="support_target")
                self.canvas.create_line(*self.center(event.x, event.y), *self.center(event.dest_x, event.dest_y),
                                        arrow=tk.LAST, fill="#76cfff", width=2,
                                        tags=("scripting", "support_target"))
            if dialog and dialog.selected_event == event.counter:
                landing(event.kind, event.x, event.y)
        identifier = dialog.selected_order if dialog else self.selected_unit
        if tab == "AI orders" and identifier in model.units:
            unit = model.units[identifier]
            if unit.arrival == self.arrival_value(self.unit_view.get()):
                for key in model.unit_footprint(unit):
                    marker(key, "#70df93", category="order_source")
        positions = [(row.x, row.y) for row in model.scripts.order_positions if row.unit == identifier]
        ids = [row.identifier for row in model.scripts.order_ids if row.unit == identifier]
        ordered = model.units.get(identifier)
        if ordered and ordered.layer == "reinforcements":
            derived = reinforcement_order(model.scripts, ordered.arrival_order)
            ids, positions = (derived[1], derived[2]) if derived else ((), ())
        if show_orders:
            for key in positions:
                marker(key, "#ffbf69", category="order_target")
        else:
            ids = ()
        for target in ids:
            entity = model.units.get(target) or model.buildings.get(target)
            if entity:
                marker((entity.x, entity.y), "#f2df7b", f"ID {target}", "order_target")
        if dialog and dialog.picking:
            try:
                for key in dialog.preview_tiles():
                    marker(key, "#ff7f96")
                if dialog.picking == "source":
                    landing(integer(dialog.event_kind.get().split(":")[0]),
                            integer(dialog.event_x.get()), integer(dialog.event_y.get()))
            except ValueError as error:
                dialog.info.set(f"Cannot preview draft targets: {error}")
        self.canvas.tag_raise("selection")

    def draw_selection(self):
        self.canvas.delete("selection")
        if not self.document or self.terrain_view:
            return
        if self.mode.get() == "Deployment":
            return
        if self.mode.get() == "Units":
            unit = self.document.model.units.get(self.selected_unit)
            if unit and unit.arrival == self.arrival_value(self.unit_view.get()):
                for key in self.document.model.unit_footprint(unit) & self.document.model.cells.keys():
                    self.canvas.create_polygon(self.diamond(*key), fill="", outline="#67e8f9", width=2, tags="selection")
                self.unit_inspected.set(f"ID {unit.identifier}, AI, arrival {ARRIVAL_TURNS[unit.arrival]}"
                                        if unit.layer == "reinforcements" else
                                        f"ID {unit.identifier}, owner {unit.owner}, AI-order turn {unit.order_turn}")
            else:
                self.selected_unit = None
                self.unit_inspected.set("No unit selected")
            return
        if self.current_layer():
            item = (self.document.model.surfaces[self.selected_surface[0]].get(self.selected_surface[1])
                    if self.selected_surface and self.selected_surface[0] == self.current_layer() else None)
            if item:
                if (item.x, item.y) in self.document.model.cells:
                    self.canvas.create_polygon(self.diamond(item.x, item.y), fill="", outline="#67e8f9",
                                               width=2, tags="selection")
                definition = self.surface_types[self.current_layer()][item.kind.upper()]
                detail = (" - movement allowed" if definition.passable else " - blocks movement") \
                    if self.current_layer() == "decors" else f" - owner {item.owner}"
                self.surface_inspected.set(self.surface_label(self.current_layer(), item.kind) + detail)
            else:
                self.selected_surface = None
                self.surface_inspected.set("No object selected")
            return
        building = self.document.model.buildings.get(self.selected_building)
        if building is None:
            self.selected_building = None
            self.inspected.set("No building selected")
            return
        for key in self.document.model.footprint(building) & self.document.model.cells.keys():
            self.canvas.create_polygon(self.diamond(*key), fill="", outline="#67e8f9", width=2, tags="selection")
        self.inspected.set(f"Building {building.identifier} - owner {building.owner}")

    def center(self, x: int, y: int) -> tuple[float, float]:
        return self.origin_x + (x - y) * 27 * self.scale, self.origin_y + (x + y) * 14 * self.scale

    def diamond(self, x: int, y: int) -> tuple[float, ...]:
        cx, cy = self.center(x, y)
        return (cx, cy - 14 * self.scale, cx + 27 * self.scale, cy,
                cx, cy + 14 * self.scale, cx - 27 * self.scale, cy)

    def event_cell(self, event) -> tuple[int, int] | None:
        if not self.document:
            return None
        return tile_at(self.canvas.canvasx(event.x), self.canvas.canvasy(event.y),
                       self.origin_x, self.origin_y, self.scale, self.document.model.size)

    def hover(self, event):
        if self.busy:
            return
        self.canvas.delete("hover")
        key = self.event_cell(event)
        if key is not None:
            cell = self.document.model.cells[key]
            protected = key in self.document.model.protected
            building = self.document.model.building_at(key)
            if self.mode.get() == "Deployment":
                item = self.document.model.deployment_at(key)
                self.canvas.create_polygon(self.diamond(*key), fill="", outline="#ffd166", width=2, tags="hover")
                self.status.set(f"Tile {key}: " + (f"player {item.player} starting zone" if item else "no starting zone")
                                + f". {self.deployment_action.get()} player {self.deployment_player.get()}.")
                return
            if self.mode.get() == "Units" and self.unit_action.get() in ("Place", "Move"):
                model = self.document.model
                moving = model.units.get(self.selected_unit) if self.unit_action.get() == "Move" else None
                try:
                    properties = self.unit_properties(placing=moving is None)
                    properties.update(x=key[0], y=key[1])
                    candidate = (replace(moving, x=key[0], y=key[1]) if moving else
                                 Unit(model.next_unit_id, layer="reinforcements" if properties["arrival"] else "units",
                                      **properties))
                    try:
                        model.validate_unit(candidate, moving)
                        color, reason = "#86efac", "valid placement"
                    except ValueError as error:
                        color, reason = "#f87171", str(error)
                    for tile in model.unit_footprint(candidate) & model.cells.keys():
                        self.canvas.create_polygon(self.diamond(*tile), fill="", outline=color, width=2, tags="hover")
                    self.status.set(f"{self.unit_action.get()} {self.unit_label(candidate.kind)} at {key}: {reason}")
                except (ValueError, KeyError) as error:
                    self.status.set(f"Unit placement blocked: {error}")
                return
            if self.mode.get() == "Buildings" and self.building_action.get() in ("Place", "Move"):
                moving = (self.document.model.buildings.get(self.selected_building)
                          if self.building_action.get() == "Move" else None)
                kind = moving.kind if moving else self.current_type()
                definition = self.building_types[kind]
                candidate = Building(moving.identifier if moving else self.document.model.next_building_id,
                                     kind, moving.owner if moving else int(self.owner.get()), definition.armor,
                                     *key, False)
                try:
                    self.document.model.validate_building(candidate, moving)
                    color, reason = "#86efac", "valid footprint"
                except ValueError as error:
                    color, reason = "#f87171", str(error)
                for tile in self.document.model.footprint(candidate) & self.document.model.cells.keys():
                    self.canvas.create_polygon(self.diamond(*tile), fill="", outline=color, width=2, tags="hover")
                self.status.set(f"{self.building_action.get()} {definition.name} at {key}: {reason}")
                return
            self.canvas.create_polygon(self.diamond(*key), fill="",
                                       outline="#f87171" if protected else "#ffd166",
                                       width=2, tags="hover")
            detail = f"building {building.identifier}" if building else self.document.model.blocked.get(
                key, "forest" if self.terrains[cell.terrain.upper()].tree else
                "protected obstacle" if protected else "clear ground")
            self.status.set(f"Tile ({cell.x}, {cell.y}) - {cell.terrain} - {detail}")

    def paint_start(self, event):
        if self.busy or not self.document:
            return
        self.canvas.focus_set()
        if self.script_dialog and self.script_dialog.picking:
            self.script_dialog.map_start(self.event_cell(event))
            return
        if self.mode.get() == "Units":
            self.paint_end()
            self.unit_click(event)
            return
        if self.mode.get() == "Buildings":
            self.building_click(event)
            return
        if self.current_layer() and self.surface_action.get() in ("Select", "Move"):
            self.paint_end()
            self.surface_click(event)
            return
        self.document.model.end_stroke()
        self.document.model.begin_stroke()
        self.skipped.clear()
        self.stroke_last = None
        self.paint_drag(event)

    def paint_drag(self, event):
        if self.script_dialog and self.script_dialog.picking:
            self.script_dialog.map_drag(self.event_cell(event))
            return
        if self.busy or not self.document or self.document.model.stroke is None:
            return
        key = self.event_cell(event)
        if key is None:
            self.stroke_last = None
            return
        points = line_cells(self.stroke_last, key, cardinal=self.current_layer() in ("roads", "walls")) \
            if self.stroke_last is not None else [key]
        self.stroke_last = key
        radius = int(self.brush.get()[0]) // 2
        model = self.document.model
        changed = set()
        for x, y in points:
            for dx in range(-radius, radius + 1):
                for dy in range(-radius, radius + 1):
                    target = x + dx, y + dy
                    if target not in model.cells:
                        continue
                    try:
                        layer = self.current_layer()
                        if self.mode.get() == "Deployment":
                            result = model.paint_deployment(*target, int(self.deployment_player.get()),
                                                            self.deployment_action.get() == "Erase")
                        elif layer:
                            result = model.paint_surface(
                                layer, *target, self.current_surface_type(), int(self.surface_owner.get()),
                                int(self.surface_armor.get()) if layer == "walls" else None,
                                self.surface_broken.get(), self.surface_action.get() == "Erase",
                                {name: flag.get() for name, flag in self.mine_flags.items()}
                                if layer == "mines" else None)
                        elif self.mode.get() == "Bridges":
                            erase = self.bridge_action.get() == "Remove banks"
                            result = model.paint(*target, self.bridge_water.get() if erase else self.bridge_bank.get(),
                                                 "erase_bridge" if erase else "bridge")
                        else:
                            mode = ("ground" if self.mode.get() == "Ground" else
                                    "erase_scenery" if self.mode.get() == "Scenery" and self.erase_trees.get() else
                                    "scenery" if self.mode.get() == "Scenery" else
                                    "erase" if self.erase_trees.get() else "trees")
                            texture = self.ground_texture.get() if mode.startswith("erase") else self.selected.get()
                            result = model.paint(*target, texture, mode)
                        if result:
                            changed.add(target)
                    except ValueError as error:
                        self.skipped.add(target)
                        self.skip_reason = str(error)
        self.refresh_cells(changed)
        self.canvas.delete("hover")
        self.canvas.create_polygon(self.diamond(*key), fill="", outline="#ffd166", width=2, tags="hover")
        self.status.set(f"Painting at {key}. {len(self.skipped)} blocked tiles skipped."
                        + (f" {self.skip_reason}" if self.skipped else ""))
        self.update_title()

    def paint_end(self, _event=None):
        if self.script_dialog:
            self.script_dialog.pick_stroke = False
            self.script_dialog.pick_last = None
        if not self.document or self.document.model.stroke is None:
            return
        changed = (len(self.document.model.stroke) + len(self.document.model.surface_stroke)
                   + len(self.document.model.deployment_stroke))
        self.document.model.end_stroke()
        self.stroke_last = None
        self.status.set(f"Paint stroke: {changed} changed tiles, {len(self.skipped)} blocked tiles skipped."
                        + (f" {self.skip_reason}" if self.skipped else ""))
        self.update_controls()

    def refresh_cells(self, keys):
        if keys and self.mode.get() == "Deployment":
            self.render_deployment()
            return
        if keys and self.current_layer():
            self.render_map(finish_stroke=False)
            return
        for key in keys:
            self.canvas.itemconfigure(self.items[key],
                                      image=self.image(self.display_terrain(key), self.zoom.get()))
        self.canvas.tag_raise("grid")
        self.canvas.tag_raise("selection")
        self.canvas.tag_raise("hover")
        if keys and self.mode.get() == "Bridges":
            self.render_bridges()

    def eyedropper(self, event):
        if self.busy or not self.document:
            return
        key = self.event_cell(event)
        if key is None:
            return
        if self.mode.get() == "Deployment":
            self.paint_end()
            item = self.document.model.deployment_at(key)
            if item:
                self.deployment_player.set(str(item.player))
                self.deployment_player_changed()
                self.status.set(f"Picked player {item.player}'s starting zone.")
            else:
                self.status.set(f"No editable starting zone at {key}.")
            return
        if self.mode.get() == "Units":
            self.paint_end()
            self.unit_action.set("Select")
            self.select_unit_at(key)
            return
        if self.mode.get() == "Buildings":
            self.select_building(self.document.model.building_at(key))
            return
        if self.current_layer():
            self.paint_end()
            self.select_surface(self.document.model.surface_at(self.current_layer(), key))
            self.surface_action.set("Select")
            return
        name = self.document.model.cells[key].terrain
        terrain = self.terrains[name.upper()]
        if self.mode.get() == "Bridges":
            if terrain.kind == 2:
                self.bridge_bank.set(name.upper())
                self.selected.set(name.upper())
                self.refresh_palette()
            elif terrain.kind == 1:
                self.bridge_water.set(name.upper())
            else:
                self.status.set("Pick a riverbank texture or deep-water replacement in Bridges mode.")
            return
        if not (terrain.paintable or terrain.tree or terrain.scenery):
            self.status.set(f"{name} is protected water/coast terrain.")
            return
        self.mode.set("Trees" if terrain.tree else "Scenery" if terrain.scenery else "Ground")
        (self.tree_texture if terrain.tree else self.scenery_texture if terrain.scenery else
         self.ground_texture).set(name.upper())
        self.change_mode()
        self.selected.set(name.upper())
        self.refresh_palette()
        self.status.set(f"Picked {name}.")

    def select_building(self, building: Building | None):
        self.selected_building = building.identifier if building else None
        if building:
            self.building_kind.set(self.type_label(building.kind))
            self.owner.set(str(building.owner))
            self.armor.set(str(building.armor))
            self.building_x.set(str(building.x))
            self.building_y.set(str(building.y))
            self.ai_target.set(building.ai_target)
            self.selected.set(self.building_kind.get())
        self.draw_selection()
        self.refresh_palette()
        self.update_controls()
        self.status.set(f"Selected building {building.identifier}. Edit properties or choose Move."
                        if building else "Click a building footprint to select it.")

    def building_properties(self) -> dict:
        fields = {"kind": self.current_type(), "owner": int(self.owner.get()), "armor": int(self.armor.get()),
                  "x": int(self.building_x.get()), "y": int(self.building_y.get()),
                  "ai_target": self.ai_target.get()}
        if fields["x"] < 0 or fields["y"] < 0:
            raise ValueError("Building coordinates cannot be negative.")
        return fields

    def object_operation(self, action):
        try:
            building = action()
            self.render_map()
            self.select_building(building)
            self.status.set("Building edit applied." if building else "Building deleted.")
        except (ValueError, KeyError) as error:
            self.status.set(f"Building edit blocked: {error}")
            messagebox.showerror("Building edit blocked", str(error), parent=self.root)

    def building_click(self, event):
        key = self.event_cell(event)
        if key is None:
            return
        model, action = self.document.model, self.building_action.get()
        if action == "Select":
            self.select_building(model.building_at(key))
        elif action == "Place":
            self.building_x.set(str(key[0]))
            self.building_y.set(str(key[1]))
            self.object_operation(lambda: model.add_building(**self.building_properties()))
        elif self.selected_building is not None:
            self.object_operation(lambda: model.update_building(self.selected_building, x=key[0], y=key[1]))
        else:
            self.status.set("Select a building before using Move.")

    def apply_building(self):
        if not self.busy and self.document and self.selected_building is not None:
            self.object_operation(lambda: self.document.model.update_building(
                self.selected_building, **self.building_properties()))

    def delete_building(self):
        if self.busy or not self.document or self.selected_building is None:
            return
        if messagebox.askyesno("Delete building", f"Delete building {self.selected_building}?",
                               parent=self.root):
            self.object_operation(lambda: self.document.model.delete_building(self.selected_building))

    def select_unit_at(self, key):
        candidates = self.document.model.units_at(key, self.arrival_value(self.unit_view.get()))
        identifiers = [unit.identifier for unit in candidates]
        index = (identifiers.index(self.selected_unit) + 1) % len(candidates) if self.selected_unit in identifiers else 0
        self.select_unit(candidates[index] if candidates else None)

    def select_unit(self, unit: Unit | None):
        self.selected_unit = unit.identifier if unit else None
        if unit:
            self.unit_kind.set(self.unit_label(unit.kind))
            self.unit_arrival.set(ARRIVAL_TURNS[unit.arrival])
            self.unit_owner.set(str(unit.owner))
            self.unit_tech.set(str(unit.tech))
            self.unit_facing.set(str(unit.facing))
            self.unit_armor.set(str(unit.armor))
            self.unit_ap.set(str(unit.ap))
            self.unit_x.set(str(unit.x))
            self.unit_y.set(str(unit.y))
            self.unit_order.set(unit.arrival_order)
            self.unit_property_widgets["Technology level"].configure(
                values=tuple(str(tech) for kind, tech in self.unit_types if kind == unit.kind))
            self.selected.set(self.unit_kind.get())
        self.draw_selection()
        self.refresh_palette()
        self.update_controls()
        self.render_scripts()

    def unit_properties(self, *, placing=False) -> dict:
        fields = {"kind": self.current_unit_type(), "x": int(self.unit_x.get()), "y": int(self.unit_y.get()),
                  "arrival": self.arrival_value(self.unit_arrival.get()), "arrival_order": self.unit_order.get()}
        if not self.unit_is_reinforcement(placing=placing):
            fields.update(owner=int(self.unit_owner.get()), tech=int(self.unit_tech.get()),
                          facing=int(self.unit_facing.get()), armor=int(self.unit_armor.get()), ap=int(self.unit_ap.get()))
        return fields

    def unit_operation(self, action):
        try:
            unit = action()
            if unit:
                self.unit_view.set(ARRIVAL_TURNS[unit.arrival])
            self.render_map()
            self.select_unit(unit)
            if self.script_dialog:
                self.script_dialog.refresh()
            self.status.set("Unit edit applied." if unit else "Unit deleted.")
        except (ValueError, KeyError) as error:
            self.status.set(f"Unit edit blocked: {error}")
            messagebox.showerror("Unit edit blocked", str(error), parent=self.root)

    def unit_click(self, event):
        key = self.event_cell(event)
        if key is None:
            return
        model, action = self.document.model, self.unit_action.get()
        if action == "Select":
            self.select_unit_at(key)
        elif action == "Place":
            self.unit_x.set(str(key[0]))
            self.unit_y.set(str(key[1]))
            self.unit_operation(lambda: model.add_unit(**self.unit_properties(placing=True)))
        elif self.selected_unit is not None:
            self.unit_operation(lambda: model.update_unit(self.selected_unit, x=key[0], y=key[1]))
        else:
            self.status.set("Select a unit before using Move.")

    def apply_unit(self):
        if not self.busy and self.document and self.selected_unit is not None:
            self.unit_operation(lambda: self.document.model.update_unit(self.selected_unit, **self.unit_properties()))

    def delete_unit(self):
        if self.busy or not self.document or self.selected_unit is None:
            return
        if messagebox.askyesno("Delete unit", f"Delete unit {self.selected_unit}?", parent=self.root):
            self.unit_operation(lambda: self.document.model.delete_unit(self.selected_unit))

    def select_surface(self, item):
        layer = self.current_layer()
        self.selected_surface = (layer, item.counter) if item else None
        if item:
            self.surface_kind.set(self.surface_label(layer, item.kind))
            self.surface_brushes[layer] = item.kind
            self.surface_owner.set(str(item.owner))
            self.surface_armor.set(str(item.armor))
            self.surface_x.set(str(item.x))
            self.surface_y.set(str(item.y))
            self.surface_broken.set(item.broken)
            if layer == "mines":
                for name in MINE_FLAGS:
                    self.mine_flags[name].set(getattr(item, name))
            self.selected.set(self.surface_label(layer, item.kind))
        self.draw_selection()
        self.refresh_palette()
        self.update_controls()

    def surface_operation(self, action):
        try:
            item = action()
            self.render_map()
            self.select_surface(item)
            self.status.set("Object edit applied." if item else "Object deleted.")
        except (ValueError, KeyError) as error:
            self.status.set(f"Object edit blocked: {error}")
            messagebox.showerror("Object edit blocked", str(error), parent=self.root)

    def surface_click(self, event):
        key = self.event_cell(event)
        if key is None:
            return
        layer = self.current_layer()
        if self.surface_action.get() == "Select":
            self.select_surface(self.document.model.surface_at(layer, key))
        elif self.selected_surface and self.selected_surface[0] == layer:
            self.surface_operation(lambda: self.document.model.update_surface(
                layer, self.selected_surface[1], x=key[0], y=key[1]))
        else:
            self.status.set("Select an object before using Move.")

    def apply_surface(self):
        if self.busy or not self.document or not self.selected_surface:
            return
        def apply():
            layer, counter = self.selected_surface
            properties = {"kind": self.current_surface_type(), "x": int(self.surface_x.get()),
                          "y": int(self.surface_y.get())}
            if layer != "decors":
                properties["owner"] = int(self.surface_owner.get())
            if layer == "walls":
                properties.update(armor=int(self.surface_armor.get()), broken=self.surface_broken.get())
            elif layer == "mines":
                properties.update({name: flag.get() for name, flag in self.mine_flags.items()})
            return self.document.model.update_surface(layer, counter, **properties)
        self.surface_operation(apply)

    def delete_surface(self):
        if self.busy or not self.document or not self.selected_surface:
            return
        layer, counter = self.selected_surface
        if messagebox.askyesno("Delete object", f"Delete the selected {layer} object?", parent=self.root):
            self.surface_operation(lambda: self.document.model.delete_surface(layer, counter))

    def delete_selected(self):
        if self.mode.get() == "Units":
            self.delete_unit()
        elif self.current_layer():
            self.delete_surface()
        elif self.mode.get() == "Buildings":
            self.delete_building()

    def undo(self):
        if self.busy or not self.document:
            return
        scenario_edit = bool(self.document.model.undo_stack and self.document.model.undo_stack[-1].scenario)
        changed = self.document.model.undo()
        self.render_map()
        self.refresh_inspector()
        self.status.set("Undo: scenario settings restored." if scenario_edit else
                        f"Undo: {len(changed)} tiles/footprint cells restored.")
        self.update_controls()

    def redo(self):
        if self.busy or not self.document:
            return
        scenario_edit = bool(self.document.model.redo_stack and self.document.model.redo_stack[-1].scenario)
        changed = self.document.model.redo()
        self.render_map()
        self.refresh_inspector()
        self.status.set("Redo: scenario settings reapplied." if scenario_edit else
                        f"Redo: {len(changed)} tiles/footprint cells reapplied.")
        self.update_controls()

    def refresh_inspector(self):
        if self.script_dialog:
            self.script_dialog.refresh()
        if self.mode.get() == "Units":
            self.select_unit(self.document.model.units.get(self.selected_unit))
        elif self.current_layer():
            item = (self.document.model.surfaces[self.selected_surface[0]].get(self.selected_surface[1])
                    if self.selected_surface else None)
            self.select_surface(item)
        elif self.mode.get() == "Buildings":
            self.select_building(self.document.model.buildings.get(self.selected_building))

    def pan_start(self, event):
        if not self.busy:
            self.paint_end()
            self.canvas.scan_mark(event.x, event.y)

    def pan_drag(self, event):
        if not self.busy:
            self.canvas.scan_dragto(event.x, event.y, gain=1)

    def wheel(self, event):
        if self.busy:
            return
        direction = -int(event.delta / 120)
        if event.state & 4:
            choices = list(ZOOMS)
            index = choices.index(self.zoom.get())
            self.zoom.set(choices[max(0, min(len(choices) - 1, index - direction))])
            self.render_map()
        elif event.state & 1:
            self.canvas.xview_scroll(direction * 3, "units")
        else:
            self.canvas.yview_scroll(direction * 3, "units")

    def save_dialog(self):
        if self.busy or not self.document:
            return
        self.paint_end()
        document = self.document
        filetypes = [("Map database copy", "*.mdb")]
        if document.member:
            filetypes.append(("Scenario archive copy", "*.zip"))
        default_name = Path(document.member or document.source.name).stem + "-edited.mdb"
        output = filedialog.asksaveasfilename(parent=self.root, title="Save edited scenario copy",
                                            initialdir=document.source.parent, initialfile=default_name,
                                            defaultextension=".mdb", filetypes=filetypes)
        if not output:
            return
        path = Path(output)
        rows = document.model.rows()
        buildings, next_id = document.model.building_rows(), document.model.next_building_id
        surfaces = document.model.surface_rows()
        units, next_unit_id = document.model.unit_rows(), document.model.next_unit_id
        scenario = document.model.scenario
        deployments = document.model.deployment_rows()
        scripts = document.model.scripts

        def saved(_):
            document.model.mark_saved()
            self.status.set(f"Saved {path}. Original files and unrelated mission data were preserved. "
                            "Stored overview artwork was not regenerated.")

        self._job("Saving edited scenario copy...",
                  lambda: document.save_copy(path, rows, buildings, next_id, surfaces, units, next_unit_id,
                                             scenario, deployments, scripts), saved)

    def edit_scenario(self):
        if not self.busy and self.document and self.document.model.scenario:
            self.paint_end()
            ScenarioDialog(self)

    @property
    def terrain_view(self):
        return self.mode.get() == "Ground" and self.terrain_only.get()

    def change_terrain_view(self):
        self.paint_end()
        if self.terrain_view and self.script_dialog:
            self.script_dialog.stop_pick()
        self.render_map()

    def display_terrain(self, key):
        name = self.document.model.cells[key].terrain
        terrain = self.terrains[name.upper()]
        if self.terrain_view and (terrain.tree or terrain.scenery):
            base = re_prefix(name.upper()) + "000"
            return base if base in self.terrains and self.terrains[base].paintable else self.ground_texture.get()
        return name

    def resize_dialog(self):
        if self.document and not self.busy:
            self.paint_end()
            ResizeDialog(self)

    def edit_scripts(self):
        if self.busy or not self.document or self.document.model.scripts is None:
            return
        self.paint_end()
        if self.script_dialog:
            self.script_dialog.lift()
            return
        self.script_dialog = ScriptDialog(self)

    def edit_selected_order(self):
        self.edit_scripts()
        if self.script_dialog:
            self.script_dialog.selected_order = self.selected_unit
            self.script_dialog.fill_order()
            self.script_dialog.notebook.select(self.script_dialog.pages["AI orders"])

    def export_skirmish(self):
        if self.busy or not self.document:
            return
        self.paint_end()
        chosen = filedialog.asksaveasfilename(parent=self.root, title="Name a NEW skirmish export folder",
                                             initialdir=self.document.source.parent,
                                             initialfile="my-skirmish", defaultextension="")
        if not chosen:
            return
        path = Path(chosen)
        document, model = self.document, self.document.model
        snapshot = dict(rows=model.rows(), buildings=model.building_rows(), next_id=model.next_building_id,
                       surfaces=model.surface_rows(), units=model.unit_rows(), next_unit_id=model.next_unit_id,
                       scenario=model.scenario, deployments=model.deployment_rows(), scripts=model.scripts)

        def exported(_):
            model.mark_saved()
            self.status.set(f"Exported {path}\\SCENS: a registered skirmish and matching catalog copy. "
                           "Installed game files were not changed.")

        self._job("Exporting a skirmish and catalog copy...",
                  lambda: document.export_skirmish(path, self.game_root / "SCENS" / "SKIRMISH.MDB",
                                                 self.game_root / "SCENS" / "SCEN1.ZIP", **snapshot), exported)

    def update_title(self):
        suffix = " *" if any(document.model.dirty for document in self.open_documents()) else ""
        title = f"LIBDAY Map Editor - {self.display_source()}{suffix}" if self.document else "LIBDAY Map Editor"
        self.root.title(title)
        if self.document:
            size = self.document.model.size
            protected = len(self.document.model.protected)
            name = self.document.model.scenario.title if self.document.model.scenario else self.display_source()
            self.heading.set(f"{name}{suffix}  |  {size}x{size} tiles  |  "
                             f"{protected} forest/obstacle tiles  |  {len(self.document.model.buildings)} buildings  |  "
                             f"{len(self.document.model.surfaces['roads'])} roads / "
                             f"{len(self.document.model.surfaces['walls'])} walls / "
                             f"{len(self.document.model.surfaces['decors'])} decors  |  "
                             f"Zoom {self.zoom.get()}"
                             + ("  |  Other variant has unsaved edits"
                                if any(document is not self.document and document.model.dirty
                                       for document in self.open_documents()) else ""))

    def update_controls(self):
        self.open_button.configure(state="disabled" if self.busy else "normal")
        self.save_button.configure(state="normal" if self.document and not self.busy else "disabled")
        self.scenario_button.configure(state="normal" if self.document and self.document.model.scenario
                                       and not self.busy else "disabled")
        self.script_button.configure(state="normal" if self.document and self.document.model.scripts is not None
                                     and not self.busy else "disabled")
        self.undo_button.configure(state="normal" if self.document and self.document.model.undo_stack
                                   and not self.busy else "disabled")
        self.redo_button.configure(state="normal" if self.document and self.document.model.redo_stack
                                   and not self.busy else "disabled")
        self.zoom_box.configure(state="disabled" if self.busy else "readonly")
        self.mode_box.configure(state="disabled" if self.busy else "readonly")
        variant = campaign_map_variant(self.document.source, self.document.member) if self.document else None
        self.alternate_map.set(variant == 2)
        self.alternate_control.configure(state="normal" if len(self.variant_sources) == 2 and not self.busy
                                         else "disabled")
        self.unit_view_box.configure(state="disabled" if self.busy else "readonly")
        self.deployment_toggle.configure(state="disabled" if self.busy else "normal")
        self.script_toggle.configure(state="disabled" if self.busy else "normal")
        self.resize_button.configure(state="normal" if self.document and not self.busy else "disabled")
        self.deployment_player_box.configure(state="disabled" if self.busy else "readonly")
        self.sync_unit_controls()
        for button in (self.apply_button, self.delete_button):
            button.configure(state="normal" if self.document and self.selected_building is not None
                             and not self.busy else "disabled")
        for button in (self.surface_apply_button, self.surface_delete_button):
            button.configure(state="normal" if self.document and self.selected_surface is not None
                             and not self.busy else "disabled")
        self.surface_full_health_button.configure(state="disabled" if self.busy else "normal")
        for button in (self.unit_apply_button, self.unit_delete_button):
            button.configure(state="normal" if self.document and self.selected_unit is not None
                             and not self.busy else "disabled")
        unit = self.document.model.units.get(self.selected_unit) if self.document else None
        self.unit_script_button.configure(state="normal" if unit
                                          and self.document.model.scripts is not None and not self.busy else "disabled")
        self.update_title()

    def help(self):
        messagebox.showinfo(
            "Controls and current limits",
            "Open: scenario MDB, or select an MDB inside a ZIP archive.\n"
            "Alternate map: unchecked = variant 1; checked = variant 2 of the same campaign mission.\n"
            "The checkbox is disabled without a matching map. Each variant keeps its own edits and undo history.\n"
            "Save copy / skirmish export saves only the currently displayed variant.\n"
            "Ground / Trees / Scenery: left-drag paints 1x1, 3x3 or 5x5 tiles.\n"
            "Ground: Terrain mode hides objects and vegetation without changing saved data or painting protections.\n"
            "Resize map: square, top-left anchored, with chosen clear-ground fill; undo/redo supported.\n"
            "Shrinking is blocked until cropped objects and script targets are moved or removed.\n"
            "Trees / Scenery: enable erasing and choose replacement ground.\n"
            "Roads / Walls / Decors: Paint or Erase with a brush.\n"
            "Deployment: choose player 1..4; Paint transfers starting tiles, Erase removes that player's tiles.\n"
            "Zones appear in this mode and hide when leaving it; use Deployment zones to view them elsewhere.\n"
            "Connections update automatically between matching types, regardless of owner.\n"
            "Units: Units at selects Start or a reinforcement turn; palette -> Place -> click map.\n"
            "Select a unit to edit its arrival/properties, or Move -> click its destination.\n"
            "Reinforcements belong to AI owner 0; technology, facing, health and AP are game-assigned.\n"
            "Scripts: timeline/native records, support, initial/custom and reinforcement/objective-derived AI orders.\n"
            "Pick IDs or paint target regions on the map; Escape ends picking. Apply each form before saving.\n"
            "Script targets are hidden by default; previews are not an AI/combat simulation.\n"
            "Right-click: pick terrain or inspect an object in the active layer.\n"
            "Roads / Walls / Decors: Select -> edit -> Apply, or Move -> click destination.\n"
            "Buildings: choose a type in the palette, then click to place.\n"
            "Select: click a building footprint; edit properties, then Apply.\n"
            "Move: select a building, then click the new footprint origin.\n"
            "Delete object / Delete key: delete the active layer's selected object.\n"
            "Middle-drag or Shift+left-drag: pan. Wheel: scroll.\n"
            "Shift+wheel: horizontal scroll. Ctrl+wheel: zoom.\n"
            "Ctrl+Z / Ctrl+Y: undo / redo all layers in one history.\n"
            "Ctrl+S: save a NEW copy, never overwrite your source.\n\n"
            "Forest tiles contain their background; erasing uses the explicitly selected ground. "
            "Scenery includes rocks, mountains and cracks; water/coast terrain stays protected. "
            "Decors are per-tile rubble and unit wrecks, some of which block movement. Wall/building "
            "previews ignore low HP and show intact artwork because fresh battles reset health. "
            "isDamaged still controls broken concrete sections. Stored HP is preserved; "
            "Full health then Apply properties explicitly edits the stored wall health. "
            "Paved-road bridges connect straight across deep water/riverbanks. Building footprints cannot "
            "overlap trees, buildings, units, roads, walls, ruins or deployment cells. Water "
            "buildings require deep water. Referenced buildings cannot be deleted.\n\n"
            "Roads may pass under reserved unit/deployment positions; walls and decors may not. "
            "Deployment painting edits authored human Turn 0 tiles, not campaign-generated zones or budgets. "
            "AI/unknown-player/nonzero-turn deployment records, Taurans On Map, mines and unverified script "
            "modes are preserved. Scheduled support effects do not reserve placement tiles. "
            "Unit previews show scheduled placements, not simulated movement or animation/technology/color changes. "
            "Building previews use the lowest available artwork level (Deploy Mk1..4 use their "
            "matching levels); scenario technology and player colors are not simulated.\n\n"
            "A saved ZIP replaces only the selected member in a COPY of the original archive. "
            "It does not add a skirmish catalog entry or install into the game. Stored "
            "overview artwork is not regenerated. No map resizing is supported.\n\n"
            "The database bridge runs local 32-bit Windows PowerShell with a process-only "
            "ExecutionPolicy Bypass; it does not change system policy.",
            parent=self.root,
        )

    def close(self):
        if self.busy:
            messagebox.showinfo("Operation in progress", "Wait for the database operation to finish before closing.",
                                parent=self.root)
            return
        self.paint_end()
        if not self.confirm_discard():
            return
        self.executor.shutdown(wait=True)
        self.close_documents()
        self.root.destroy()


def re_prefix(name: str) -> str:
    return "".join(char for char in name if char.isalpha())


def default_game_root() -> Path | None:
    """Uses $LIBDAY_ROOT, else the legacy layout where tools sit inside the game folder."""
    if os.environ.get("LIBDAY_ROOT"):
        return Path(os.environ["LIBDAY_ROOT"])
    legacy = Path(__file__).resolve().parent.parent
    return legacy if (legacy / "GLOBALS" / "GLOBAL.MDB").is_file() else None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root", type=Path, default=default_game_root(),
                        help="Liberation Day install directory (defaults to $LIBDAY_ROOT)")
    parser.add_argument("--open", type=Path, dest="initial")
    parser.add_argument("--member", help="MDB member to open when --open is a ZIP archive")
    args = parser.parse_args()
    if hasattr(ctypes, "windll"):
        ctypes.windll.user32.SetProcessDPIAware()
    root = tk.Tk()
    if args.game_root is None:
        root.withdraw()
        chosen = filedialog.askdirectory(title="Select the Liberation Day install folder", parent=root)
        if not chosen:
            root.destroy()
            raise SystemExit(1)
        args.game_root = Path(chosen)
        root.deiconify()
    try:
        MapEditor(root, args.game_root, args.initial, args.member)
    except (*ERRORS, RuntimeError) as error:
        messagebox.showerror("Cannot start LIBDAY Map Editor", str(error), parent=root)
        root.destroy()
        raise SystemExit(1) from error
    root.mainloop()


if __name__ == "__main__":
    main()
