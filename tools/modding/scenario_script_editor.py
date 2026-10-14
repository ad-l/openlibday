"""Native-backed scenario script controls with nonmodal map picking."""

from __future__ import annotations

import re
import tkinter as tk
from tkinter import messagebox, ttk

from map_editor_core import line_cells
from scenario_scripts import (
    EVENT_TYPES, OBJECTIVES, OBJECTIVE_HELP, ORDERS, SCRIPT_FIELDS, diagnostics, reinforcement_order, timeline,
)


def integer(text: str) -> int:
    if not re.fullmatch(r"-?[0-9]+", text.strip()):
        raise ValueError("Enter an integer, not a decimal or expression.")
    return int(text)


def id_values(text: str) -> tuple[int, ...]:
    return tuple(integer(value) for value in re.split(r"[,\s]+", text.strip())) if text.strip() else ()


def position_values(text: str) -> tuple[tuple[int, int], ...]:
    result = []
    for line in text.splitlines():
        if not line.strip():
            continue
        values = re.split(r"[,\s]+", line.strip())
        if len(values) != 2:
            raise ValueError("Enter one X,Y tile per line.")
        result.append((integer(values[0]), integer(values[1])))
    return tuple(result)


class ScriptDialog(tk.Toplevel):
    def __init__(self, editor):
        super().__init__(editor.root)
        self.editor = editor
        self.document = editor.document
        self.selected_event: int | None = None
        self.selected_order: int | None = None
        self.picking: str | None = None
        self.pick_last = None
        self.pick_stroke = False
        self.pick_erase = tk.BooleanVar(value=False)
        self.info = tk.StringVar(value="Edits enter map history only when applied. Closing discards unapplied form values.")
        self.title("Scenario scripts - native actions")
        self.geometry("1000x820")
        self.minsize(820, 700)
        self.transient(editor.root)
        self.protocol("WM_DELETE_WINDOW", self.close)
        body = ttk.Frame(self, padding=12)
        body.pack(fill="both", expand=True)
        self.notebook = ttk.Notebook(body)
        self.notebook.pack(fill="both", expand=True)
        self.pages = {}
        for name in ("Timeline / diagnostics", "Scheduled support", "AI orders", "Objective"):
            page = ttk.Frame(self.notebook, padding=10)
            self.pages[name] = page
            self.notebook.add(page, text=name)
        self._inspector()
        self._support()
        self._orders()
        self._objective()
        controls = ttk.Frame(body)
        controls.pack(fill="x", pady=(10, 0))
        ttk.Button(controls, text="Done picking", command=self.stop_pick).pack(side="left")
        ttk.Checkbutton(controls, text="Erase picked tiles", variable=self.pick_erase).pack(side="left", padx=12)
        ttk.Button(controls, text="Refresh from map", command=self.refresh).pack(side="left")
        ttk.Button(controls, text="Close", command=self.close).pack(side="right")
        ttk.Label(body, textvariable=self.info, wraplength=940).pack(anchor="w", pady=(8, 0))
        self.bind("<Escape>", lambda _: self.stop_pick())
        self.refresh()
        self.notebook.bind("<<NotebookTabChanged>>", self.tab_changed)

    @property
    def model(self):
        return self.document.model

    @property
    def active_tab(self):
        return self.notebook.tab(self.notebook.select(), "text")

    def tab_changed(self, event):
        if event.widget is self.notebook:
            self.stop_pick()

    def _tree(self, parent, columns, height=8):
        frame = ttk.Frame(parent)
        frame.pack(fill="both", expand=True)
        tree = ttk.Treeview(frame, columns=columns, show="headings", height=height, selectmode="browse")
        for column in columns:
            tree.heading(column, text=column)
            tree.column(column, width=95 if column != "Details" else 550, minwidth=60)
        vertical = ttk.Scrollbar(frame, command=tree.yview)
        horizontal = ttk.Scrollbar(frame, orient="horizontal", command=tree.xview)
        tree.configure(yscrollcommand=vertical.set, xscrollcommand=horizontal.set)
        vertical.pack(side="right", fill="y")
        horizontal.pack(side="bottom", fill="x")
        tree.pack(fill="both", expand=True)
        return tree

    def _text(self, parent, caption, height=6):
        frame = ttk.LabelFrame(parent, text=caption, padding=5)
        frame.pack(fill="both", expand=True, pady=5)
        text = tk.Text(frame, height=height, wrap="none", undo=True)
        scroll = ttk.Scrollbar(frame, command=text.yview)
        text.configure(yscrollcommand=scroll.set)
        scroll.pack(side="right", fill="y")
        text.pack(fill="both", expand=True)
        return text

    @staticmethod
    def put(text, value):
        text.configure(state="normal")
        text.delete("1.0", "end")
        text.insert("1.0", value)
        text.edit_reset()

    @staticmethod
    def value(text):
        return text.get("1.0", "end-1c")

    def _field(self, frame, row, caption, variable, values=None):
        ttk.Label(frame, text=caption).grid(row=row, column=0, sticky="w", padx=(0, 12), pady=4)
        widget = (ttk.Combobox(frame, textvariable=variable, values=values, state="readonly")
                  if values is not None else ttk.Entry(frame, textvariable=variable))
        widget.grid(row=row, column=1, sticky="ew", pady=4)
        frame.columnconfigure(1, weight=1)
        return widget

    def _inspector(self):
        page = self.pages["Timeline / diagnostics"]
        ttk.Label(page, text="Arrival, order expiry and hold duration are different clocks. "
                  "This is an inspector, not a combat/AI simulation.", wraplength=900).pack(anchor="w", pady=5)
        tabs = ttk.Notebook(page)
        tabs.pack(fill="both", expand=True)
        timeline_page, raw_page, warnings_page = (ttk.Frame(tabs, padding=8) for _ in range(3))
        tabs.add(timeline_page, text="Timeline")
        tabs.add(raw_page, text="Native records")
        tabs.add(warnings_page, text="Diagnostics")
        self.timeline_tree = self._tree(timeline_page, ("Turn / value", "Timing", "ID / row", "Details"))
        self.raw_table = tk.StringVar(value="Automated Units")
        self.raw_choice = ttk.Combobox(raw_page, textvariable=self.raw_table, state="readonly")
        self.raw_choice.pack(fill="x", pady=(0, 8))
        self.raw_choice.bind("<<ComboboxSelected>>", lambda _: self.refresh_raw())
        self.raw_tree = self._tree(raw_page, ())
        self.warnings = self._text(warnings_page, "Warnings do not repair or discard original records", height=18)

    def _support(self):
        page = self.pages["Scheduled support"]
        self.event_tree = self._tree(page, ("Row", "Turn", "Type", "Source", "Destination"), height=6)
        self.event_tree.bind("<<TreeviewSelect>>", self.choose_event)
        fields = ttk.Frame(page)
        fields.pack(fill="x", pady=8)
        self.event_kind = tk.StringVar(value=self.model.support_types[14].label)
        self.event_turn = tk.StringVar(value="1")
        self.event_x, self.event_y, self.event_dx, self.event_dy = (tk.StringVar(value="0") for _ in range(4))
        labels = [self.model.support_types[kind].label for kind in sorted(EVENT_TYPES)]
        self.event_kind_box = self._field(fields, 0, "Native support type", self.event_kind, labels)
        self.event_kind_box.bind("<<ComboboxSelected>>", self.support_kind_changed)
        self.event_turn_box = self._field(fields, 1, "Arrival turn (1..99, exact match)", self.event_turn)
        coords = ttk.Frame(page)
        coords.pack(fill="x")
        self.event_coordinate_widgets = []
        for caption, variable in (("Source X", self.event_x), ("Y", self.event_y),
                                  ("Destination X", self.event_dx), ("Y", self.event_dy)):
            ttk.Label(coords, text=caption).pack(side="left", padx=(0, 4))
            entry = ttk.Entry(coords, textvariable=variable, width=7)
            entry.pack(side="left", padx=(0, 12))
            self.event_coordinate_widgets.append(entry)
        self.event_description = tk.StringVar()
        self.event_icon = ttk.Label(page)
        self.event_icon.pack(anchor="w", pady=6)
        ttk.Label(page, textvariable=self.event_description, wraplength=900).pack(anchor="w", pady=6)
        buttons = ttk.Frame(page)
        buttons.pack(fill="x", pady=6)
        self.event_buttons = {}
        for text, command in (("New", self.new_event), ("Pick source / target", lambda: self.start_pick("source")),
                              ("Pick destination", lambda: self.start_pick("destination")),
                              ("Apply event", self.apply_event), ("Delete event", self.delete_event)):
            button = ttk.Button(buttons, text=text, command=command)
            button.pack(side="left", padx=(0, 7))
            self.event_buttons[text] = button
        ttk.Label(page, text="Non-transports use a single effect tile: destination is synchronized on Apply. "
                  "Requests belong to AI owner 0 and are consumed without arrival retry. "
                  "Payloads/damage are read-only cached GLOBAL definitions; no GLOBAL edits.",
                  wraplength=900).pack(anchor="w", pady=5)

    def _orders(self):
        page = self.pages["AI orders"]
        fields = ttk.Frame(page)
        fields.pack(fill="x")
        self.order_unit = tk.StringVar()
        self.order_kind = tk.StringVar(value="0: No Order")
        self.order_turn = tk.StringVar(value="99")
        self.order_probability = tk.StringVar(value="100")
        self.order_enemy = tk.StringVar(value="65535: No specific enemy type")
        self.order_unit_box = self._field(fields, 0, "Unit (initial or reinforcement)", self.order_unit, [])
        self.order_unit_box.bind("<<ComboboxSelected>>", self.choose_order_unit)
        self.order_kind_box = self._field(fields, 1, "Native AI order", self.order_kind,
                                         [f"{kind}: {name}" for kind, name in ORDERS.items()])
        self.order_kind_box.bind("<<ComboboxSelected>>", lambda _: self.order_controls())
        self.order_turn_box = self._field(fields, 2, "Active through turn (not arrival)", self.order_turn)
        self.order_probability_box = self._field(fields, 3, "Retention value (0..100)", self.order_probability)
        self.order_enemy_box = self._field(fields, 4, "Preferred enemy military type", self.order_enemy,
                    ["65535: No specific enemy type", *[self.editor.unit_label(kind)
                     for kind, tech in sorted(self.editor.unit_types) if tech == 0]])
        self.reinforcement_controls = ttk.Frame(page)
        self.reinforcement_order = tk.BooleanVar(value=False)
        ttk.Checkbutton(self.reinforcement_controls, text="Use objective-derived AI order",
                        variable=self.reinforcement_order, command=self.preview_reinforcement_order).pack(anchor="w")
        self.reinforcement_description = tk.StringVar()
        ttk.Label(self.reinforcement_controls, textvariable=self.reinforcement_description,
                  wraplength=900).pack(anchor="w")
        self.order_ids = self._text(page, "Target IDs: Escort / Defend / Attack (units or buildings)", height=3)
        self.order_positions = self._text(page, "Target tiles: Goto / Goto Fast / Patrol; one X,Y per line", height=4)
        buttons = ttk.Frame(page)
        buttons.pack(fill="x", pady=6)
        self.order_buttons = {}
        for text, command in (("Pick ordered unit", lambda: self.start_pick("unit")),
                              ("Pick target IDs", lambda: self.start_pick("order_ids")),
                              ("Paint target tiles", lambda: self.start_pick("order_tiles")),
                              ("Clear targets", self.clear_order_targets), ("Apply order", self.apply_order)):
            button = ttk.Button(buttons, text=text, command=command)
            button.pack(side="left", padx=(0, 7))
            self.order_buttons[text] = button
        ttk.Label(page, text="Targets are collections, not a sequential route. Unused old target rows are retained "
                  "unless explicitly cleared. A failed retention roll clears the order; value 0 still permits roll 0. "
                  "Reinforcements use the objective-derived checkbox above; their generated order/targets are read-only.",
                  wraplength=900).pack(anchor="w", pady=5)

    def _objective(self):
        page = self.pages["Objective"]
        fields = ttk.Frame(page)
        fields.pack(fill="x")
        self.goal_kind = tk.StringVar()
        self.goal_turn = tk.StringVar()
        self.goal_kind_box = self._field(fields, 0, "Single native objective", self.goal_kind,
                                        [f"{kind}: {OBJECTIVES[kind]}" for kind in range(7)])
        self.goal_kind_box.bind("<<ComboboxSelected>>", lambda _: self.goal_description_changed())
        self.goal_turn_box = self._field(fields, 1, "Raw Hold Position threshold (0..99)", self.goal_turn)
        self.goal_description = tk.StringVar()
        ttk.Label(page, textvariable=self.goal_description, wraplength=900).pack(anchor="w", pady=10)
        self.goal_ids = self._text(page, "Unit/building IDs or survival exceptions (kind-specific)", height=5)
        self.goal_positions = self._text(page, "Region / destination / road-mine tiles: one X,Y per line", height=7)
        buttons = ttk.Frame(page)
        buttons.pack(fill="x", pady=6)
        self.goal_buttons = {}
        for text, command in (("Pick target IDs", lambda: self.start_pick("goal_ids")),
                              ("Paint target region", lambda: self.start_pick("goal_tiles")),
                              ("Apply objective", self.apply_goal)):
            button = ttk.Button(buttons, text=text, command=command)
            button.pack(side="left", padx=(0, 7))
            self.goal_buttons[text] = button
        ttk.Label(page, text="Briefing text is separate in Scenario settings. Keep/Protect use the global turn limit. "
                  "Changing this preset also changes objective-derived reinforcement orders. Deathmatch/internal "
                  "objectives and unexplained fields remain preserve-only.", wraplength=900).pack(anchor="w", pady=5)

    def operation(self, action):
        try:
            if self.editor.busy or self.editor.document is not self.document:
                raise ValueError("Wait for the current operation; this form belongs to its original map variant.")
            self.stop_pick()
            action()
            self.editor.render_map()
            self.refresh()
            self.info.set("Applied to map history. Save Copy or export the skirmish to persist it.")
            self.editor.status.set("Scenario script edit applied; original files are unchanged.")
        except (ValueError, KeyError, tk.TclError) as error:
            self.info.set(f"Cannot apply: {error}")
            messagebox.showerror("Cannot edit scenario script", str(error), parent=self)

    def refresh(self):
        self.stop_pick()
        self.refresh_inspection()
        self.event_tree.delete(*self.event_tree.get_children())
        for event in self.model.scripts.events:
            definition = self.model.support_types.get(event.kind)
            self.event_tree.insert("", "end", iid=str(event.counter), values=(
                event.counter, event.turn, definition.label if definition else f"Unknown {event.kind}",
                f"{event.x},{event.y}", f"{event.dest_x},{event.dest_y}"))
        if self.selected_event is not None and self.event_tree.exists(str(self.selected_event)):
            self.event_tree.selection_set(str(self.selected_event))
            self.fill_event()
        else:
            self.selected_event = None
            self.support_kind_changed()
        self.order_unit_box.configure(values=[self.order_label(unit) for unit in sorted(
            self.model.units.values(), key=lambda unit: unit.identifier)])
        self.fill_order()
        self.fill_goal()

    def refresh_inspection(self):
        self.timeline_tree.delete(*self.timeline_tree.get_children())
        for row in timeline(self.model):
            self.timeline_tree.insert("", "end", values=row)
        issues = diagnostics(self.model)
        self.put(self.warnings, "\n\n".join(issues) if issues else "No script diagnostics.")
        self.warnings.configure(state="disabled")
        self.raw_choice.configure(values=(*self.model.scripts.records(), "Starting-unit AI fields", "Reinforcement"))
        self.refresh_raw()

    def refresh_raw(self):
        table = self.raw_table.get()
        if table == "Starting-unit AI fields":
            rows = [{"ID": unit.identifier, "Owner": unit.owner, "OrderID": unit.order_id,
                     "Turn": unit.order_turn, "Probability": unit.probability, "EnemyUnitType": unit.enemy_type}
                    for unit in self.model.units.values() if unit.layer == "units"]
        elif table == "Reinforcement":
            rows = self.model.unit_rows()["reinforcements"]
        else:
            rows = self.model.scripts.rows()[table]
        columns = tuple(rows[0]) if rows else SCRIPT_FIELDS.get(table, ("Empty table",))
        self.raw_tree.configure(columns=columns)
        for column in columns:
            self.raw_tree.heading(column, text=column)
            self.raw_tree.column(column, width=120, minwidth=70)
        self.raw_tree.delete(*self.raw_tree.get_children())
        for row in rows:
            self.raw_tree.insert("", "end", values=[row[column] for column in columns])

    def choose_event(self, _=None):
        selection = self.event_tree.selection()
        if selection:
            self.stop_pick()
            self.selected_event = int(selection[0])
            self.fill_event()
            self.editor.render_scripts()

    def fill_event(self):
        event = next((item for item in self.model.scripts.events if item.counter == self.selected_event), None)
        if event is None:
            return
        support = self.model.support_types.get(event.kind)
        self.event_kind.set(support.label if support else f"{event.kind}: Unknown support")
        self.event_turn.set(str(event.turn))
        for variable, value in zip((self.event_x, self.event_y, self.event_dx, self.event_dy),
                                   (event.x, event.y, event.dest_x, event.dest_y)):
            variable.set(str(value))
        self.event_kind_box.configure(state="readonly" if event.editable else "disabled")
        self.support_kind_changed()

    def support_kind_changed(self, _=None):
        kind = integer(self.event_kind.get().split(":")[0])
        support = self.model.support_types.get(kind)
        self.event_description.set(support.description() if support else "Unknown support: preserve-only.")
        event = next((item for item in self.model.scripts.events if item.counter == self.selected_event), None)
        editable = event is None or event.editable
        self.event_kind_box.configure(state="readonly" if editable else "disabled")
        self.event_turn_box.configure(state="normal" if editable else "disabled")
        for index, widget in enumerate(self.event_coordinate_widgets):
            widget.configure(state="normal" if editable and (index < 2 or kind in (0, 7)) else "disabled")
        for text, button in self.event_buttons.items():
            enabled = text == "New" or editable and (
                text != "Delete event" or event is not None) and (text != "Pick destination" or kind in (0, 7))
            button.configure(state="normal" if enabled else "disabled")
        if support and support.category == 4:
            identifier = int(dict(support.properties)["unitIDDrop"])
            self.event_icon.configure(image=self.editor.unit_thumbnail(identifier), text="")
        else:
            self.event_icon.configure(image="", text="")

    def new_event(self):
        self.stop_pick()
        self.selected_event = None
        self.event_tree.selection_remove(*self.event_tree.selection())
        self.event_kind_box.configure(state="readonly")
        self.event_kind.set(self.model.support_types[14].label)
        self.event_turn.set("1")
        self.support_kind_changed()
        self.info.set("New event: choose its type/turn and pick a target, then Apply event.")
        self.editor.render_scripts()

    def apply_event(self):
        def apply():
            kind = integer(self.event_kind.get().split(":")[0])
            x, y, turn = map(integer, (self.event_x.get(), self.event_y.get(), self.event_turn.get()))
            dx, dy = (integer(self.event_dx.get()), integer(self.event_dy.get())) if kind in (0, 7) else (x, y)
            if self.selected_event is None:
                event = self.model.add_support(kind, x, y, turn, dx, dy)
            else:
                event = self.model.update_support(self.selected_event, kind=kind, x=x, y=y,
                                                  dest_x=dx, dest_y=dy, turn=turn)
            self.selected_event = event.counter
        self.operation(apply)

    def delete_event(self):
        if self.selected_event is None:
            self.info.set("Select an existing event to delete.")
            return
        def delete():
            self.model.delete_support(self.selected_event)
            self.selected_event = None
        self.operation(delete)

    def order_label(self, unit):
        timing = f"reinforcement turn {unit.arrival}" if unit.layer == "reinforcements" else f"initial, owner {unit.owner}"
        return f"{unit.identifier}: {self.editor.unit_types[unit.kind, 0].name} ({timing})"

    def choose_order_unit(self, _=None):
        self.stop_pick()
        self.selected_order = integer(self.order_unit.get().split(":")[0])
        self.fill_order()
        unit = self.model.units.get(self.selected_order)
        if unit is not None:
            self.editor.unit_view.set("Start" if unit.arrival == 0 else f"Turn {unit.arrival}")
            if self.editor.mode.get() == "Units":
                self.editor.select_unit(unit)
            self.editor.render_map()
        else:
            self.editor.render_scripts()

    def fill_order(self):
        unit = self.model.units.get(self.selected_order)
        if unit is None:
            self.selected_order = None
            self.order_unit.set("")
            self.put(self.order_ids, "")
            self.put(self.order_positions, "")
            self.order_controls()
            return
        self.order_unit.set(self.order_label(unit))
        if unit.layer == "reinforcements":
            self.reinforcement_order.set(unit.arrival_order)
            self.preview_reinforcement_order()
            return
        self.order_kind.set(f"{unit.order_id}: {ORDERS.get(unit.order_id, 'Unverified (preserve-only)')}")
        self.order_kind_box.configure(state="readonly" if unit.order_id in ORDERS else "disabled")
        self.order_turn.set(str(unit.order_turn))
        self.order_probability.set(str(unit.probability))
        self.order_enemy.set("65535: No specific enemy type" if unit.enemy_type == 65535 else
                             self.editor.unit_label(unit.enemy_type) if (unit.enemy_type, 0) in self.editor.unit_types
                             else f"{unit.enemy_type}: Unknown (preserve-only)")
        self.put(self.order_ids, "\n".join(str(row.identifier) for row in self.model.scripts.order_ids
                                          if row.unit == unit.identifier))
        self.put(self.order_positions, "\n".join(f"{row.x},{row.y}" for row in self.model.scripts.order_positions
                                                if row.unit == unit.identifier))
        self.order_controls()

    def preview_reinforcement_order(self):
        derived = reinforcement_order(self.model.scripts, self.reinforcement_order.get())
        self.order_kind.set(f"{derived[0]}: {ORDERS[derived[0]]}" if derived else "0: Unknown derived order")
        self.order_turn.set("99")
        self.order_probability.set("100")
        self.order_enemy.set("65535: No specific enemy type")
        self.put(self.order_ids, "\n".join(map(str, derived[1])) if derived else "")
        self.put(self.order_positions, "\n".join(f"{x},{y}" for x, y in derived[2]) if derived else "")
        self.reinforcement_description.set(
            "Only the checkbox is saved for reinforcements. Order and targets are derived from the "
            "current objective at arrival; expiry 99 and retention 100 are fixed by the engine. "
            "The map shows the applied flag until Apply order."
            if derived else "Cannot preview: the map does not contain exactly one objective. "
            "Only the native order flag is editable.")
        self.order_controls()

    def order_controls(self):
        unit = self.model.units.get(self.selected_order)
        reinforcement = unit is not None and unit.layer == "reinforcements"
        self.reinforcement_controls.pack_forget()
        if reinforcement:
            self.reinforcement_controls.pack(fill="x", before=self.order_ids.master)
        editable = unit is not None and unit.layer == "units" and unit.order_id in ORDERS
        self.order_kind_box.configure(state="readonly" if editable else "disabled")
        for widget in (self.order_turn_box, self.order_probability_box):
            widget.configure(state="normal" if editable else "disabled")
        self.order_enemy_box.configure(state="readonly" if editable else "disabled")
        for widget in (self.order_ids, self.order_positions):
            widget.configure(state="normal" if editable else "disabled")
        kind = integer(self.order_kind.get().split(":")[0])
        for text, button in self.order_buttons.items():
            enabled = text == "Pick ordered unit" or reinforcement and text == "Apply order" or editable and (
                text != "Pick target IDs" or kind in (3, 4, 6)) and (
                text != "Paint target tiles" or kind in (1, 2, 5))
            button.configure(state="normal" if enabled else "disabled")

    def clear_order_targets(self):
        unit = self.model.units.get(self.selected_order)
        if unit is None or unit.layer != "units":
            return
        self.put(self.order_ids, "")
        self.put(self.order_positions, "")
        self.info.set("Targets cleared in the form only; Apply order to remove the native rows.")

    def apply_order(self):
        def apply():
            if self.selected_order is None:
                raise ValueError("Choose an initial unit or reinforcement.")
            unit = self.model.units.get(self.selected_order)
            if unit is not None and unit.layer == "reinforcements":
                self.model.update_unit(unit.identifier, arrival_order=self.reinforcement_order.get())
                return
            self.model.set_unit_order(
                self.selected_order, integer(self.order_kind.get().split(":")[0]), integer(self.order_turn.get()),
                integer(self.order_probability.get()), integer(self.order_enemy.get().split(":")[0]),
                id_values(self.value(self.order_ids)), position_values(self.value(self.order_positions)))
        self.operation(apply)

    def fill_goal(self):
        goals = self.model.scripts.objectives
        if len(goals) != 1:
            self.goal_kind.set("")
            self.goal_kind_box.configure(state="disabled")
            self.goal_description.set("Nonstandard objective record count: preserve-only.")
            self.goal_turn_box.configure(state="disabled")
            for button in self.goal_buttons.values():
                button.configure(state="disabled")
        else:
            goal = goals[0]
            self.goal_kind.set(f"{goal.kind}: {OBJECTIVES.get(goal.kind, 'Unknown (preserve-only)')}")
            self.goal_turn.set(str(goal.turn))
            self.goal_kind_box.configure(state="readonly" if goal.kind in range(7) else "disabled")
            self.goal_description_changed()
        self.put(self.goal_ids, "\n".join(str(row.identifier) for row in self.model.scripts.objective_ids))
        self.put(self.goal_positions, "\n".join(f"{row.x},{row.y}" for row in self.model.scripts.objective_positions))

    def goal_description_changed(self):
        kind = integer(self.goal_kind.get().split(":")[0])
        self.goal_description.set(OBJECTIVE_HELP.get(kind, "Unverified preset: preserve-only."))
        self.goal_turn_box.configure(state="normal" if kind == 1 else "disabled")
        editable = len(self.model.scripts.objectives) == 1 and self.model.scripts.objectives[0].kind in range(7)
        for text, button in self.goal_buttons.items():
            enabled = editable and (text != "Pick target IDs" or kind in (0, 2, 4, 5, 6)) and (
                text != "Paint target region" or kind in (0, 1, 2, 3, 4))
            button.configure(state="normal" if enabled else "disabled")

    def apply_goal(self):
        def apply():
            kind = integer(self.goal_kind.get().split(":")[0])
            if len(self.model.scripts.objectives) != 1:
                raise ValueError("A single native objective is required.")
            turn = integer(self.goal_turn.get()) if kind == 1 else self.model.scripts.objectives[0].turn
            self.model.set_objective(kind, turn, id_values(self.value(self.goal_ids)),
                                     position_values(self.value(self.goal_positions)))
        self.operation(apply)

    def start_pick(self, mode):
        try:
            if self.editor.busy:
                raise ValueError("Wait for the current operation.")
            if mode.startswith("order_") and self.selected_order is None:
                raise ValueError("Choose the ordered unit first.")
            if mode.startswith("order_") and self.model.units[self.selected_order].layer == "reinforcements":
                raise ValueError("Reinforcement targets are derived from the objective, not independently editable.")
            if mode == "destination" and integer(self.event_kind.get().split(":")[0]) not in (0, 7):
                raise ValueError("Only transport events have a separate destination.")
            if mode.endswith("ids"):
                widget = self.order_ids if mode == "order_ids" else self.goal_ids
                id_values(self.value(widget))
            if mode.endswith("tiles"):
                widget = self.order_positions if mode == "order_tiles" else self.goal_positions
                position_values(self.value(widget))
            self.editor.paint_end()
            self.picking = mode
            self.pick_last = None
            self.pick_stroke = False
            self.info.set(f"Picking {mode.replace('_', ' ')} on the map. Pan/zoom still work; "
                          "Done picking/Escape stops it. Apply the form to commit.")
            self.editor.status.set(self.info.get())
            if self.editor.terrain_view:
                self.editor.terrain_only.set(False)
                self.editor.render_map()
            else:
                self.editor.render_scripts()
        except (ValueError, KeyError, tk.TclError) as error:
            messagebox.showerror("Cannot pick script target", str(error), parent=self)

    def stop_pick(self):
        self.picking = None
        self.pick_last = None
        self.pick_stroke = False
        self.editor.render_scripts()

    def map_start(self, key):
        self.pick_stroke = True
        self.pick_last = None
        self.map_drag(key)

    def map_drag(self, key):
        if key is None or not self.pick_stroke:
            self.pick_last = None
            return
        try:
            mode = self.picking
            if mode in ("source", "destination"):
                variables = (self.event_x, self.event_y) if mode == "source" else (self.event_dx, self.event_dy)
                for variable, value in zip(variables, key):
                    variable.set(str(value))
                if mode == "source" and integer(self.event_kind.get().split(":")[0]) not in (0, 7):
                    self.event_dx.set(str(key[0]))
                    self.event_dy.set(str(key[1]))
                self.pick_stroke = False
            elif mode == "unit" or mode and mode.endswith("ids"):
                units = self.model.units_at(key, self.editor.arrival_value(self.editor.unit_view.get()))
                building = self.model.building_at(key)
                if mode == "unit":
                    unit = next(iter(units), None)
                    if unit is None:
                        raise ValueError("Click a unit in the current Units at view, or choose it in the dropdown.")
                    self.selected_order = unit.identifier
                    self.fill_order()
                else:
                    identifier = units[0].identifier if units else building.identifier if building else None
                    if identifier is None:
                        raise ValueError("Click a unit/building, not an empty tile.")
                    widget = self.order_ids if mode == "order_ids" else self.goal_ids
                    values = id_values(self.value(widget))
                    if identifier not in values:
                        self.put(widget, "\n".join(map(str, (*values, identifier))))
                self.pick_stroke = False
            elif mode and mode.endswith("tiles"):
                widget = self.order_positions if mode == "order_tiles" else self.goal_positions
                values = list(position_values(self.value(widget)))
                points = line_cells(self.pick_last, key) if self.pick_last is not None else [key]
                radius = int(self.editor.brush.get()[0]) // 2
                for x, y in points:
                    for dx in range(-radius, radius + 1):
                        for dy in range(-radius, radius + 1):
                            point = x + dx, y + dy
                            if point not in self.model.cells:
                                continue
                            if self.pick_erase.get():
                                values = [value for value in values if value != point]
                            elif point not in values:
                                values.append(point)
                self.put(widget, "\n".join(f"{x},{y}" for x, y in values))
                self.pick_last = key
            self.editor.render_scripts()
        except (ValueError, KeyError, tk.TclError) as error:
            self.stop_pick()
            messagebox.showerror("Cannot pick script target", str(error), parent=self)

    def preview_tiles(self):
        if self.picking in ("order_ids", "goal_ids"):
            widget = self.order_ids if self.picking == "order_ids" else self.goal_ids
            entities = (self.model.units.get(identifier) or self.model.buildings.get(identifier)
                        for identifier in id_values(self.value(widget)))
            return tuple((entity.x, entity.y) for entity in entities if entity is not None)
        if self.picking in ("order_tiles", "goal_tiles"):
            widget = self.order_positions if self.picking == "order_tiles" else self.goal_positions
            return position_values(self.value(widget))
        if self.picking in ("source", "destination"):
            return ((integer(self.event_x.get()), integer(self.event_y.get())),
                    (integer(self.event_dx.get()), integer(self.event_dy.get())))
        return ()

    def close(self):
        self.picking = None
        self.editor.script_dialog = None
        self.editor.render_scripts()
        self.destroy()
