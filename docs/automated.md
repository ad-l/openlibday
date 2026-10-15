# Automated units (off-map supports)

Automated units are aircraft and drops called in from off the map: a
bomber, a transport that lifts a unit and sets it down elsewhere, a mine
detector, paratroopers and so on. They were recovered from `LIBDAY.EXE`
(addresses below are in the 1080p build) and the GLOBAL database tables
`Automated Units`, `Automated Units Infos` and `Automated Units Properties`.

The rules live in `libday-view::support` (pure, unit tested). The sidebar,
targeting and animation are in `openlibday/src/automated.rs`.

## Data

| Category | Types | Effect |
|---|---|---|
| 0 Destroy | Bomber, Gas, Nuke, Fighters, Gene Bomb | `damage` + 1 on everything within `damageArea`, friend and foe, falling off as a weapon's. `damageType` `AR` destroys (armour); anything else (Gas: `AP`, 300 over 3x3) paralyses: action points are drained and the excess becomes `negAP`, withheld from the unit's next turn, buildings and walls unharmed (`0x4723eb`, string compare at `0x47246a`, then `0x418542`) |
| 1 Repair | Armor, Medic, Movement | `armorInc` / `actionInc` restored to the player's units, falling off with Chebyshev distance, capped at the maximum |
| 2 Build | Bridge | one bridge section (road type 1) on a bridge site, see [Bridges](#bridges) |
| 3 Mine | Mine | reveals hidden mines within `mineDetectionRange` |
| 4 Drop | Footmen, Troopers, ... | `nbDrop` units of `unitIDDrop` at the `unitPos` offsets with the given facings, full AP |
| 5 Transport | Transport | two clicks: the unit to lift, then where to set it down |

Each type has a flat cost, a race, an SMK prefix (`HBMB`, `HHTR`...), a
cursor, an explanation shown in the status bar, the anti-aircraft position
`AAPos` in SMK pixels, and the frames at which the flak starts (`FrameAA`)
and the effect applies (`FrameAction`). `DeployOnce` types can be used once
per battle. Transports have a second animation with its own frames and
`SecondExplain` ("Click on destination").

## Player

- **AUTOMATED UNITS** (player's turn, nothing moving) swaps the sidebar for
  the support list (`1024X768.SMK` frame 507, the same layout as deployment):
  portrait, name and price. Rows the credits do not cover are greyed, and
  clicking one says "Need more tactical resource".
- Clicking a row arms the support and shows its explanation. A map click
  checks the target ("Invalid target" otherwise), deducts the price from the
  same credits as reinforcements, and plays the animation.
- There is no action point cost and no per-turn limit.
- MAP, GOAL, the arrow on AUTOMATED UNITS and Escape leave the list.

## Anti-aircraft

When the aircraft arrives (`0x41bc04`), the hostile units that can fire at it
are listed:
- units on the map (not waiting to arrive) that are anti-aircraft and can
  overwatch;
- with the action points to shoot;
- with the target within their range.

They fire, strongest first (`damage base + random / 2`, then shoot AP). Each
adds its flat `DamageAA` until the aircraft's armour is gone. The animation
variant follows: intact, damaged, or shot down. The survival fraction scales
the effect:
- a damaged drop lands half its units;
- a damaged mine detector has a shorter range;
- a downed aircraft does nothing.

Each shooter's impact animation is drawn at `AAPos`, staggered by 3 frames.

## Drops and transports

- A drop onto a mine sets it off.
- A spot where the unit cannot stand loses that unit.
- A transport's lifted unit leaves the map between the two legs and arrives
  with no action points left.

## AI

A scenario's `Automated Units` table lists supports the AI launches for free
on a given turn, with a start tile (and destination for a transport). Stage
6 of the AI turn (`0x46feb0`) launches them one at a time before the AI moves
its units. Supports that are Human and not Bomber, Gas or Nuke are skipped,
as in the original (`0x470acf`).

## Bridges

Bridges are not in the scenario data: the road layer computes the bridge
sites from the terrain when the map loads (`0x42fb2d`):

1. Every riverbank tile with plain to its west and water (deep water or
   riverbank) to its east starts an east-going span. Failing that, plain to
   its north and water to its south starts a south-going span. Rows and
   columns run from 0 to size − 2 (`0x42fbb1`).
2. The span crosses deep water to the next riverbank, which must have plain
   beyond it. Otherwise the span is dropped (`0x42fe1d`).
3. A span is dropped when its start and its end are each next to those of
   an earlier span (same row or column ±1). Wide fords therefore keep every
   other crossing (`0x42ff6e`).
4. A span sharing a tile with an earlier one is dropped. The rest contribute
   their tiles to the site list (`0x43002d`).

The pre-placed bridges of the original maps all lie on these sites.

A section may be built on a site tile (`0x42f5dd`) when the tile is free:
no unit, building, wall or road. A riverbank end is always buildable, but
deep water needs a road on an orthogonal neighbour. Bridges therefore grow
one tile at a time from either bank.

While Bridge is armed, the sites without a road show `BRIDGE` (red cross,
buildable now) or `BRIDG001` (girders) from the symbol sheet (`0x4722b3`).
Building lays road type 1.
## Approximations

- **Availability.** The original offers the supports whose prerequisite
  building the player owns. We use the scenario's own list (`setup.automated`),
  which matches the skirmishes. Campaigns will need the building rule.
- **Bridge.** The bridge is placed as road kind 1 on water or a crack. The
  exact tile choice was not traced.
- **AI purchases.** AI stages 2 and 7 (buying supports with the campaign's
  AI budget) are not implemented. They belong to the campaign work in
  [campaigns.md](campaigns.md).
- **Cursors.** The support cursors come from the `cursorID` names in the
  symbol sheet and have not been checked against the original.

## Testing

[scripts/automated.txt](../scripts/automated.txt) plays a transport, a mine
detector, a gas bomb and a bomber on *Opportunity Knocks*.
[scripts/bridge.txt](../scripts/bridge.txt) builds two sections of the
unfinished bridge on *Recycle This!* and checks `automated.bridge_sites`
(`[x, y, buildable]` in the state).
[scripts/scripted_support.txt](../scripts/scripted_support.txt) checks the
scenario's scripted Footmen drop on *Opportunity Knocks* turn 5
(`automated.scripted` lists the pending ones). The `grant ID` and
`credits N` script cheats offer any support and set the credits.
