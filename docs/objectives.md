# Skirmish objectives, reinforcements and the battle report

How a battle ends, recovered from `LIBDAY.EXE`. The code is
`libday-data::objective` (the predicates), `libday-data::battle` (statistics
and rank) and `openlibday::skirmish` (timing, reinforcements, report).

## Data

| Table | Use |
|---|---|
| `Objective` | `ObjectifId` (the kind, below) and `TurnNumber` (only Hold uses it) |
| `Objectives ID List` | unit / building ids the objective names |
| `Objectives Pos List` | the objective tiles |
| `Scenario Info` | `Turns`: the turn limit (0 or less: none in openlibday) |
| `Reinforcement` | units entering later: `Turn`, position, and whether they take an objective order |

A scenario without an `Objective` row only ends on the turn limit or a cheat.

## Turn counter

The original advances its turn counter when the AI's phase begins;
openlibday advances `clock.turn` after it. The objective turn is therefore
`clock.turn` during the player's phase and `clock.turn + 1` during the AI's.
Objectives are judged whenever nothing is animating (every frame, and after
every AI action), only while the battle is on.

## Predicates (`0x43c067`..`0x43c987`)

| Kind | Name | Won | Lost |
|---|---|---|---|
| 0 | Destroy | every listed id destroyed and no road or mine left on the listed tiles | turn limit |
| 1 | Hold | a non-AI unit on the tiles for more than `TurnNumber` turns (one turn's absence is forgiven, two reset the count) | too few turns left to make it |
| 2 | Escort | a listed unit stands on the tiles | every listed unit destroyed |
| 3 | Protect | the turn limit passes | an AI unit on the tiles on the last turn |
| 4 | Intercept | every listed unit destroyed | a listed unit reaches the tiles |
| 5 | Keep | the turn limit passes | a listed id destroyed |
| 6 | Destroy all except | every other AI unit destroyed | a listed id destroyed |

After the predicate (`0x43c678`) a forced outcome (the original's win/lose
cheats; the `win` / `lose` script commands) wins, then the turn limit: past
`Turns` without an outcome is a defeat. Losing every unit is not in itself a
defeat.

## Reinforcements (`0x416576`, placed by `0x4405d4`)

After the AI's phase, every reinforcement whose `Turn` has come enters on its
own tile, provided the tile is on the map and no unit or building overlaps it
and its id is unused; otherwise it waits for a later turn. It enters for the
AI at tech 0 with full armour and action points, facing the map centre. A
reinforcement flagged to follow the objective gets an order (expiry 99,
probability 100) replacing its targets:

| Objective | Order | Targets |
|---|---|---|
| Destroy, no ids | 5 Patrol | objective tiles |
| Destroy with ids, Intercept | 3 Escort | objective ids |
| Hold, Protect | 5 Patrol | objective tiles |
| Escort, Keep | 6 Attack | objective ids |
| Destroy all except | none | |

Arrivals have no action points (`0x434179`). The arrival is announced as
string 896 "%1 reinforcements have arrived" for 2 s with the `reinf` sound
(`0x4542c5`).

## Player reinforcements

Observed in the original (Crime and Punishment) and `0x4363f0`:

- Credits (player record +8) are set to the scenario's `ReinfRes` when the
  battle starts (leftover deployment points are lost) and grow by
  `ReinfRes` at every turn; unspent credits carry over. No difficulty or
  building modifier was found for skirmishes.
- REINFORCEMENT, during the player's turn, opens the deployment sidebar and
  start zone with no BACK. A unit costs `(int)(Cost * (turn * 0.2f + 1))`
  (Trooper, cost 5: 6 on turn 1, 7 on turn 2).
- Units placed now can be recalled for the same price; units already in the
  battle cannot (their tile is occupied).
- CONTINUE (no confirmation, `Escape` too) returns to the battle and the
  selection; the new units have 0 AP until the next turn.

## Statistics and rank

Each destroyed unit or building counts as lost for its owner and destroyed
for the attacker (the shooter, or a mine's owner) when they differ. The
rank (`0x415746`) sums, over unit types at tech 0, `(2 * destroyed - lost) *
cost`, plus over building types `(destroyed - lost) * cost`.

## Battle report (`0x45ded0`)

Frame 365 of `1024X768.SMK` (448x480) at (44, 1), CONTINUE (frames 366-369)
at (300, 419) in it. Strings from `LIBDAYEN.DLL`: title `0x201`, outcome
`0x207` (won) / `0x208` (lost), headings `0xb4`, `0x200`, `0x203`, `0x202`,
counts `0x204` (`%1!d! Units`) and `0x205` (`%1!d! Buildings`). The title and
outcome are centred; the headings sit at y 185 with columns at x 25, 115,
250 and 360 of the text area (inset 20); the human player's row follows two
lines below with buildings on the next line. Only the human player is
listed. CONTINUE, Return or Escape returns to the skirmish selector.
