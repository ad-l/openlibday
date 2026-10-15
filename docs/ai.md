# The computer player (AI)

Reverse-engineered from `LIBDAY.EXE` (classes `CAIbrain`, `CAIaction*`,
code `0x46cd0f`–`0x4712e2`). Addresses are virtual addresses in the
original executable. The AI always plays as **player 0**.

Tags: **[quirk]** marks original behaviour that looks like a bug but is
reproduced. **[?]** marks a detail that is inferred rather than read in full.

## 1. Overview

An AI turn is a fixed sequence of **stages** run by the brain
(`CAIbrain::run`, `0x46fcbd`). Each stage either:

- builds a list of **actions**, one per unit, or
- executes the actions of such a list.

An action is a small state machine (`execute` is vtable slot 5). The brain
calls `execute` repeatedly, one step per call, until it reports that it is
done. Then the brain moves to the next action.

The brain works in time slices: it returns to the game loop every 4000 ms
and resumes where it left off. Moves and shots are ordinary game orders, so
they animate and can trigger reaction fire and mines exactly as the human
player's do.

| Stage | Function | What it does |
|---|---|---|
| 0 | `0x470b55`, `0x470d71` | Roll each order's **Probability**, expire orders past their **Turn**, then build the authored-order actions (list `+0xfc`). |
| 1 | — | Nothing. |
| 2 | `0x470658` | Dynamic support purchase (campaign only). |
| 3 | `0x46ff88` | Early-turn drop purchase against massed light units. |
| 4 | `0x470b99` | Build a **shoot_overwatch** action for every armed unit (list `+0xc4`). |
| 5 | | Execute list `+0xc4`: fire from the current tile, before anything moves. |
| 6 | `0x46feb0` | Launch authored automated units (supports) arriving this turn. |
| 7 | `0x470658` | Dynamic support purchase again (campaign only). |
| 8 | `0x470cc6` | Build a **base action** for every mobile unit without an order (list `+0xe0`). |
| 9 | `0x46ff2b` | Execute list `+0xe0`. Skipped when there is no hostile unit. |
| 10 | `0x470353(1)` | Execute **goto** actions, then move them all together, keeping a shot in reserve. |
| 11 | `0x470353(2)` | Execute **goto fast** actions, then move them all together, spending all AP. |
| 12 | `0x4702f6` | Execute the remaining authored orders: escort, defend, patrol, attack. |
| 13 | | End of turn. |

### 1.1 Priority between scripts and default behaviour

Each unit gets **one** action per turn, chosen by its order:

- order 0 → base action (stage 8);
- orders 1–6 → their authored action (stage 0).

Stage 4 is the exception. It also gives *every* armed AI unit a
fire-from-where-you-stand action, whether or not it has an order.

The execution order within a turn is:

1. **Opportunistic fire** (stage 5) by all units: static units (towers)
   first, then paralysers, then jammers by increasing range.
2. Automated units arriving this turn (stage 6).
3. **Unordered units** (stage 9): the base action plans a target, moves and
   fires.
4. **goto** (stage 10).
5. **goto fast** (stage 11).
6. **escort → defend → patrol → attack** (stage 12, order id ascending).

Ties within one order id keep the scenario's unit-list order.

Because opportunistic fire runs first, a scripted unit that can already hit
something will usually spend its AP on fire. When it has fewer than 5 AP
left, it gets no scripted action that turn.

### 1.2 Order lifetime (`Turn`, `Probability`)

The unit fields come from `Units On Map` and `Reinforcement`: `OrderID`,
`Turn`, `Probability`, `EnemyUnitType`, plus `Unit Order ID List` and
`Unit Order Pos List`.

- **Probability** (`0x470b55`, stage 0, every turn): for each AI unit with an
  order, roll `rand() % 100`. If the roll is greater than `Probability`, set
  the order to 0. This is permanent: the unit becomes a normal AI unit.
  - With `Probability` 100, the order is never dropped.
  - With `Probability` 90, each turn there is a 9% chance (rolls 91–99) of
    dropping it.
- **Turn** (`0x470d71`): an order whose `Turn` is less than the current turn
  is set to 0. So `Turn` is the **last turn** on which the order is followed.
- An action is built only for a **mobile** unit (Movement not 0 or 8) with
  **AP > 4**.
- Order 7 is accepted by the order setter but has no action (the original
  would crash). We ignore it.
- Orders are also cleared when they are completed:
  - goto: the last waypoint is reached;
  - attack, defend, escort: the target ID list is empty.

  Patrol never completes; only `Turn` or `Probability` ends it.
- A cleared order makes the unit an ordinary AI unit from the next turn on.

`EnemyUnitType` doubles the scoring priority of matching target units in
the base target evaluation (§3.4). That evaluation is used by base, attack,
escort (once in its zone), patrol and shoot_overwatch.

### 1.3 Hostility and unit sets

From the AI's point of view (`0x4346b5(0, unit)`), a unit is **hostile** if:

- its owner is not 0; or
- it is owned by player 0, has `Race` 0, and its type's `isConvoy` differs
  from `isAiTarget`.

The iterators used by the stages are:

| Iterator | Set |
|---|---|
| `0x434813` | Hostile units. |
| `0x434840` | AI units with `Race` ≠ 0 and Movement not in {0, 8} ("mobile"). |
| `0x43487f` | AI units with Movement 0 (NoneGround) or 8 (NoneWater) ("static": towers and similar). |
| `0x4348b1` | Mobile AI units with order 0. |
| `0x4348fb` | Mobile AI units with an order. |

All of them iterate the unit list in scenario order.

## 2. Common mechanics

### 2.1 Action bookkeeping

Every action has:

- an execute **counter**. Each call increments it, and on the 11th call the
  action reports done whatever its state;
- the unit **id**, looked up on every call. If the unit is gone, the action
  is done;
- a **state**.

So an action makes at most about three plan → move → fire cycles per turn.

**AP** means remaining action points, never negative. Most actions stop
immediately when `AP < 4`.

### 2.2 Target record and scores

The brain owns two target records: **best** and **candidate**. A record
holds:

- a kind: unit, building, wall, mine or none;
- the target object;
- a position;
- an integer score;
- a path.

A reset record has kind none and score **−65535** (global `0x4f4330`),
which means "no target".

A record is **valid** while the same object is still at its position. A
destroyed target makes the record invalid.

**Keep-best** (`0x46d09e`): a candidate replaces the best record when:

- it scores at least 1 point more; or
- it is a near-tie (best is equal to the candidate or 1 more) **and** the
  current best is not strictly closer to the reference point on either
  axis.

The reference point is the attacker's tile. When both records have the same
position, it is the current target's position instead.

Ties therefore favour the **later** candidate, so iteration order matters.

```
closer(p, q, r) = q == (-1,-1) or |p.x-r.x| < |q.x-r.x| or |p.y-r.y| < |q.y-r.y|
```

### 2.3 Pathfinding (`0x42d82e`)

The pathfinder builds a distance field from the unit's tile. The start cell
gets a large value. Moving into a cell subtracts the direction cost and the
cell's terrain cost, so a larger value means more AP left.

- **Windowed** flood (`limited`):
  - The search is limited to the unit's tile inflated by 20, clipped to the
    inner map, unless the unit is inside the central 12×12 of the map.
  - At most 7 relaxation sweeps.
  - When the map is wider than 56, one `rand()` is consumed and discarded.
  - Boats always use the full flood.
- **Full** flood: used by goto.

To build a path, the pathfinder backtracks from the destination to the
neighbour with the highest value. Neighbours are scanned in the order S, SW,
W, NW, N, NE, E, SE, i.e. offsets `(0,1) (-1,1) (-1,0) (-1,-1) (0,-1)
(1,-1) (1,0) (1,1)`. Equal values are broken with `closer`.

Paths are a list of steps. Each step has a tile, a cost and a "can stop
here" flag.

Teleporters (Movement 7) get a single-step path facing `rand() % 8`.

**AP truncation** (`0x42d73c`): walk the steps subtracting each cost. When
AP goes negative, cut **after the last stoppable step**. If the step before
was not stoppable, it keeps walking. **[quirk]** This can overshoot the AP
budget.

openlibday's `libday-sim::path` plays the role of this pathfinder. Its
movement costs and stoppable tiles are the ones the player's moves use.

### 2.4 Fire check (`0x435ca1`, `0x435be0`)

This is the same check the player's fire orders use (`libday-sim::fire::check`):

- target on the map and outside the shooter's own footprint;
- the shooter has a weapon;
- AP ≥ `actionPointsShoot`;
- for each tile of the target's footprint: Chebyshev distance d with
  `min_range < d <= range`, and clear line of sight. Line of sight is
  skipped for indirect weapons (WeaponType 1).

### 2.5 Firing (`0x46d268` → `0x416bb8(unit, pos, repeat = 1)`)

Firing first requires the record to be valid and the fire check to pass.

It then **repeat-fires** until AP < `actionPointsShoot` or the target is
gone. For paralysers, it stops when the target unit has 0 AP.

These are normal shots, with scatter, damage, area damage and destruction.
This is the same behaviour as the player's lock-and-fire.

## 3. Base action — unordered units (`CAIaction`, execute `0x46d172`)

### 3.1 State machine

| State | Behaviour |
|---|---|
| 0 **plan** | Run §3.2. If no target was found: set state 3. If `counterAttack` is set, the unit may call a transport (§3.7); otherwise it walks towards the map centre (§3.6). |
| 1 **move** | Run §3.5. |
| 2 **fire** | Fire at **best** (§2.5). If the target was destroyed, go back to state 0 (re-plan). If it survived, the action is done. |
| 3 | Done. |

`counterAttack` is brain flag `+0x18`. It is only ever set by campaign
counter-attacks.

### 3.2 Plan (`0x46dcc6`)

```
best.reset()
if AP < 4: done
flood (windowed) from the unit
for e in hostile units (list order):            evaluate unit target e        (§3.3)
if counterAttack and DamageType == 0:
    for b in buildings with owner != 0:         evaluate building b
for m in mines revealed to player 0:            evaluate mine m
if best is none or (best.path.len > 25 and best.score < 0):
    best.reset()
    for w in walls:                             evaluate wall w
```

The path length counts the origin tile.

### 3.3 Candidate firing tiles

For each candidate tile T, the tile is kept only when all of these hold:

- T is on the map;
- T is reachable, and the path is built;
- the fire check from T against the target passes. The check uses the
  unit's **current** AP.

Each surviving tile is scored (§3.4) and offered to keep-best.

| Target | Tiles (in order) | Factor |
|---|---|---|
| Unit | Rings at Chebyshev distance `R`, `m` and `(R+m)/2` around the footprint, where `R` = WeaponRange and `m` = max(1, MinRange). The last two rings only when `R > 1`. | 500 |
| Building | `(l-1,t-1)`, `(r+1,b+1)`, `(t+1,r-1)`, `(b-1,l+1)`, where (l, t, r, b) is the footprint rectangle (exclusive r, b). **[quirk]** The last two swap x and y. | 1000 |
| Wall | The 3×3 square centred on the wall, x outer, y inner. | 5 |
| Mine | `x in mx-1..=mx`, `y in my-1..=my` (2×2). **[quirk]** | 500 |

Ring order for a rectangle (l, t, r, b), exclusive, duplicates included:

1. Top edge left→right.
2. Right edge top→bottom.
3. Bottom edge right→left.
4. Left edge bottom→top.

### 3.4 Score (`0x46ce87`)

```
shots  = AP / actionPointsShoot                    (integer division)
dmg    = shots * (DamageBase + DamageRandom / 2)   (integer average)
kill   = min(100, 100 * dmg / targetArmour)        (armour 0 → 100)
pen    = min(500, 10 * path.len)
base   = kill * 0.01 * factor - pen
unit target and kill > 99: base += target Cost
p      = AIPriority[attacker.AIcategory][targetCategory]
unit target:   p *= 2 if target type == attacker's EnemyUnitType
               p = 0  if attacker paralyses and target cannot fire now
building, wall, mine: p = 0 if attacker paralyses
score  = trunc(p * 0.01 * base)
```

- The target category is:
  - for units, the target's `AIcategory`;
  - for buildings, the building's `AIcategory`;
  - 7 for walls;
  - 8 for mines.
- `AIPriority` is the `AI Priority` table in `GLOBAL.MDB`: `AiSource` ×
  `AiTarget`, 12 categories (`AI Category Lookup`). Missing cells are 0.
- "Can fire now" means: has a weapon and AP ≥ `actionPointsShoot`.

### 3.5 Move (`0x46e86c`)

```
cut best.path to AP (§2.3)
if empty or target already hittable from here: → fire
first of:
  a. first path step that can hit the target, is stoppable and leaves
     AP > actionPointsShoot: stop there
  b. (unit target that can fire now) the last stoppable step before the
     first step the target could hit, unless the unit is already exposed
  c. (canOverwatch) stop on the first stoppable step that leaves
     AP < actionPointsShoot  [quirk: on, not before]; or the whole path if
     it keeps the reserve
  d. the whole AP-cut path
after the move: at destination → fire; interrupted (mine, reaction fire)
→ re-plan
```

The base action never sets overwatch.

### 3.6 No target: walk to the centre (`0x46d5dc`)

The unit tries the tiles of the 12×12 square at
`((w-12)/2, (w-12)/2)` in order, x outer and y inner. **[quirk]** The
square uses the map width for both axes. It moves along the first path it
finds, cut to AP.

Because the flood is windowed, this only works within about 20 tiles of
the centre.

### 3.7 No target in a counter-attack: transport (`0x46d3e0`)

This applies only in the campaign, with `counterAttack` set and AP > 10,
and not for boats or submarines. The unit buys support type 7 if the
support budget allows. It is airlifted to a random placeable tile within 10
of the first hostile unit that has one.

## 4. shoot_overwatch — opportunistic fire (`0x46f11e`)

Built in stage 4 for:

- every mobile AI unit with **AP > 4**; then
- every static AI unit (towers).

The list is ordered by insertion sort with comparator `0x471030`.

**Insertion sort** (`0x4711ae`): insert each new element before the first
placed element `e` with `less(new, e)`, otherwise append.

`less(a, b)` (`0x471030`):

1. a static → true.
2. b static → false.
3. a paralyses → true.
4. b paralyses → false.
5. a has no jammer (`Jammer <= 0`) → false.
6. Otherwise `a.range < b.range`.

Execution:

```
0: best.reset(); for e in hostile units passing the fire check from the
   unit's tile: score e (§3.4, path length 0) with factor 1000 if e can fire
   now else 500; keep best. None → done.
1: (no-op step)
2: fire (§2.5). Target destroyed → state 0 (pick another); survived → done.
```

The unit never moves. It keeps firing while targets die and AP lasts. AP is
only kept for the human turn when nothing was in range.

## 5. Authored orders

Attack, defend and escort use only the **first** id in the order's ID list.
It is looked up as a unit first, then as a building.

When the target is gone, the id is popped, one per execute call. When the
list is empty the order is cleared (`0x46d2d5`) and the action ends.

### 5.1 goto (1) and goto fast (2) (`0x46e427`, `0x46e641`)

The two are identical except for the stage that runs them and the AP
reserve.

```
plan: no waypoints → clear order, done
      AP < 4       → done
      unit on head waypoint → pop it, done (no move this turn)
      else queue a move to the head waypoint
```

After all goto (or goto fast) actions have planned, the queued moves are
flushed together (`0x47142f`), in queue order:

1. Choose the final tile.
   - If the head waypoint is stoppable and not claimed by an earlier mover,
     use it.
   - Otherwise use the nearest free stoppable 8-neighbour, in the order
     (x-1,y), (x,y-1), (x,y+1), (x+1,y), (x-1,y-1), (x-1,y+1), (x+1,y-1),
     (x+1,y+1).
2. Build the path with a full flood. Tiles claimed by earlier movers are
   blocked; tiles they vacate are free.
3. Cut the path to the AP budget.
   - goto: AP minus `actionPointsShoot` for types that can overwatch.
   - goto fast: all AP.
4. Execute the moves one after another as normal moves (reaction fire,
   mines).
5. For each mover: if it ends within 1 tile (Chebyshev) of its head
   waypoint, pop the waypoint. With no waypoints left, clear the order.

Waypoints are thus visited in sequence, one leg per turn. goto never fires
by itself; it relies on stage 5 and reaction fire.

### 5.2 Patrol (5) (`0x46ee2e`)

Patrol has no waypoint index. Each turn:

- **Not on a waypoint:**
  1. Walk towards the waypoint with the shortest path. The path length is
     counted in steps; ties keep the earlier waypoint. The flood is
     windowed.
  2. Then re-plan in the same turn.
- **On a waypoint:**
  1. Evaluate hostile units as the base action does (§3.3).
  2. Cut the attack path at the first stoppable step that is not a
     waypoint, so the unit strays at most one tile off the route.
  3. Move, then fire.

The patrol never pops waypoints or clears its order; it guards the route.

### 5.3 Attack (6) (`0x46d6c2`)

```
0: no targets or AP < 4 → done. Evaluate the first target (unit: §3.3
   rings; building: §3.3 corners). Target missing → pop id, re-plan.
   Path found → 1, else done.
1: move (§3.5)
2: fire (§2.5); destroyed → 0 (next id); survived → done
```

### 5.4 Defend (4) and escort (3) (`0x46d897`, `0x46dad6`)

```
zone = target footprint inflated by N (defend 2, escort 3), then right+1,
       bottom+1   [quirk: asymmetric]
unit inside zone:
    defend: done (does nothing)
    escort: behave as the base action (§3) for the rest of the turn
otherwise: path to the target's nearest reachable 8-neighbour (fewest
    steps, scan order of §2.3), stop at the first stoppable step inside the
    zone, cut to AP, done.
```

Defend never fires by itself. Its fire comes from stage 5, and from
reaction fire during the human turn.

## 6. Supports bought by the AI

The AI can buy supports in three ways:

- **Authored automated units** (stage 6): every `Automated Units` row with
  arrival turn equal to the current turn is launched from (StartX, StartY)
  to (DestX, DestY).
- **Stage 3** (all modes, including skirmish):
  - Runs on turns ≤ 4, and only if `rand() % 100 <= 66`.
  - Let N be the number of hostile units. For each of the types H_Trooper,
    H_Grenadier, H_Assault Trooper, H_LAV and H_Light_Tank, compute that
    type's share in percent.
  - If `N > 30` and a type's share is over 60%, buy Shubblar Pyro-Maniac
    drops (Drop-category support entry 19): 3, 2 or 1 drops when the AI's
    money exceeds 1200, 700 or 300.
  - Each drop targets the first hostile unit with armour ≤ 30 that is a
    trooper or vehicle, starting from a random index.
- **Dynamic support** (stages 2 and 7, campaign only):
  - Chance: campaign `+0x90`, or 25 in a counter-attack.
  - Picks a random available support: mine detector, medic, fighters/Gene
    Bomb, or drops.
  - Each support type has its own target chooser. The support's cost comes
    out of the AI's support budget.

## 7. openlibday implementation notes

- `crates/openlibday/src/ai.rs` holds the brain state.
- Each frame of the AI phase, it advances one step, but only while no move
  or shot animation is playing. Moves and shots go through the same code
  paths as the player's, so walking, scatter, damage, mines and reaction fire
  all behave identically.
- The C runtime `rand()` stream is not reproduced bit-for-bit. The AI uses
  the game's random source in the same places: probability rolls, stage 3,
  the shuffle, and the teleport facing.
- Supports (stages 2, 3, 6 and 7) depend on automated units, which are not
  implemented yet. They are documented here but left out for now.
- Not reproduced: the 4000 ms time slices.
