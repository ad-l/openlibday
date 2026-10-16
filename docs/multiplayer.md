# Multiplayer plan

Status: **MP0, MP1 and MP2 done** (state hash, frame-rate independence,
command layer, seeds, replay, player table; in-process loopback host and
client; TCP sessions, waiting area and chat); MP3 next. Compatibility with the original
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

### Done so far

- `sync.rs`: `state_hash` (script `hash`, `hash` in the state JSON). It
  leaves out what only the host keeps: the AI's unit orders and order
  targets, its support budget, and `rand()` (each command restarts it).
- Walk events play in a global due-time order, so walks give the same
  result whatever the frame rate.
- `command.rs`: `Command` (move, attack, turn, overwatch, auto fire,
  support, scripted support, deploy, recall, upgrade, start battle,
  begin/finish reinforcement, end turn; the AI's walk, batch walk, shoot
  and end of turn) and `Entry { player, seed, check, command }`. The UI
  checks what it can and calls `order`; the AI calls `order_ai`; both go
  through `submit`, which on a host (`net.seeds`) draws a seed and records
  `check`, the state hash the command was given in. `run` reseeds `rand()`,
  logs the entry and applies it, validating ownership and phase again.
  A follower (`net.inbox`) runs received entries one at a time while
  nothing plays, does not run the AI, and records the first entry whose
  `check` it does not match (`net.desync`).
- Script commands `seeds N`, `commands FILE` and `replay FILE`;
  `scripts/mp_host.txt` plays a modern-AI skirmish as a host and
  `scripts/mp_client.txt` follows its log to the same hash (the
  `replay_scripts` test; the client also matches at other frame rates).
- AI-private decisions (its budget, which supports to buy, its plans) stay
  outside commands: only their effects are commands.
- `players.rs`: `Players`, the seats (humans in slots 1-4, controller
  local or AI, the AI in slot 0) and `local`, the human this computer
  plays now. Turns run through the humans in slot order, then the AI.
  Credits, start zones and drop tech are per slot; commands are validated
  against `clock.player`. The UI asks for the local seat (`my_turn`,
  `credits()`), applying a command uses the command's player. Several local
  humans give hotseat play (script `humans N`, `scripts/hotseat.txt`):
  they deploy one after the other (CONTINUE hands over) and take their
  turns in order. Still shared: the skirmish tech setup. The campaign
  had one human (`CAMPAIGN_PLAYER`) until MP6.
- MP1, `link.rs`: `ToHost::Request { command }` and `ToClient::Start {
  map, humans, slot }` / `ToClient::Run { entry }`, JSON over an mpsc
  `loopback()` pair. The host keeps a `Peer` per client in `net.peers`
  and broadcasts every entry it runs; a client (`net.host`) sends its
  orders as requests, which the host submits as that peer's slot when
  nothing plays. `receive`/`pump` move messages each frame; link errors
  land in `net.fault`. Players can be `Remote` (`Players::online`).
- Choosing a skirmish on the host sends `Start`, which opens the same map
  on the client (`join_skirmish`). Only the host buys technology; leaving
  the tech screen is the `Brief` command. Deployment runs in parallel:
  each human deploys in its zone and CONTINUE is `Ready`; the battle starts
  once every human is ready. UI side effects of commands (messages,
  sounds, screen and selection changes) happen only for the local player.
- Script commands `join` (another copy joins over a loopback link), `as N
  COMMAND` (run a command in slot N's copy) and `synced`; the `win`/`lose`/
  `kill` cheats act on every copy. `scripts/loopback.txt` (the
  `loopback_script` test) deploys, plays turns with both humans and the AI,
  and wins the battle with the copies in step throughout.
- MP2, `link.rs` + `lobby.rs`: the same messages as length-prefixed JSON
  over TCP (default port 41530). A client sends `Hello { version, digest,
  name }`; the host refuses a different build, different game data (a
  digest of the data files), a started game or a full one (4 humans), and
  otherwise answers `Welcome { slot, names }`, the lowest free slot.
  `Roster` updates the names; slots are renumbered without gaps while in
  the waiting area. MULTI PLAYER opens SESSIONS (name, host address, HOST /
  JOIN); both lead to the WAITING AREA (players, game information, chat).
  The host's START opens the skirmish list; the report's CONTINUE returns
  everyone to the waiting area. A player who leaves a started battle is
  stood in for: the host readies, finishes reinforcing and ends turns for
  that slot. Chat (the CHAT button or the waiting area) goes through the
  host, which relays it to everyone; the last lines are overlaid on the
  battle for 10 s. Saving, loading and restarting are disabled in network
  games. CLI `--host`, `--join ADDR`, `--port N`, `--name NAME`; script
  commands `name`, `host`, `connect`, `say` and `await PATH VALUE`.
  `scripts/tcp_host.txt` + `tcp_client.txt` (the `tcp_scripts` test) play
  a skirmish between two processes and compare state hashes.
- Waiting-area game information: the host's LAN address (`IP:port`) and
  a WAN address (empty until UPnP), each with a COPY button; game mode
  (Skirmish / Campaign), difficulty (campaign only) and turn limit (1–20 min or
  unlimited). Only the host's arrows change them; `Settings` messages keep
  the clients in step (protocol 3). With a limit, the bottom bar counts
  down the human turn (red in the last 30 s), otherwise it shows the time
  elapsed; when it runs out the host finishes reinforcing or ends the turn
  for that player through the command log. Script command `turnlimit N`;
  `scripts/lobby_settings.txt` and turn 3 of the TCP scripts test them.
- MP4, internet play:
  - `router.rs`: hosting asks the router to forward the TCP port to this
    computer, first with UPnP IGD (`igd-next`; a 1-hour lease, or a
    permanent one if that is all the router takes, or another external
    port if the port is taken), then NAT-PMP (`natpmp`). A thread renews
    the lease every 20 min and removes the mapping when the game is left
    or closed (if the game crashes the lease expires on its own). The
    WAN row shows the external `IP:port` (the join code, with COPY), or
    "Asking the router..." / "Router did not open the port"; a chat
    notice says which, and warns when the router's own address is
    private (carrier-grade NAT or a second router), where the mapping
    does not help. Tested with UPnP on a home router: the mapping is
    listed while hosting and gone afterwards.
  - `discovery.rs`: hosts answer UDP questions on port 41530 with
    `{id, version, name, port, lan, players, max, started}`. The SESSIONS
    dialog asks every second (to 255.255.255.255, each interface's
    broadcast address and 127.0.0.1) and lists the games heard in the
    last 4 s; a host answering on several networks is shown at its main
    LAN address. Clicking a game fills the host address; JOIN joins it.
  - Transport: an idle link sends an empty frame every 20 s and one
    silent for 90 s is closed, so routers keep connections open and dead
    peers are noticed. Hosts also listen on IPv6 (`[::]:port`), so IPv6
    addresses can be joined directly. Protocol 4.
  - Scripted runs never touch the router or the network's broadcast:
    script commands `upnp on|off`, `discover on|off`, `sessions` (open
    SESSIONS) and `sleep MS` (real time, for network threads); state
    `online.lan`, `online.wan`, `online.wan_note` and `games`. The TCP
    scripts find the host by discovery and join it from the list.
- MP6, co-op campaign (protocol 5):
  - START in Campaign mode makes a new campaign for everyone in the
    waiting area at the chosen difficulty and sends it with
    `ToClient::Campaign { state, humans, slot, seed }`; every copy starts
    from the same state and `rand()` seed (`start_online_campaign`,
    `follow_campaign`).
  - `campaign::State` v2 keeps a `Member` per human (slot, name, credits,
    technology points, unit levels); v1 saves become one member. The
    continent, provinces, base layout and stage are shared.
  - Battles: each human defends their own headquarters and deploys in
    their own zone with their own deployment points (the sum of their HQ
    and Deploy buildings, scaled by `PLAYERS_PERCENT`) and unit levels.
    The base is lost only when every human HQ falls.
  - Base building runs in parallel: building, moving and recycling are
    the commands `BaseBuild`, `BaseMove`, `BaseRecycle`, paid from the
    player's own credits; each player builds next to their own roads.
    ARMY upgrades are `ArmyUpgrade`, paid from the player's own points
    and needing their own Research Ctr.
  - The host chooses the province (`Invade`) and closes the battle
    reports (`Proceed`); clients are told to wait. Rewards (credits and
    technology) go to every member in full.
  - Only the host saves (between battles, into `campaign_online.json` and
    the save list); an online campaign save cannot yet be resumed (alone
    or online). Winning or losing the campaign returns to the waiting
    area.
  - Script command `mpcampaign [DIFFICULTY]`; `scripts/mp_campaign.txt`
    plays the first attack, parallel base building, an upgrade and a
    province with two copies in step.

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
     external `IP:port` as a copyable join code. (Done, MP4.)
  3. Fallback: manual port forwarding, IPv6 direct, or a VPN such as
     Tailscale or ZeroTier. Document these in the README.
  4. Hole punching needs a rendezvous server, so it is out of scope.
- Keepalive pings; a timeout marks a player disconnected. (Done, MP4.)

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
| MP4 | LAN discovery, UPnP/NAT-PMP, join code (done; awaiting a game between two homes) | internet game between two homes |
| MP5 | Disconnect/reconnect, resync, host saves/loads | fault-injection tests pass |
| MP6 | Co-op campaign: shared continent, bases, counterattacks (done; resuming online saves to do) | a continent played by two players |
| later | Player versus player (`SCENS\DEATHM.ZIP`) | not planned yet |

## Open questions

- **Co-op campaign:** answered for MP6 (see above): one shared continent
  and base, per-player money and research, host-chosen provinces. Still
  open: resuming a saved online campaign in the waiting area.
- **Turn order with several humans:** do all human players move before the
  AI, one after the other, or at the same time?

These questions can be answered by more disassembly of `CServerPhaseTactic`,
`CServerModeCampaign` and the multiplayer menus.
