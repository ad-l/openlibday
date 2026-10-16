# openlibday

An open-source reimplementation of the engine of **Liberation Day** (1998),
the turn-based isometric tactics game, for modern Windows (Linux to
follow).

openlibday contains **no original game assets or code**: it plays the game
from the data of a Liberation Day installation you own.

What it plays:

- the campaign, with base building, counterattacks and the continent
  map;
- skirmishes, on every original map;
- co-operative multiplayer for 2–4 players against the computer
  (skirmishes and campaigns), over the local network or the internet;
- unlimited saves;
- any window size, with interface scaling and map zoom.

## Installation (Windows)

1. Install Liberation Day from the original CD (or copy an existing
   installation). openlibday needs its game directory, the one holding
   `IMGS`, `GLOBALS`, `SCENS` and so on (for example `C:\libday`). The
   original game does not need to run.
2. Download `openlibday-VERSION-windows-x64.zip` from the
   [releases](https://github.com/ad-l/openlibday/releases).
3. Extract `openlibday.exe` into that game directory, next to `IMGS`
   (for example `C:\libday\openlibday.exe`).
4. Double-click `openlibday.exe`.

openlibday looks for the game data in the directory it is started from.
To keep it elsewhere, set the `LIBDAY_ROOT` environment variable to the
game directory, or start it with `openlibday --root C:\libday`.

Settings are in the in-game OPTIONS (main menu RESOLUTION, or MENU in
battle) and are saved, with saved games, in `%APPDATA%\openlibday\`.

## Multiplayer

One player hosts (MULTI PLAYER, enter a name, HOST); the others JOIN with
the host's address, or pick the game from the list when it is on the same
network. In the waiting area the host chooses a skirmish or a campaign,
the difficulty and a turn time limit, and can LOAD a multiplayer campaign
saved earlier (with the same players). Everyone needs the same openlibday
version and game data.

Over the internet, hosting asks the router to open TCP port 41530 (UPnP or
NAT-PMP), and the waiting area shows the address to give the others (WAN,
with Copy). If the router does not do it (the chat says so):

- forward TCP port 41530 on the host's router to the host's LAN address
  (shown in the waiting area), and give the others the host's public IP;
- or join over IPv6, if both homes have it (`[2001:db8::1]:41530`);
- or put everyone on a virtual network such as Tailscale or ZeroTier and
  use the host's address there.

A WAN address the chat flags as behind another router (carrier-grade NAT)
cannot be reached from outside: use IPv6 or a virtual network then. See
[docs/multiplayer.md](docs/multiplayer.md).

## Building from source

Requires stable Rust (1.88+), CMake and a C compiler (MSVC Build Tools on
Windows); SDL3 is built from source and linked statically.

```powershell
$env:LIBDAY_ROOT = 'C:\libday'     # the game directory, for running from the source tree
cargo run --release -p openlibday
cargo test --workspace             # tests needing game data skip without LIBDAY_ROOT
cargo build --release -p openlibday --features gui   # no console window (the release build)
```

`openlibday --help` lists the command-line options. `--script FILE
[--headless]` replays a test script (see `scripts/` and
`crates/openlibday/src/script.rs`); `cargo test` runs them when
`LIBDAY_ROOT` is set.

| Crate | Purpose |
|---|---|
| `libday-formats` | Decoders: DBI sprites, INI, Jet 3 (Access 97), ZIP, PE string tables, Smacker video and audio; encoders for modding |
| `libday-data` | Locates an installation, databases, scenarios, typed game definitions, campaigns and savegames |
| `libday-view` | Renderer-independent map scene, interface layout, combat and support rules |
| `libday-sim` | Movement rules and AP pathfinding, fire checks and line of sight |
| `libday-tools` | `libday` command-line tool for inspecting and exporting data |
| `openlibday` | The SDL3 game |

## Documentation

- [docs/PLAN.md](docs/PLAN.md): architecture and roadmap;
- [docs/campaigns.md](docs/campaigns.md), [docs/ai.md](docs/ai.md),
  [docs/objectives.md](docs/objectives.md), [docs/tech.md](docs/tech.md),
  [docs/automated.md](docs/automated.md),
  [docs/multiplayer.md](docs/multiplayer.md): how the engine reproduces
  the original;
- [tools/modding/README.md](tools/modding/README.md): the map/scenario and
  asset editors (Python/Tk, Windows) and the reverse-engineering
  knowledge base, including file formats and savegames.

## License

The code is under the MIT license ([LICENSE](LICENSE)). No game data is
included. The bundled Liberation Sans font
(`crates/libday-view/assets/fonts`) is under the SIL Open Font License 1.1.