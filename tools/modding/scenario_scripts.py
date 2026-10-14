"""Lossless native scenario records and read-only GLOBAL support definitions."""

from __future__ import annotations

from collections import Counter, defaultdict
import csv
from dataclasses import dataclass, replace
import json
from pathlib import Path


ORDERS = {0: "No Order", 1: "Goto", 2: "Goto Fast", 3: "Escort", 4: "Defend", 5: "Patrol", 6: "Attack"}
OBJECTIVES = {0: "Destroy", 1: "Hold Position", 2: "Escort", 3: "Protect", 4: "Intercept",
              5: "Keep", 6: "Destroy All Except", 7: "Counterattack (internal)",
              8: "Death match 2 cities", 9: "Death match city + alien", 10: "Death match slug fest"}
OBJECTIVE_HELP = {
    0: "Destroy the listed units/buildings and clear roads/mines at the listed tiles (not walls).",
    1: "Non-AI units on any target tile contribute once per turn. Native count must exceed the raw "
       "TurnNumber; gap/reset and visible duration are not fully verified.",
    2: "A listed surviving unit reaching any destination satisfies the native predicate. "
       "Full mission outcome handling is not yet verified.",
    3: "Keep AI units out of all target tiles at the scenario's global turn limit.",
    4: "Stop listed units reaching any target tile. Destroying all listed units satisfies the predicate.",
    5: "Keep every listed unit/building alive until after the global scenario turn limit.",
    6: "Keep exception IDs alive and eliminate all other AI military units (not every enemy building).",
}
EVENT_TYPES = frozenset((0, 4, 5, 6, 7, 11, 12, 13, 14, 19, 20, 21, 22, 23, 25))
SCRIPT_FIELDS = {
    "Automated Units": ("Counter", "AutomatedType", "StartX", "StartY", "DestX", "DestY", "TurnArrival"),
    "Objective": ("Counter", "ObjectifId", "TurnNumber"),
    "Objectives ID List": ("Counter", "Id"),
    "Objectives Pos List": ("Counter", "PosX", "PosY"),
    "Unit Order ID List": ("Counter", "Unit ID", "Id"),
    "Unit Order Pos List": ("Counter", "Unit ID", "PosX", "PosY"),
}


def reinforcement_order(data, enabled: bool) -> tuple[int, tuple[int, ...], tuple[tuple[int, int], ...]] | None:
    """Mirror the objective-derived spawn helper at 0x41671C; None means ambiguous metadata."""
    if not enabled:
        return 0, (), ()
    if data is None or len(data.objectives) != 1:
        return None
    objective = data.objectives[0].kind
    ids = tuple(row.identifier for row in data.objective_ids)
    positions = tuple((row.x, row.y) for row in data.objective_positions)
    kind = (3 if ids else 5) if objective == 0 else {1: 5, 2: 6, 3: 5, 4: 3, 5: 6}.get(objective, 0)
    return kind, ids if kind in (3, 6) else (), positions if kind == 5 else ()


@dataclass(frozen=True)
class SupportType:
    identifier: int
    name: str
    race: int
    category: int
    properties: tuple[tuple[str, str], ...]

    @classmethod
    def from_row(cls, row: dict) -> SupportType:
        properties = json.loads(row["properties"]) if isinstance(row["properties"], str) else row["properties"]
        return cls(int(row["typeID"]), str(row["NameEnglish"]), int(row["Race"]), int(row["Category"]),
                   tuple(sorted((str(key), str(value)) for key, value in properties.items())))

    @property
    def label(self) -> str:
        race = {0: "Human", 1: "Tauran", 2: "Shubblar", 3: "Shadamar"}.get(self.race, f"Race {self.race}")
        return f"{self.identifier}: {race} {self.name}"

    @property
    def offsets(self) -> tuple[tuple[int, int], ...]:
        if self.category != 4:
            return ()
        values = dict(self.properties)
        count = int(values["nbDrop"])
        if not 1 <= count <= 5:
            raise ValueError(f"Unsupported nominal drop count for support type {self.identifier}: {count}")
        return tuple((int(values[f"unitPosX{index}"]), int(values[f"unitPosY{index}"]))
                     for index in range(1, count + 1))

    def description(self) -> str:
        values = dict(self.properties)
        if self.category == 4:
            return (f"Nominal {len(self.offsets)} units of military type {values['unitIDDrop']}; "
                    f"landing offsets {self.offsets}. Runtime technology, interception and occupancy apply.")
        if self.category == 0:
            return (f"Base {values['damageType']} damage {values['damage']}; native area parameter "
                    f"{values['damageArea']} (not a verified radius). Aircraft damage reduces the effect.")
        if self.category == 5:
            return "Transport an AI unit at the source to the destination. Runtime placement rules apply."
        return "Preserve-only scheduled type: its scenario execution has not been verified."


def load_support_types(path: Path) -> dict[int, SupportType]:
    result = {}
    with path.open(newline="", encoding="utf-8-sig") as stream:
        for row in csv.DictReader(stream):
            item = SupportType.from_row(row)
            if item.identifier in result or item.identifier not in range(27) or item.category not in range(6):
                raise ValueError(f"Invalid/duplicate support definition: {item.identifier}")
            item.description()
            result[item.identifier] = item
    if set(result) != set(range(27)):
        raise ValueError("Support definitions require all 27 native types.")
    return result


@dataclass(frozen=True)
class SupportEvent:
    counter: int
    kind: int
    x: int
    y: int
    dest_x: int
    dest_y: int
    turn: int

    @property
    def editable(self) -> bool:
        return self.kind in EVENT_TYPES and 1 <= self.turn <= 99

    def row(self) -> dict:
        return dict(zip(SCRIPT_FIELDS["Automated Units"],
                        (self.counter, self.kind, self.x, self.y, self.dest_x, self.dest_y, self.turn)))


@dataclass(frozen=True)
class Objective:
    counter: int
    kind: int
    turn: int

    def row(self) -> dict:
        return dict(zip(SCRIPT_FIELDS["Objective"], (self.counter, self.kind, self.turn)))


@dataclass(frozen=True)
class IdTarget:
    counter: int
    identifier: int
    unit: int | None = None

    def row(self) -> dict:
        return ({"Counter": self.counter, "Id": self.identifier} if self.unit is None else
                {"Counter": self.counter, "Unit ID": self.unit, "Id": self.identifier})


@dataclass(frozen=True)
class PosTarget:
    counter: int
    x: int
    y: int
    unit: int | None = None

    def row(self) -> dict:
        row = {"Counter": self.counter, "PosX": self.x, "PosY": self.y}
        if self.unit is not None:
            row["Unit ID"] = self.unit
        return row


@dataclass(frozen=True)
class ScriptData:
    events: tuple[SupportEvent, ...] = ()
    objectives: tuple[Objective, ...] = ()
    objective_ids: tuple[IdTarget, ...] = ()
    objective_positions: tuple[PosTarget, ...] = ()
    order_ids: tuple[IdTarget, ...] = ()
    order_positions: tuple[PosTarget, ...] = ()

    @classmethod
    def from_rows(cls, rows: dict) -> ScriptData:
        result = cls(
            tuple(SupportEvent(*(int(row[field]) for field in SCRIPT_FIELDS["Automated Units"]))
                  for row in rows["Automated Units"]),
            tuple(Objective(*(int(row[field]) for field in SCRIPT_FIELDS["Objective"]))
                  for row in rows["Objective"]),
            tuple(IdTarget(int(row["Counter"]), int(row["Id"])) for row in rows["Objectives ID List"]),
            tuple(PosTarget(int(row["Counter"]), int(row["PosX"]), int(row["PosY"]))
                  for row in rows["Objectives Pos List"]),
            tuple(IdTarget(int(row["Counter"]), int(row["Id"]), int(row["Unit ID"]))
                  for row in rows["Unit Order ID List"]),
            tuple(PosTarget(int(row["Counter"]), int(row["PosX"]), int(row["PosY"]), int(row["Unit ID"]))
                  for row in rows["Unit Order Pos List"]),
        )
        for table, records in result.records().items():
            counters = [record.counter for record in records]
            if any(counter < 0 for counter in counters) or len(set(counters)) != len(counters):
                raise ValueError(f"Invalid or duplicate native script counter in {table}.")
        return result

    def records(self) -> dict:
        return dict(zip(SCRIPT_FIELDS, (self.events, self.objectives, self.objective_ids,
                                        self.objective_positions, self.order_ids, self.order_positions)))

    def rows(self) -> dict:
        return {table: [record.row() for record in records] for table, records in self.records().items()}

    def next_counter(self, attribute: str) -> int:
        return min((record.counter for record in getattr(self, attribute)), default=0) - 1 \
            if any(record.counter < 0 for record in getattr(self, attribute)) else -1

    def targets(self, attribute: str, values: tuple, unit: int | None = None) -> ScriptData:
        records = getattr(self, attribute)
        position = attribute.endswith("positions")
        pools = defaultdict(list)
        for record in records:
            if record.unit == unit:
                pools[(record.x, record.y) if position else record.identifier].append(record)
        counter = self.next_counter(attribute)
        incoming = []
        for value in values:
            if pools[value]:
                incoming.append(pools[value].pop(0))
            else:
                incoming.append(PosTarget(counter, *value, unit) if position else IdTarget(counter, value, unit))
                counter -= 1
        retained = {record.counter for record in incoming}
        existing = {record.counter for record in records}
        remaining = [record for record in records if record.unit != unit or record.counter in retained]
        remaining.extend(record for record in incoming if record.counter not in existing)
        return replace(self, **{attribute: tuple(remaining)})

    @property
    def references(self) -> set[int]:
        return ({record.identifier for record in (*self.objective_ids, *self.order_ids)}
                | {record.unit for record in (*self.order_ids, *self.order_positions)})

    def validate_change(self, previous: ScriptData, size: int, entities: set[int],
                        units: set[int], supports: dict[int, SupportType]):
        for table, records in self.records().items():
            old = {record.counter: record for record in previous.records()[table]}
            seen = set()
            keys = Counter()
            for record in records:
                if record.counter in seen or (record.counter >= 0 and record.counter not in old):
                    raise ValueError(f"Unknown/duplicate native script counter in {table}.")
                seen.add(record.counter)
                row = record.row()
                if any(type(value) is not int or not -2147483648 <= value <= 2147483647 for value in row.values()):
                    raise ValueError("Native script fields must be signed 32-bit integers.")
                key = tuple(value for name, value in row.items() if name != "Counter")
                keys[key] += 1
                if record == old.get(record.counter):
                    continue
                if isinstance(record, SupportEvent):
                    if not record.editable or record.kind not in supports:
                        raise ValueError("Support edits require a verified native type and arrival turn 1..99.")
                    if old.get(record.counter) is not None and not old[record.counter].editable:
                        raise ValueError("Unverified support events must be preserved.")
                    points = ((record.x, record.y), (record.dest_x, record.dest_y))
                    if record.kind not in (0, 7) and points[0] != points[1]:
                        raise ValueError("Non-transport support requires identical source and destination.")
                elif isinstance(record, Objective):
                    if (record.kind not in range(7) or len(self.objectives) != 1
                            or len(previous.objectives) != 1 or record.counter not in old):
                        raise ValueError("Only a single verified campaign objective can be edited.")
                    if old[record.counter].kind not in range(7):
                        raise ValueError("Deathmatch/internal objectives are preserve-only.")
                    if record.turn != old[record.counter].turn and (
                            record.kind != 1 or not 0 <= record.turn <= 99):
                        raise ValueError("Only Hold Position's raw threshold is editable (0..99).")
                    points = ()
                else:
                    if record.unit is not None and record.unit not in units:
                        raise ValueError("An AI target row must belong to a starting unit.")
                    if isinstance(record, IdTarget):
                        if record.identifier not in entities:
                            raise ValueError(f"Target ID {record.identifier} is not a unit or building in this map.")
                        points = ()
                    else:
                        points = ((record.x, record.y),)
                if any(not 0 <= x < size or not 0 <= y < size for x, y in points):
                    raise ValueError("Script coordinates must be inside the map.")
            for record in records:
                key = tuple(value for name, value in record.row().items() if name != "Counter")
                if record != old.get(record.counter) and keys[key] > 1 and not isinstance(record, SupportEvent):
                    raise ValueError("New target rows cannot duplicate another target.")
            if table == "Automated Units":
                if any(not record.editable and record.counter not in seen for record in previous.events):
                    raise ValueError("Unverified support events must be preserved.")
            if table == "Objective" and len(records) != len(old):
                raise ValueError("Objective records cannot be added or deleted.")
        if (len(previous.objectives) != 1 or previous.objectives[0].kind not in range(7)) and (
                self.objective_ids != previous.objective_ids
                or self.objective_positions != previous.objective_positions):
            raise ValueError("Targets of deathmatch/internal objectives are preserve-only.")


def diagnostics(model) -> list[str]:
    data = model.scripts
    if data is None:
        return ["This document has no native script snapshot."]
    result = []
    limit = model.scenario.turns if model.scenario else 100
    entities = model.units.keys() | model.buildings.keys()
    if len(data.objectives) != 1:
        result.append(f"Objective record count {len(data.objectives)}: preserve-only.")
    elif data.objectives[0].kind not in range(7):
        result.append("Deathmatch/internal objective: preserve-only.")
    for event in data.events:
        if not event.editable:
            result.append(f"Support row {event.counter}: unverified type/turn, preserve-only.")
        if event.turn > limit:
            result.append(f"Support row {event.counter}: turn {event.turn} is beyond the mission limit {limit}.")
        support = model.support_types.get(event.kind)
        if support and support.category == 4:
            unit_type = model.unit_types[int(dict(support.properties)["unitIDDrop"]), 0]
            for dx, dy in support.offsets:
                key = event.x + dx, event.y + dy
                footprint = [(key[0] + x, key[1] + y) for x in range(unit_type.width)
                             for y in range(unit_type.height)]
                if any(point not in model.cells or point in model.blocked or model.building_at(point)
                       for point in footprint):
                    result.append(f"Support row {event.counter}: nominal landing footprint near {key} is "
                                  "outside/reserved/occupied in the editor; "
                                  "future occupancy and interception can change the payload.")
                    break
    for table, records in data.records().items():
        if "List" not in table:
            continue
        seen = set()
        duplicates = 0
        for record in records:
            value = (record.unit, record.identifier) if isinstance(record, IdTarget) else (
                record.unit, record.x, record.y)
            if value in seen:
                duplicates += 1
            seen.add(value)
            if record.unit is not None and (
                    record.unit not in model.units or model.units[record.unit].layer != "units"):
                result.append(f"{table} row {record.counter}: absent starting-unit owner {record.unit}.")
            if isinstance(record, IdTarget):
                if record.identifier not in entities:
                    result.append(f"{table} row {record.counter}: target {record.identifier} is absent.")
                elif record.identifier in model.units and model.units[record.identifier].layer == "reinforcements":
                    result.append(f"{table} row {record.counter}: target {record.identifier} arrives later; "
                                  "the order/objective can evaluate before it exists.")
            elif (record.x, record.y) not in model.cells:
                result.append(f"{table} row {record.counter}: tile is outside the map.")
        if duplicates:
            result.append(f"{table}: {duplicates} duplicate target rows are preserved.")
    for unit in model.units.values():
        if unit.layer == "reinforcements":
            if unit.arrival > limit:
                result.append(f"Reinforcement {unit.identifier}: arrival is beyond the mission limit.")
            continue
        if unit.order_id not in ORDERS:
            result.append(f"Unit {unit.identifier}: unverified order {unit.order_id}, preserve-only.")
        if unit.order_id:
            targets = data.order_positions if unit.order_id in (1, 2, 5) else data.order_ids
            if not any(record.unit == unit.identifier for record in targets):
                result.append(f"Unit {unit.identifier}: authored order has no targets.")
            if unit.owner != 0:
                result.append(f"Unit {unit.identifier}: AI order on a non-AI owner; execution is not guaranteed.")
            if unit.probability < 100:
                result.append(f"Unit {unit.identifier}: retention {unit.probability} can clear its order "
                              "(inclusive native roll; 0 is not disabled).")
    return result


def timeline(model) -> list[tuple[int, str, str, str]]:
    result = []
    if model.scripts is None:
        return result
    for event in model.scripts.events:
        definition = model.support_types.get(event.kind)
        name = definition.label if definition else f"Unknown support {event.kind}"
        result.append((event.turn, "Support request", str(event.counter),
                       f"{name}: ({event.x},{event.y}) -> ({event.dest_x},{event.dest_y}); exact turn, no retry"))
    for unit in model.units.values():
        if unit.layer == "reinforcements":
            derived = reinforcement_order(model.scripts, unit.arrival_order)
            detail = ("; objective-derived order unavailable (ambiguous objective)" if derived is None else
                      f"; objective-derived {ORDERS[derived[0]]}; expiry 99, retention 100"
                      if unit.arrival_order else "")
            result.append((unit.arrival, "Reinforcement", str(unit.identifier),
                           f"Type {unit.kind} at ({unit.x},{unit.y}); blocked arrivals retry"
                           + detail))
        else:
            result.append((0, "Initial unit", str(unit.identifier),
                           f"Type {unit.kind} at ({unit.x},{unit.y}); owner {unit.owner}"))
            if unit.order_id:
                result.append((unit.order_turn, "Order active through", str(unit.identifier),
                               f"{ORDERS.get(unit.order_id, f'Unknown {unit.order_id}')}; retention {unit.probability}"))
    for goal in model.scripts.objectives:
        turn = goal.turn if goal.kind == 1 else model.scenario.turns if model.scenario else 100
        timing = "Raw hold threshold" if goal.kind == 1 else "Global mission limit"
        result.append((turn, timing, str(goal.counter), OBJECTIVES.get(goal.kind, f"Unknown {goal.kind}")))
    return sorted(result)
