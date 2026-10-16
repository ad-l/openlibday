# Multiplayer plan

Status: **plan, not implemented.** Compatibility with the original
executable's network protocol is not a goal.

## Goals

- 2–4 human players on home computers, one of them the **host**; no central
  server, no accounts.
- The **host is authoritative**: it owns the only random number generator,
  runs the AI, resolves every shot, drop, mine, reaction and reinforcement,
  and decides when phases change. Clients only send requests and play back
  what the host tells them.
- Modes, in order: multiplayer skirmish (any scenario, humans + AI), death
  match (`SCENS\DEATHM.ZIP`, all against all), co-op campaign (later, see
  open questions).
- Single player keeps working unchanged; it becomes "a host with no clients".

## What the original did

The executable links Interactive Magic's `CImMulti` (DirectPlay wrappers
`IMMULTR5.DLL`, `IMDP35.DLL`, `IMSGL5.DLL`) and is already split into a
server and a client (`CFH2ServerApp` / `CFH2ClientApp`, `CServerPhase*` /
`CClientPhase*`, `CServerMode{Skirmish,Campaign,DeathMatch}`):

- clients send requests: `CRequestUnitMoveMsg`, `CRequestUnitShootMsg`,
  `CRequestEndTactTurnMsg`, `CReqDeployUnitsMsg`, `CReqReinfUnitsMsg`,
  `CReqUpdUnitTechMsg`, build/recycle requests;
- the server answers with resolved tactical messages bundled in
  `CTacticMMsgList`: `CTMMsgMovement`, `Shooting`, `DmgUnit`, `DtrUnit`,
  `NewUnit`, `AutoUnit`, `MineExplosion`, `UnitAA`, `Tech`, `Bridge`,
  `BurnedTerrain`, `Repair`, `UpdateUnit`, plus AI tactic messages;
- phase messages (`GoDeploy`, `GoTactic`, `GoTacticWait`,
  `GoAftermath{Skirmish,DeathMatch,Campaign,CounterAtt}`), objectives, chat
  (`SOUNDS\CHAT.WAV`), saves and cheats;
- a lobby (`CMenuSessions`, `CMenuPlayerLogin`, `CMenuLobby`,
  `CMenuSkirmishMulti`, `CMenuNewCampaignMulti`), a turn timer (the skirmish
  dialog's "SET TIME") and a waiting video (`Waiting.smk`);
- starting zones merge by the number of human players (`0x44049C`, see the
  modding README).

We follow the same shape: **requests up, resolved events down**.

## Model: host-authoritative command/event stream

Pure lockstep (everyone simulates from the same seed and inputs) is simpler
on the wire, but any divergence anywhere desyncs the game, and it lets every
client predict hidden rolls. Instead:

1. A client sends an **intent** (`Move { unit, path }`, `Shoot { unit,
   target }`, `EndTurn`, `Deploy`, `BuySupport`, `LaunchSupport`, `Upgrade`,
   `Chat`, ...).
2. The host validates it against its state (whose turn it is, ownership, AP,
   reachability, money) and rejects or executes it.
3. Execution produces an ordered list of **events** carrying every random
   outcome already resolved (where a shot lands, the damage, which units
   die, reaction fire, mines triggered, drop positions, ...).
4. The host broadcasts the events. Every peer, the host included, applies
   them through the *same* `apply(event)` code and animates them. Clients
   never roll dice.
5. After each batch every peer sends a **state hash**; a mismatch triggers a
   full resync from the host's snapshot (the battle save JSON).

AI turns are the same thing: the AI produces intents on the host.

## Required refactor (milestone 0, useful even without networking)

Today the `Viewer` mixes simulation and presentation. UI handlers mutate the
world directly, `Shot` carries a whole result `Scenario` swapped in at a frame,
and one `self.rng` serves both rules and cosmetics. Before any networking:

- **Command layer.** Every rule-changing action goes through one
  `Viewer::submit(Intent, player)` with a validation function. Mouse, keyboard,
  the AI and the script runner all call it. The script runner's UI-level
  commands stay; add simulation-level script commands for replay tests.
- **Events out of resolution.** Split each action into *resolve* (rules +
  RNG, host only) → `Vec<Event>` and *apply* (deterministic, all peers),
  with animation driven from the event list. Start with the `Shot` result
  (damage, removed units/buildings, ruins, explosions, reaction bursts),
  then movement (interrupted moves, mines), supports (`support::execute`,
  drops landing on occupied tiles) and turn changes (reinforcements, AP
  regeneration, objectives, counterattacks).
- **RNG split.** Rule RNG (host only) and a cosmetic RNG (local, never
  affects state: smoke, scatter frames, sound variants). Audit the current
  uses in `damage.rs`, `scatter.rs`, `support.rs`, `combat.rs`, `ai.rs`,
  `automated.rs`, `moves.rs`, `campaign.rs`.
- **Players.** Replace the `PLAYER = 1` / `AI = 0` constants and
  `skirmish_players()` with a player table: slot, name, colour, controller
  (local human / remote human / AI), team, money, tech, supports budget. The
  UI "local player" becomes a parameter. Turn order, objectives, AI budget,
  drop tech, zone merge and reinforcement code read from the table.
- **State hash.** A stable hash of the simulation state (units, buildings,
  terrain damage, money, turn, RNG position), never of presentation state.
- **Replay log.** Record intents and events; replaying them must reproduce
  the hash. This is the first networking test and gives replays for free.

Single player must pass all existing tests and scripts after each step.

## Transport

- **TCP** with length-prefixed messages. The game is turn-based and
  low-bandwidth, so ordered and reliable delivery is what we want. Encoding
  is serde: JSON while developing, optionally `bincode`/`postcard` later.
  One network thread per connection exchanging messages with the game loop
  over channels. No async runtime is needed.
- **Handshake:** protocol version, game data hash (so everyone has the same
  `LIBDAY.EXE` data and mods), player name, optional session password.
- **Reachability without a server:**
  1. LAN: UDP broadcast discovery, which lists hosts in the session menu.
  2. Internet: the host asks the router for a port mapping with **UPnP IGD**
     (`igd-next` crate) and, where available, NAT-PMP/PCP. It shows its
     external `IP:port` as a copyable join code.
  3. Fallback: manual port forwarding, IPv6 direct, or a VPN such as
     Tailscale or ZeroTier. Document these in the README.
  4. Hole punching needs a rendezvous server, so it is out of scope.
- Keepalive pings; a timeout marks a player disconnected.

## Protocol sketch

```
Client → Host: Hello, Ready, Intent(seq, Intent), StateHash(turn, seq, hash),
               Chat, ResyncRequest, Leave
Host → Client: Welcome(slot, lobby), Lobby(state), Start(snapshot, players),
               Events(seq, [Event]), Reject(seq, reason), Phase(..),
               Snapshot(state), Chat, Kick
```

Events are numbered. A client that misses a message reconnects and asks for
everything after its last sequence number, or for a snapshot.

## Hidden information

The host sends each client only what that player may know:

- enemy mines (`mines.rs`) are visible only to their owner and to players who
  detected them;
- the AI's budget and plans;
- enemy money and tech, if the original hides them.

Movement through an unseen mine field arrives as a move event that ends with a
mine-explosion event. Fog of war, if the original has one for multiplayer,
gets the same filtering. The filtering lives in a single `visible_to(player,
&Event)` function.

## Game flow and UI

- **Front menu:** enable `Button::MultiPlayer` (currently disabled in
  `front.rs`). It leads to *Host* / *Join* (LAN list plus manual address).
- **Lobby** (host):
  - map from the skirmish list or `DEATHM.ZIP`;
  - slots 1–4, each human, AI or closed, with colour and team;
  - starting resources and the turn timer ("SET TIME");
  - chat, ready flags, Start.
  - Clients see the same screen read-only, except for their name and ready flag.
- **Setup and deployment:** each player buys and upgrades in parallel. Each
  player gets their starting zone, merged per human count as `0x44049C`
  does. The host starts the battle when everyone is ready or the timer runs out.
- **Tactical turns:** sequential, one player at a time in scenario order. The
  others watch the events animate, with input locked and the HUD showing whose
  turn it is. Reaction fire against a waiting player is resolved by the host
  inside the active player's events. The turn timer ends the turn
  automatically.
- **Aftermath:** host-decided win/loss per player and team. Death match ends
  when one player or team is left.
- **Chat:** an overlay in all phases, with `CHAT.WAV`.

## Disconnects, saves, cheating

- **Disconnect:** pause the game and wait N seconds for a reconnect, matched by
  name or token. After that the host may hand the slot to the AI or drop it.
  On reconnect the client receives a snapshot and the events since.
- **Saves:** only the host saves; it reuses the battle save JSON plus the
  player table. Loading in the lobby re-binds slots to names.
- **Host migration:** not planned. If the host leaves, the game ends with a
  save.
- **Cheating:** clients can't change outcomes because every intent is
  validated and the rolls stay on the host. The host can cheat, which is
  acceptable among friends. Debug and cheat keys are disabled for clients.

## Testing

- Replay determinism: script a battle, record the log, replay it headless,
  and compare the hashes.
- In-process loopback: a host plus N clients over channels in one headless
  process. Script steps can act as any player
  (`as 2 click ...`), and every turn asserts that the hashes are equal.
- Two processes on `127.0.0.1`: `openlibday --headless --host PORT --script
  a.txt` and `--join 127.0.0.1:PORT --script b.txt`.
- Fault injection: drop a connection mid-turn and reconnect, and corrupt a
  client state to make sure a resync happens.
- Manual tests over the internet with UPnP between two home networks.

## Milestones

| # | Content | Done when |
|---|---------|-----------|
| MP0 | Command layer, resolve/apply events, RNG split, player table, state hash, replay log | single player unchanged; replay test passes |
| MP1 | In-process loopback host + client, 2-player hotseat-over-channels skirmish | loopback script plays a full battle with equal hashes |
| MP2 | TCP transport, handshake, manual IP join, waiting/turn HUD, chat | two local processes finish a skirmish |
| MP3 | Lobby UI, slots (human/AI/closed), teams, zone merge, turn timer | 3–4 players + AI from the menu |
| MP4 | LAN discovery, UPnP/NAT-PMP, join code | internet game between two homes |
| MP5 | Death match maps and aftermath, hidden info filtering | `DEATHM.ZIP` maps playable |
| MP6 | Disconnect/reconnect, resync, host saves/loads | fault-injection tests pass |
| MP7 | (optional) co-op campaign / counterattacks | to be designed |

## Open questions

- **Co-op campaign:** the original has `CMenuNewCampaignMulti` and
  `CServerModeCampaign`, and the campaign data contains "the other players'
  bases" (`campaign.rs`). We need to find out whether it was shared co-op or
  competitive provinces before designing MP7.
- **Teams:** check whether the original supports alliances, or whether death
  match is strictly free-for-all.
- **Turn order and AI with several humans:** do all human players move before
  the AI, or are turns interleaved in slot order?
- **Fog of war:** does the original hide units in multiplayer?

These questions can be answered by more disassembly of `CServerPhaseTactic`,
`CServerModeDeathMatch::init_scenario` and the multiplayer menus.
