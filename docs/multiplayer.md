# Multiplayer plan

Status: **plan, not implemented.** Compatibility with the original
executable's network protocol is not a goal.

## Goals

- 2–4 human players on home computers, one of them the **host**; no central
  server, no accounts.
- **Co-operative only:** the humans play together against the AI, in
  skirmishes and in the campaign. No player-versus-player (no death match)
  for now.
- The **host is authoritative**: it owns the rule RNG, runs the AI, orders
  every command and decides when phases change. Clients send requests and
  execute what the host tells them, with the host's dice.
- The original has no fog of war, so every peer may see the whole state.
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

We follow the same shape, requests up and authority on the host, but send
commands with the host's dice rather than resolved results (see below).

## Model: host-ordered commands with host seeds

Every rule RNG draw happens when an action starts: a shot's result scenario,
scatter, drop positions and damage are computed up front, and the AI's draws
are its own decisions. So instead of shipping resolved outcomes:

1. A client sends a **command** request (`Move { unit, steps }`,
   `Shoot { unit, target }`, `Turn`, `Overwatch`, `EndTurn`, `Deploy`,
   `Upgrade`, `LaunchSupport`, ...).
2. The host validates it against its state (whose turn it is, ownership, AP,
   money) and rejects it, or appends it to the game's command stream with a
   **fresh seed** drawn from its private RNG.
3. Every peer, the host included, executes the stream in order, each command
   once the previous one has finished playing, reseeding the rule RNG with
   the command's seed. Nobody can predict a roll before the host has
   accepted the command.
4. The AI runs on the host only; its actions are commands like anyone
   else's, so clients run the AI's commands, not the AI.
5. After each command every peer sends a **state hash**; a mismatch makes the
   host send a snapshot (the battle save JSON) to resynchronise.

This needs the simulation to be deterministic on every peer: the same
outcome whatever the frame rate, animation speed options or window size.

## Required refactor (MP0, useful even without networking)

- **State hash.** A stable hash of the simulation state (units, buildings,
  terrain, mines, money, turn, phase, RNG state), never of presentation
  state.
- **Frame-rate independence.** Walks advance by the wall clock, a whole
  walker's due steps at a time, so group walks (halted when one member is
  spotted) and other overlapping animations can depend on frame timing and
  on the speed options. Rule effects must happen in an order fixed by the
  simulation. Test: the same script at different frame rates and speeds
  ends with the same hash.
- **Command layer.** Every rule-changing action goes through one
  `Viewer::submit(Command)`, validated and queued until the simulation is
  idle. Mouse, keyboard, the AI and the script runner all call it.
- **Replay log.** Record the commands; replaying them from the same start
  must reproduce the hash. This is the first networking test and gives
  replays for free.
- **Seeds.** `submit` takes an optional seed that reseeds the rule RNG
  (unused in single player, whose rolls stay as they are). Presentation
  never draws from the rule RNG.
- **Players.** Replace the `PLAYER = 1` / `AI = 0` constants and
  `skirmish_players()` with a player table: slot, name, colour, controller
  (local human / remote human / AI), money, tech. The UI's "local player"
  becomes a parameter. Turn order, objectives, drop tech, zone merge and
  reinforcement code read from the table.

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
Client → Host: Hello, Ready, Request(id, Command), StateHash(seq, hash),
               Chat, ResyncRequest, Leave
Host → Client: Welcome(slot, lobby), Lobby(state), Start(snapshot, players),
               Command(seq, player, seed, Command), Reject(id, reason),
               Snapshot(seq, state), Chat, Kick
```

Commands are numbered. A client that misses a message reconnects and asks
for everything after its last sequence number, or for a snapshot.

## Hidden information

None: the original has no fog of war. Mines stay hidden from players who
have not detected them in the interface only; since the game is co-operative
this is not a cheating concern.

## Game flow and UI

- **Front menu:** enable `Button::MultiPlayer` (currently disabled in
  `front.rs`). It leads to *Host* / *Join* (LAN list plus manual address).
- **Lobby** (host):
  - a skirmish map, or a new or saved campaign;
  - slots 1–4, each human or closed, with colour;
  - starting resources and the turn timer ("SET TIME");
  - chat, ready flags, Start.
  - Clients see the same screen read-only, except for their name and ready flag.
- **Setup and deployment:** each player buys and upgrades in parallel. Each
  player gets their starting zone, merged per human count as `0x44049C`
  does. The host starts the battle when everyone is ready or the timer runs out.
- **Tactical turns:** sequential, one player at a time in scenario order. The
  others watch the commands play, with input locked and the HUD showing whose
  turn it is. Reaction fire against a waiting player plays inside the active
  player's command. The turn timer ends the turn automatically.
- **Aftermath:** the humans win or lose together, decided on the host.
- **Chat:** an overlay in all phases, with `CHAT.WAV`.

## Disconnects, saves, cheating

- **Disconnect:** pause the game and wait N seconds for a reconnect, matched by
  name or token. After that the host may hand the slot to the AI or drop it.
  On reconnect the client receives a snapshot and the commands since.
- **Saves:** only the host saves; it reuses the battle save JSON plus the
  player table. Loading in the lobby re-binds slots to names.
- **Host migration:** not planned. If the host leaves, the game ends with a
  save.
- **Cheating:** clients can't change outcomes because every intent is
  validated and the seeds come from the host. The host can cheat, which is
  acceptable among friends. Debug and cheat keys are disabled for clients.

## Testing

- Replay determinism: script a battle, record the commands, replay them
  headless, and compare the hashes; run the same script at different frame
  rates and speed options and compare the hashes.
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
| MP0 | State hash, frame-rate independence, command layer, replay log, seeds, player table | single player unchanged; replay and frame-rate tests pass |
| MP1 | In-process loopback host + client, 2-player hotseat-over-channels skirmish | loopback script plays a full battle with equal hashes |
| MP2 | TCP transport, handshake, manual IP join, waiting/turn HUD, chat | two local processes finish a skirmish |
| MP3 | Lobby UI, slots (human/closed), zone merge, turn timer | 3–4 players against the AI from the menu |
| MP4 | LAN discovery, UPnP/NAT-PMP, join code | internet game between two homes |
| MP5 | Disconnect/reconnect, resync, host saves/loads | fault-injection tests pass |
| MP6 | Co-op campaign: shared continent, bases, counterattacks | a continent played by two players |
| later | Player versus player (`SCENS\DEATHM.ZIP`) | not planned yet |

## Open questions

- **Co-op campaign:** the original has `CMenuNewCampaignMulti` and
  `CServerModeCampaign`, and the campaign data contains "the other players'
  bases" (`campaign.rs`). We need to find out how the continent, money and
  research are shared before designing MP6.
- **Turn order with several humans:** do all human players move before the
  AI, one after the other, or at the same time?

These questions can be answered by more disassembly of `CServerPhaseTactic`,
`CServerModeCampaign` and the multiplayer menus.
