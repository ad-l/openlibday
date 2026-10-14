"""LIBDAY Asset Editor: view unit/building game data, sprites and animations.

Static sprites come from ``IMGS\\UB.DBI`` and animations from
``ANMSUNIT\\ANMSUNIT.FF``. Sprites and animation frames are exported and
imported as PNG files so they can be edited in any image editor. Edits are
staged until saved as copies or installed into the game (with ``.orig``
backups).
"""
from __future__ import annotations

import argparse
import base64
import ctypes
import tkinter as tk
import tkinter.font as tkfont
from pathlib import Path
from tkinter import filedialog, messagebox, ttk

from asset_editor_core import (ACTIONS, FIELD_GROUPS, HIDDEN_FIELDS, TECH_LEVELS, AssetStore, capabilities,
                               display_value)
from map_editor import default_game_root

ZOOMS = {"1x": 1, "2x": 2, "3x": 3, "4x": 4}


class AssetEditor:
    def __init__(self, root: tk.Tk, game_root: Path):
        self.root = root
        self.store = AssetStore(game_root)
        self.images: dict = {}
        self.kind_var = tk.StringVar(value="Units")
        self.filter_var = tk.StringVar()
        self.status = tk.StringVar(value="Select a unit or building type.")
        self.capability_text = tk.StringVar()
        self.sprite_zoom = tk.StringVar(value="1x")
        self.anim_zoom = tk.StringVar(value="2x")
        self.colour = tk.StringVar()
        self.action = tk.StringVar()
        self.facing = tk.StringVar()
        self.frame_index = tk.IntVar(value=0)
        self.anim_info = tk.StringVar()
        self.selection = None  # ("unit"|"building", type id)
        self.selected_set = None
        self.animations = []
        self.anim_frames: list[tk.PhotoImage] = []
        self.anim_meta: dict = {}
        self.playing = False
        self.play_job = None
        self.build()
        self.sprite_zoom.set(f"{min(4, round(2 * self.scale))}x")
        self.anim_zoom.set(f"{min(4, max(1, round(self.scale)))}x")
        self.refresh_list()
        root.protocol("WM_DELETE_WINDOW", self.close)

    # -- layout -----------------------------------------------------------------

    def px(self, value: int) -> int:
        """Scales a 96-dpi pixel size to the screen."""
        return round(value * self.scale)

    def build(self):
        self.root.title("LIBDAY Asset Editor")
        self.scale = max(1.0, self.root.winfo_fpixels("1i") / 96)
        self.root.geometry(f"{self.px(1400)}x{self.px(900)}")
        linespace = tkfont.nametofont("TkDefaultFont").metrics("linespace")
        ttk.Style(self.root).configure("Treeview", rowheight=linespace + self.px(4))
        top = ttk.Frame(self.root, padding=6)
        top.pack(fill="x")
        for label in ("Units", "Buildings"):
            ttk.Radiobutton(top, text=label, value=label, variable=self.kind_var,
                            command=self.refresh_list).pack(side="left")
        ttk.Label(top, text="  Filter:").pack(side="left")
        entry = ttk.Entry(top, textvariable=self.filter_var, width=24)
        entry.pack(side="left")
        self.filter_var.trace_add("write", lambda *_: self.refresh_list())
        ttk.Button(top, text="Install into game...", command=self.install).pack(side="right")
        ttk.Button(top, text="Save copies...", command=self.save_copies).pack(side="right", padx=4)

        panes = ttk.PanedWindow(self.root, orient="horizontal")
        panes.pack(fill="both", expand=True)
        left = ttk.Frame(panes)
        self.type_list = ttk.Treeview(left, columns=("name", "image"), show="headings", selectmode="browse")
        self.type_list.heading("name", text="Type")
        self.type_list.heading("image", text="Image ID")
        self.type_list.column("name", width=self.px(180))
        self.type_list.column("image", width=self.px(70), stretch=False)
        scroll = ttk.Scrollbar(left, command=self.type_list.yview)
        self.type_list.configure(yscrollcommand=scroll.set)
        self.type_list.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        self.type_list.bind("<<TreeviewSelect>>", self.select_type)
        panes.add(left, weight=1)

        self.notebook = ttk.Notebook(panes)
        panes.add(self.notebook, weight=4)
        self.build_data_tab()
        self.build_sprite_tab()
        self.build_animation_tab()
        ttk.Label(self.root, textvariable=self.status, padding=(8, 4)).pack(fill="x")

    def build_data_tab(self):
        tab = ttk.Frame(self.notebook, padding=6)
        self.notebook.add(tab, text="Game data")
        ttk.Label(tab, textvariable=self.capability_text, wraplength=self.px(1000), justify="left",
                  font=("Segoe UI", 10, "bold")).pack(fill="x", pady=(0, 6))
        columns = ["field", *[f"L{level}" for level in TECH_LEVELS]]
        self.data_tree = ttk.Treeview(tab, columns=columns, show="tree headings", selectmode="browse")
        self.data_tree.column("#0", width=self.px(150), stretch=False)
        self.data_tree.heading("#0", text="Group", anchor="w")
        self.data_tree.heading("field", text="Field", anchor="w")
        self.data_tree.column("field", width=self.px(230), stretch=False, anchor="w")
        for level in TECH_LEVELS:
            self.data_tree.heading(f"L{level}", text=f"Tech {level}", anchor="w")
            self.data_tree.column(f"L{level}", width=self.px(90), minwidth=self.px(60), anchor="w")
        self.data_tree.tag_configure("varies", background="#fff4d6")
        self.data_tree.bind("<<TreeviewSelect>>", self.show_field)
        yscroll = ttk.Scrollbar(tab, command=self.data_tree.yview)
        xscroll = ttk.Scrollbar(tab, orient="horizontal", command=self.data_tree.xview)
        self.data_tree.configure(yscrollcommand=yscroll.set, xscrollcommand=xscroll.set)
        xscroll.pack(side="bottom", fill="x")
        self.data_tree.pack(side="left", fill="both", expand=True)
        yscroll.pack(side="right", fill="y")

    def build_sprite_tab(self):
        tab = ttk.Frame(self.notebook, padding=6)
        self.notebook.add(tab, text="Sprites")
        bar = ttk.Frame(tab)
        bar.pack(fill="x")
        ttk.Label(bar, text="Zoom:").pack(side="left")
        zoom = ttk.Combobox(bar, textvariable=self.sprite_zoom, values=list(ZOOMS), width=4, state="readonly")
        zoom.pack(side="left")
        zoom.bind("<<ComboboxSelected>>", lambda _: self.render_sprites())
        for text, command in (("Export all...", self.export_all_sprites),
                              ("Import folder...", self.import_sprite_folder),
                              ("Export selected...", self.export_selected_sprite),
                              ("Replace selected...", self.replace_selected_sprite)):
            ttk.Button(bar, text=text, command=command).pack(side="left", padx=3)
        self.sprite_info = tk.StringVar()
        ttk.Label(tab, textvariable=self.sprite_info).pack(fill="x", pady=4)
        frame = ttk.Frame(tab)
        frame.pack(fill="both", expand=True)
        self.sprite_canvas = tk.Canvas(frame, background="#3a4a3a", highlightthickness=0)
        ys = ttk.Scrollbar(frame, command=self.sprite_canvas.yview)
        xs = ttk.Scrollbar(frame, orient="horizontal", command=self.sprite_canvas.xview)
        self.sprite_canvas.configure(yscrollcommand=ys.set, xscrollcommand=xs.set)
        xs.pack(side="bottom", fill="x")
        self.sprite_canvas.pack(side="left", fill="both", expand=True)
        ys.pack(side="right", fill="y")
        self.sprite_canvas.bind("<Button-1>", self.click_sprite)
        self.sprite_canvas.bind("<MouseWheel>",
                                lambda e: self.sprite_canvas.yview_scroll(-1 if e.delta > 0 else 1, "units"))

    def build_animation_tab(self):
        tab = ttk.Frame(self.notebook, padding=6)
        self.notebook.add(tab, text="Animations")
        bar = ttk.Frame(tab)
        bar.pack(fill="x")
        self.combos = {}
        for label, variable, width in (("Player colour:", self.colour, 4), ("Action:", self.action, 22),
                                       ("Facing:", self.facing, 4)):
            ttk.Label(bar, text=label).pack(side="left", padx=(6, 2))
            combo = ttk.Combobox(bar, textvariable=variable, width=width, state="readonly")
            combo.pack(side="left")
            combo.bind("<<ComboboxSelected>>", lambda _: self.animation_changed())
            self.combos[label] = combo
        ttk.Label(bar, text="Zoom:").pack(side="left", padx=(12, 2))
        zoom = ttk.Combobox(bar, textvariable=self.anim_zoom, values=list(ZOOMS), width=4, state="readonly")
        zoom.pack(side="left")
        zoom.bind("<<ComboboxSelected>>", lambda _: self.load_animation())
        controls = ttk.Frame(tab)
        controls.pack(fill="x", pady=4)
        self.play_button = ttk.Button(controls, text="Play", command=self.toggle_play)
        self.play_button.pack(side="left")
        self.frame_scale = ttk.Scale(controls, from_=0, to=0, orient="horizontal", length=400,
                                     command=lambda v: self.show_frame(int(float(v))))
        self.frame_scale.pack(side="left", padx=6)
        for text, command in (("Export frames...", self.export_frames), ("Import frames...", self.import_frames),
                              ("Revert", self.revert_animation)):
            ttk.Button(controls, text=text, command=command).pack(side="left", padx=3)
        ttk.Label(tab, textvariable=self.anim_info).pack(fill="x")
        self.anim_canvas = tk.Canvas(tab, background="#202020", highlightthickness=0)
        self.anim_canvas.pack(fill="both", expand=True)

    # -- type list ----------------------------------------------------------------

    def refresh_list(self):
        self.type_list.delete(*self.type_list.get_children())
        text = self.filter_var.get().strip().lower()
        if self.kind_var.get() == "Units":
            items = [("unit", k.type_id, k.name, k.image_id) for k in self.store.units.values()]
        else:
            items = [("building", k.type_id, k.name, k.image_id) for k in self.store.buildings.values()]
        for namespace, type_id, name, image in items:
            label = f"{type_id} {name}"
            if text and text not in label.lower() and text not in image.lower():
                continue
            self.type_list.insert("", "end", iid=f"{namespace}:{type_id}", values=(label, image))

    def select_type(self, _event=None):
        chosen = self.type_list.selection()
        if not chosen:
            return
        namespace, type_id = chosen[0].split(":")
        self.selection = (namespace, int(type_id))
        self.selected_set = None
        self.render_data()
        self.render_sprites()
        self.setup_animations()

    def kind(self):
        namespace, type_id = self.selection
        return self.store.units[type_id] if namespace == "unit" else self.store.buildings[type_id]

    # -- data tab -------------------------------------------------------------------

    def render_data(self):
        tree = self.data_tree
        tree.delete(*tree.get_children())
        namespace, _ = self.selection
        kind = self.kind()
        labels = self.store.labels
        if namespace == "unit":
            levels = kind.levels
            caps = capabilities(kind.base, labels)
            present = sorted(levels)
            self.capability_text.set(f"{kind.name} - tech levels {present[0]}..{present[-1]} "
                                     f"({len(present)} defined)\n" + " | ".join(caps))
            rows = levels
        else:
            rows = {0: kind.row}
            self.capability_text.set(f"{kind.name} - building {kind.size[0]}x{kind.size[1]}, armor "
                                     f"{kind.row.get('Armor')}")
        columns = list(next(iter(rows.values())))
        grouped = {group: [c for c in fields if c in columns] for group, fields in FIELD_GROUPS.items()}
        listed = {c for fields in grouped.values() for c in fields}
        grouped["Other"] = [c for c in columns if c not in listed and c not in HIDDEN_FIELDS]
        shown = [f"L{level}" for level in TECH_LEVELS if level in rows]
        tree.configure(displaycolumns=["field", *shown])
        if namespace == "building":
            tree.heading("L0", text="Value")
            tree.column("L0", width=self.px(500))
        else:
            tree.heading("L0", text="Tech 0")
            tree.column("L0", width=self.px(90))
        for group, fields in grouped.items():
            if not fields:
                continue
            parent = tree.insert("", "end", text=group, open=True)
            for column in fields:
                values = {level: display_value(column, row.get(column), labels) for level, row in rows.items()}
                cells = [values.get(level, "") for level in TECH_LEVELS]
                varies = len(set(values.values())) > 1
                tree.insert(parent, "end", values=(column, *cells), tags=("varies",) if varies else ())

    def show_field(self, _event=None):
        """Shows the untruncated values of the selected data row in the status bar."""
        chosen = self.data_tree.selection()
        values = self.data_tree.item(chosen[0], "values") if chosen else ()
        if not values:
            return
        field, cells = values[0], values[1:]
        shown = [f"T{level}: {cells[level]}" for level in TECH_LEVELS
                 if level < len(cells) and str(cells[level]) != ""]
        unique = {text.split(": ", 1)[1] for text in shown}
        self.status.set(f"{field} = {unique.pop()}" if len(unique) == 1 else f"{field}: " + " | ".join(shown))

    # -- sprites tab ------------------------------------------------------------------

    def sprite_prefix(self):
        return self.kind().image_id

    def sprite_sets(self):
        return self.store.sprite_sets(self.sprite_prefix())

    def photo(self, texture, zoom: int) -> tk.PhotoImage:
        image = tk.PhotoImage(master=self.root, data=base64.b64encode(texture.png()))
        return image.zoom(zoom) if zoom > 1 else image

    def render_sprites(self):
        canvas = self.sprite_canvas
        canvas.delete("all")
        self.images.pop("sprites", None)
        if not self.selection:
            return
        namespace, _ = self.selection
        kind = self.kind()
        sets = self.sprite_sets()
        zoom = ZOOMS[self.sprite_zoom.get()]
        photos = []
        self.sprite_cells = {}
        rows = sorted({a for a, _ in sets})
        cols = sorted({b for _, b in sets})
        size = kind.size
        cell_w = max((size[0] + size[1]) * 27 * zoom + 12, self.px(70))
        cell_h = ((size[0] + size[1] - 2) * 14 + 56) * zoom + 24
        row_name, col_name = ("Frame", "Facing") if namespace == "unit" else ("Level", "State")
        left = self.px(70)
        for c, b in enumerate(cols):
            canvas.create_text(left + c * cell_w + cell_w // 2, self.px(10), text=f"{col_name} {b}", fill="white")
        for r, a in enumerate(rows):
            y = self.px(24) + r * cell_h
            canvas.create_text(left // 2, y + cell_h // 2, text=f"{row_name} {a}", fill="white")
            for c, b in enumerate(cols):
                parts = sets.get((a, b))
                if not parts:
                    continue
                x = left + c * cell_w
                photo = self.photo(self.store.composite(parts, size), zoom)
                photos.append(photo)
                canvas.create_image(x + 6, y + 4, image=photo, anchor="nw")
                edited = any(name.upper() in self.store.sprite_edits for name in parts.values())
                selected = self.selected_set == (a, b)
                outline = "#ffd200" if selected else "#ff8000" if edited else "#556655"
                canvas.create_rectangle(x, y, x + cell_w - 4, y + cell_h - 4, outline=outline,
                                        width=3 if selected else 1)
                self.sprite_cells[(a, b)] = (x, y, x + cell_w, y + cell_h)
        self.images["sprites"] = photos
        canvas.configure(scrollregion=canvas.bbox("all") or (0, 0, 0, 0))
        total = sum(len(p) for p in sets.values())
        self.sprite_info.set(f"{self.sprite_prefix()}: {len(sets)} sets, {total} sprites in UB.DBI "
                             f"(name = ID + {row_name.lower()} + {col_name.lower()} + dx + dy). "
                             + self.selected_sprite_text())

    def selected_sprite_text(self):
        if self.selected_set is None:
            return "Click a set to select it."
        parts = self.sprite_sets().get(self.selected_set, {})
        return "Selected: " + ", ".join(sorted(parts.values()))

    def click_sprite(self, event):
        x, y = self.sprite_canvas.canvasx(event.x), self.sprite_canvas.canvasy(event.y)
        for key, (x0, y0, x1, y1) in getattr(self, "sprite_cells", {}).items():
            if x0 <= x < x1 and y0 <= y < y1:
                self.selected_set = key
                self.render_sprites()
                return

    def export_all_sprites(self):
        if not self.selection:
            return
        folder = filedialog.askdirectory(parent=self.root, title="Export sprites to folder")
        if folder:
            names = sorted(n for parts in self.sprite_sets().values() for n in parts.values())
            self.run(lambda: self.store.export_sprites(names, Path(folder)),
                     f"Exported {len(names)} sprites to {folder}.")

    def import_sprite_folder(self):
        if not self.selection:
            return
        folder = filedialog.askdirectory(parent=self.root, title="Import <sprite name>.png files from folder")
        if not folder:
            return
        names = [n for parts in self.sprite_sets().values() for n in parts.values()]
        result = []
        self.run(lambda: result.extend(self.store.import_sprite_folder(Path(folder), names)), None)
        self.status.set(f"Imported {len(result)} sprites from {folder}." if result
                        else "No matching <sprite name>.png files found.")
        self.render_sprites()

    def selected_parts(self):
        if self.selected_set is None:
            messagebox.showinfo("No sprite selected", "Click a sprite set first.", parent=self.root)
            return None
        return self.sprite_sets()[self.selected_set]

    def export_selected_sprite(self):
        parts = self.selected_parts()
        if parts:
            folder = filedialog.askdirectory(parent=self.root, title="Export selected sprites to folder")
            if folder:
                self.run(lambda: self.store.export_sprites(sorted(parts.values()), Path(folder)),
                         f"Exported {len(parts)} sprite(s) to {folder}.")

    def replace_selected_sprite(self):
        parts = self.selected_parts()
        if not parts:
            return
        if len(parts) == 1:
            name = next(iter(parts.values()))
            path = filedialog.askopenfilename(parent=self.root, title=f"Replace {name} (54x56 PNG)",
                                              filetypes=[("PNG image", "*.png")])
            if path:
                self.run(lambda: self.store.replace_sprites({name: Path(path)}), f"Replaced {name}.")
        else:
            folder = filedialog.askdirectory(parent=self.root,
                                             title="Folder with " + ", ".join(sorted(parts.values())) + " PNGs")
            if folder:
                result = []
                self.run(lambda: result.extend(self.store.import_sprite_folder(Path(folder), list(parts.values()))),
                         None)
                self.status.set(f"Replaced {len(result)} of {len(parts)} parts.")
        self.render_sprites()

    # -- animations tab -------------------------------------------------------------------

    def setup_animations(self):
        namespace, _ = self.selection
        kind = self.kind()
        self.stop()
        if namespace == "unit":
            self.animations = self.store.unit_animations(kind)
        else:
            self.animations = self.store.building_animations(kind)
        unit = namespace == "unit"
        colours = sorted({f"P{a.colour}" for a in self.animations if a.colour is not None})
        self.combos["Player colour:"].configure(values=colours, state="readonly" if unit else "disabled")
        actions = []
        for a in self.animations:
            label = f"{a.action} {ACTIONS[a.action][0]}" if a.action in ACTIONS else (a.action or "Explosion")
            if label not in actions:
                actions.append(label)
        self.combos["Action:"].configure(values=actions)
        self.colour.set(colours[0] if colours else "")
        self.action.set(actions[0] if actions else "")
        self.animation_changed()

    def current_animation(self):
        if not self.animations:
            return None
        if self.animations[0].action is None:
            return self.animations[0]
        colour = int(self.colour.get()[1:] or 0)
        action = self.action.get().split(" ", 1)[0]
        candidates = [a for a in self.animations if a.colour == colour and a.action == action]
        facings = [str(a.facing) for a in candidates]
        self.combos["Facing:"].configure(values=facings, state="readonly")
        if self.facing.get() not in facings:
            self.facing.set(facings[0] if facings else "")
        return next((a for a in candidates if str(a.facing) == self.facing.get()), None)

    def animation_changed(self):
        self.load_animation()

    def load_animation(self):
        self.stop()
        self.anim_canvas.delete("all")
        self.anim_frames = []
        animation = self.current_animation()
        if animation is None:
            self.anim_info.set("No animation for this selection.")
            return
        try:
            paths, meta = self.store.frames(animation.member)
        except RuntimeError as error:
            self.anim_info.set(str(error))
            return
        zoom = ZOOMS[self.anim_zoom.get()]
        for path in paths:
            image = tk.PhotoImage(master=self.root, file=str(path))
            self.anim_frames.append(image.zoom(zoom) if zoom > 1 else image)
        self.anim_meta = meta
        edited = " - EDITED" if animation.member in self.store.animation_edits else ""
        fps = 1_000_000 / meta["frame_micros"] if meta["frame_micros"] else 0
        self.anim_info.set(f"{animation.member}: {meta['frames']} frames, {meta['width']}x{meta['height']}, "
                           f"{fps:.1f} fps, audio {'yes' if meta['audio'] else 'no'}, "
                           f"loops {'yes' if meta['ring_frame'] else 'no'}{edited}. "
                           "Imported frames must keep the frame count and size; palette and sound are kept.")
        self.frame_scale.configure(to=max(0, len(self.anim_frames) - 1))
        self.frame_index.set(0)
        self.show_frame(0)

    def show_frame(self, index: int):
        if not self.anim_frames:
            return
        index = max(0, min(index, len(self.anim_frames) - 1))
        self.frame_index.set(index)
        self.anim_canvas.delete("all")
        self.anim_canvas.create_image(10, 10, image=self.anim_frames[index], anchor="nw")
        self.anim_canvas.create_text(12, 10 + self.anim_frames[index].height() + 10, anchor="nw", fill="white",
                                     text=f"Frame {index + 1}/{len(self.anim_frames)}")

    def toggle_play(self):
        if self.playing:
            self.stop()
        elif self.anim_frames:
            self.playing = True
            self.play_button.configure(text="Pause")
            self.tick()

    def tick(self):
        if not self.playing:
            return
        index = (self.frame_index.get() + 1) % len(self.anim_frames)
        self.frame_scale.set(index)
        self.show_frame(index)
        delay = max(15, int(self.anim_meta.get("frame_micros", 66660) / 1000))
        self.play_job = self.root.after(delay, self.tick)

    def stop(self):
        self.playing = False
        if self.play_job:
            self.root.after_cancel(self.play_job)
            self.play_job = None
        if hasattr(self, "play_button"):
            self.play_button.configure(text="Play")

    def export_frames(self):
        animation = self.current_animation()
        if animation:
            folder = filedialog.askdirectory(parent=self.root, title="Export frames (frame_NNN.png) to folder")
            if folder:
                self.run(lambda: self.store.export_animation(animation.member, Path(folder)),
                         f"Exported {animation.member} to {folder}.")

    def import_frames(self):
        animation = self.current_animation()
        if not animation:
            return
        folder = filedialog.askdirectory(parent=self.root, title="Folder with frame_NNN.png files")
        if folder and self.run(lambda: self.store.replace_animation(animation.member, Path(folder)),
                               f"Re-encoded {animation.member}."):
            self.load_animation()

    def revert_animation(self):
        animation = self.current_animation()
        if animation and animation.member in self.store.animation_edits:
            self.store.revert_animation(animation.member)
            self.load_animation()
            self.status.set(f"Reverted {animation.member}.")

    # -- saving ------------------------------------------------------------------------

    def run(self, action, message) -> bool:
        self.root.configure(cursor="watch")
        self.root.update_idletasks()
        try:
            action()
        except (OSError, RuntimeError, KeyError, ValueError) as error:
            messagebox.showerror("Asset Editor", str(error), parent=self.root)
            return False
        finally:
            self.root.configure(cursor="")
        if message:
            self.status.set(message)
        return True

    def pending_text(self):
        return (f"{len(self.store.sprite_edits)} sprite(s) and {len(self.store.animation_edits)} animation(s) "
                "edited")

    def save_copies(self):
        if not self.store.dirty:
            self.status.set("Nothing to save.")
            return
        folder = filedialog.askdirectory(parent=self.root, title="Folder for edited UB.DBI / ANMSUNIT.FF copies")
        if folder:
            written = []
            self.run(lambda: written.extend(self.store.save_copies(Path(folder))), None)
            self.status.set("Saved " + ", ".join(str(p) for p in written) + ".")

    def install(self):
        if not self.store.dirty:
            self.status.set("Nothing to install.")
            return
        if not messagebox.askyesno("Install into game",
                                   f"{self.pending_text()}. Overwrite the game's UB.DBI / ANMSUNIT.FF?\n"
                                   "A one-time .orig backup of each file is kept.", parent=self.root):
            return
        written = []
        if self.run(lambda: written.extend(self.store.install()), None):
            self.status.set("Installed " + ", ".join(str(p) for p in written) + " (backups: *.orig).")

    def close(self):
        if self.store.dirty and not messagebox.askyesno(
                "Discard edits?", f"{self.pending_text()} and not installed or saved. Quit anyway?",
                parent=self.root):
            return
        self.stop()
        self.store.close()
        self.root.destroy()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-root", type=Path, default=default_game_root(),
                        help="Liberation Day install directory (defaults to $LIBDAY_ROOT)")
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
        AssetEditor(root, args.game_root)
    except (OSError, RuntimeError, ValueError) as error:
        messagebox.showerror("Cannot start LIBDAY Asset Editor", str(error), parent=root)
        root.destroy()
        raise SystemExit(1) from error
    root.mainloop()


if __name__ == "__main__":
    main()
