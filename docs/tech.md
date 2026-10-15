# Technology buildings and capture

Buildings whose GLOBAL `Building Types` row has a positive `TechReward`
(type `+0x3c`) hold technology:

| Type | Name | TechReward | Size |
|---|---|---|---|
| 235 | Tech Center | 1000 | 3x3 |
| 236 | Tech Center | 500 | 3x2 |
| 237 | Tech Center | 250 | 2x2 |
| 1011 | Aztek Ruin | 6000 | 5x5 |
| 1014 | Pyramid | 6000 | 4x4 |
| 1015 | Aztek face | 500 | 2x2 |
| 1016 | Aztek face | 250 | 1x1 |

The `Ruin Types` table has no technology column: "ruins with tech" are the
Aztek building types above.

## Capture (`0x41ae03`)

The server keeps a list of captured buildings (`[esi+0x48]`). On each pass
it first drops buildings no longer on the map, then for every technology
building takes the tiles ringing its footprint (`0x48af9f`: the rectangle
grown by one, less the footprint, on-map tiles only). If any of them holds
a unit of a human player (owner `+0x1c != 0`) the building is captured,
otherwise it is released; each change sends `CTMMsgTech` (vtable
`0x4af13c`, ctor `0x41d5f1`: building id, on/off flag at `+0x12`). The pass
runs after moves (`0x419e07`), attacks (`0x41a7a3`), new map objects
(`0x4180b9`) and at phase start (`CServerPhaseTactic` slot 25, `0x4160d9`).

`0x41b07a` re-runs the pass and sums the captured rewards: into the battle
result message (`0x413f4c`, field `+0xa`) and, in a campaign, into the
player's technology points on a win (`0x47f2e5` -> `0x47e6f1`).

openlibday: `libday_view::tech::captured` (re-judged every frame while no
walk or shot plays, and once more as the battle ends), `Flow.tech`.

## Display

- **Map** (`CIsoAnimTech`, draw `0x46ac4d`): a captured building shows
  `LILBULB` (`SYMBOLS.DBI`, 54x56 cell) on its footprint's top-left tile,
  and the capture plays the `tech` sound (`SOUNDS\TECH.WAV`).
- **MAP overview** (`0x459530`): every technology building, captured or not
  (`0x459bce`), blinks `LBULB`: `LIBDAY.EXE` bitmaps 305 (bright), 306,
  307 (dark), 308, 15x22, black transparent, one frame per 175 ms (timer
  `0x45a162`), drawn with its top-left (2, 8) pixels up-left of the
  footprint's centre (`0x48b187`, `0x459dc6`).
- **Report**: string `0x206` "New technologies found : %1!d!", drawn by
  the campaign aftermath (`CAftermathInterface` slot 20, `0x45e4cd`). The
  skirmish aftermath (`CAftermathSkirInterface` slot 20, `0x45e6e8`) is a
  no-op; openlibday shows the line in skirmishes too.

Related strings: `0x286` "TECH %1!d!" (building info, `0x455fac`),
`0x360` "Tech: %1!d!".

Test: `scripts/tech.txt` (Testing Ground).
