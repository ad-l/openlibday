//! Scripted runs (`--script FILE`, optionally `--headless`): a list of
//! commands that drive the game through its own input handlers on a
//! virtual clock, and record screenshots, frame sequences and the game
//! state. Without `--headless` the run is shown in the window (paced in
//! real time) and the game stays interactive once the script ends.
//!
//! One command per line; `#` starts a comment. Coordinates are output
//! pixels for `click`/`move` and map tiles for `tile`/`hover`/`centre`.
//!
//! ```text
//! fps N                 frame rate of wait/idle/record (default 30)
//! wait N                advance N frames
//! idle [N]              advance until no unit moves, the AI's turn is over
//!                       and no command waits (at most N frames, default 3000)
//! key NAME              press a key (SDL name: Return, Escape, Up, F5, A...)
//! click X Y [right]     click output pixel (X, Y)
//! move X Y              move the mouse to output pixel (X, Y)
//! tile X Y [right]      click map tile (X, Y), recentring if it is off screen
//! hover X Y             move the mouse over map tile (X, Y)
//! inspect X Y         hold the right button on map tile (X, Y)
//! button NAME           click an interface button (EndTurn, TechContinue...)
//! province N            click province N on the continent overview
//! continent N           jump the campaign to continent N (0-3) with every
//!                       province but the capital captured
//! skirmish NAME|N       pick a skirmish by file, title or list number
//!                       (opening the selector if it is not shown)
//! deploy NAME           pick a unit type in the deployment list
//! support NAME|ID       open AUTOMATED UNITS and arm that support (then
//!                       aim it with `tile`)
//! centre X Y            centre the view on map tile (X, Y)
//! zoom F                set the zoom (1 = original map pixels)
//! screenshot FILE       save the screen as PNG
//! record DIR N          save N consecutive frames as DIR\frameNNNN.png
//! state [FILE]          write the game state as JSON (stdout without FILE;
//!                       `view` is the tile at the middle of the map view)
//! expect PATH VALUE     fail unless the state at PATH (dotted; `id=N`
//!                       picks the array element with that id, `-1` the
//!                       last) is VALUE
//! echo TEXT             print TEXT
//! hash                  print the simulation state hash (`hash` in `state`)
//! type TEXT             type TEXT on the keyboard (the NEW GAME name)
//! seed N                reseed the game's rand() (scripts start at 1)
//! seeds N               act as a multiplayer host: every command gets a seed,
//!                       drawn from a generator seeded with N
//! humans N              N humans (1-4) take turns at this computer (hotseat)
//!                       from the next skirmish on
//! commands FILE         write the commands run so far as JSON
//! replay FILE           follow the commands in FILE (as a multiplayer client:
//!                       the AI does not think, its commands come in)
//! mine KIND OWNER X Y [seen]  lay an armed mine of type KIND, hidden from
//!                       all but OWNER (from nobody with `seen`)
//! grant ID            add automated unit ID to the support list
//! credits N             set the player's credits
//! place I X Y         move unit I (state index) to map tile (X, Y)
//! ap I N              set unit I's remaining action points to N
//! reactions on|off    whether overwatch spots moving units (default on)
//! ai off|legacy|modern  how the computer player plays (default: settings)
//! option NAME [STEP]    change an OPTIONS table setting (Scale, Fullscreen, Intro,
//!                       Scroll, Grid, TerrainOnly, Volume, Ai) by STEP
//!                       (default 1; check boxes toggle)
//! join                  another copy of the game joins this one (the host,
//!                       seeded with 1 unless `seeds` came first) over an
//!                       in-process link and plays the next human slot
//! as N COMMAND          run COMMAND in the copy playing slot N (1: the
//!                       host); `wait` and `idle` always run every copy
//! synced                fail unless every copy has the host's state hash
//! name NAME             the player's name (for network games)
//! host [PORT]           host a network game (default port 41530)
//! connect ADDRESS       join the network game at ADDRESS[:PORT]
//! say TEXT              send a chat line in a network game
//! await PATH VALUE [S]  advance frames until the state at PATH is VALUE,
//!                       failing after S seconds of real time (default 60);
//!                       (how a network game waits for the other computers)
//! quit                  end the program
//! ```

use std::path::{Path, PathBuf};
use std::time::Duration;

use libday_view::tile_center;
use sdl3::EventPump;
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::PixelFormat;
use sdl3::render::Canvas;
use sdl3::video::Window;
use serde_json::{Value, json};

use crate::Viewer;
use crate::hud::{Gfx, Result};

const DEFAULT_FPS: f64 = 30.0;
const IDLE_FRAMES: u32 = 3000;

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    Fps(f64),
    Wait(u32),
    Idle(u32),
    Key(String),
    Click {
        x: i32,
        y: i32,
        right: bool,
    },
    Move {
        x: i32,
        y: i32,
    },
    Tile {
        x: i64,
        y: i64,
        right: bool,
    },
    Hover {
        x: i64,
        y: i64,
    },
    Inspect {
        x: i64,
        y: i64,
    },
    Place(usize, i64, i64),
    Ap(usize, i64),
    Reactions(bool),
    Ai(Option<crate::settings::AiMode>),
    Button(String),
    Province(i64),
    Continent(i64),
    Option(String, i32),
    Skirmish(String),
    Deploy(String),
    /// Selects a structure of the base editor's lists by name.
    Build(String),
    Support(String),
    Grant(i64),
    Credits(i64),
    Centre(f64, f64),
    Zoom(f32),
    Screenshot(PathBuf),
    Record(PathBuf, u32),
    State(Option<PathBuf>),
    Expect(String, String),
    Echo(String),
    Hash,
    /// Text typed on the keyboard (the NEW GAME dialog's name).
    Type(String),
    Seed(u32),
    Seeds(u32),
    Humans(usize),
    Commands(PathBuf),
    Replay(PathBuf),
    Mine {
        kind: i64,
        owner: i64,
        x: i64,
        y: i64,
        seen: bool,
    },
    /// The win / lose cheats: the battle ends with this outcome.
    Force(libday_data::objective::Outcome),
    /// Removes the unit or building with this id as if destroyed.
    Kill(i64),
    /// Another copy of the game joins this one, the host, over a loopback
    /// link.
    Join,
    /// A command run by the copy playing slot N.
    As(i64, Box<Command>),
    /// Every copy has the host's state hash.
    Synced,
    Name(String),
    Host(u16),
    Connect(String),
    Say(String),
    Await(String, String, f64),
    Quit,
}

/// A command and its line number.
pub type Line = (usize, Command);

/// Parses a script; errors name the line.
pub fn parse(text: &str) -> std::result::Result<Vec<Line>, String> {
    let mut out = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let command = parse_line(line).map_err(|e| format!("line {}: {e}: {line}", i + 1))?;
        out.push((i + 1, command));
    }
    Ok(out)
}

fn parse_line(line: &str) -> std::result::Result<Command, String> {
    let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    let args: Vec<&str> = rest.split_whitespace().collect();
    fn num<T: std::str::FromStr>(s: Option<&&str>) -> std::result::Result<T, String> {
        let s = s.ok_or("missing argument")?;
        s.parse().map_err(|_| format!("bad number {s}"))
    }
    let right = |i: usize| match args.get(i) {
        None => Ok(false),
        Some(&"right") => Ok(true),
        Some(other) => Err(format!("unexpected {other}")),
    };
    let text = || {
        if rest.is_empty() {
            Err("missing argument".to_string())
        } else {
            Ok(rest.to_string())
        }
    };
    Ok(match word.to_ascii_lowercase().as_str() {
        "fps" => Command::Fps(
            num(args.first())
                .and_then(|f: f64| (f > 0.0).then_some(f).ok_or("fps must be positive".into()))?,
        ),
        "wait" => Command::Wait(num(args.first())?),
        "idle" => Command::Idle(match args.first() {
            Some(_) => num(args.first())?,
            None => IDLE_FRAMES,
        }),
        "key" => Command::Key(text()?),
        "click" => Command::Click {
            x: num(args.first())?,
            y: num(args.get(1))?,
            right: right(2)?,
        },
        "move" => Command::Move {
            x: num(args.first())?,
            y: num(args.get(1))?,
        },
        "tile" => Command::Tile {
            x: num(args.first())?,
            y: num(args.get(1))?,
            right: right(2)?,
        },
        "reactions" => Command::Reactions(match args.first() {
            Some(&"on") => true,
            Some(&"off") => false,
            _ => return Err("expected on or off".into()),
        }),
        "ai" => Command::Ai(match args.first() {
            Some(&"off") => None,
            Some(m) => {
                Some(crate::settings::AiMode::parse(m).ok_or("expected off, legacy or modern")?)
            }
            None => return Err("expected off, legacy or modern".into()),
        }),
        "place" => Command::Place(num(args.first())?, num(args.get(1))?, num(args.get(2))?),
        "ap" => Command::Ap(num(args.first())?, num(args.get(1))?),
        "inspect" => Command::Inspect {
            x: num(args.first())?,
            y: num(args.get(1))?,
        },
        "hover" => Command::Hover {
            x: num(args.first())?,
            y: num(args.get(1))?,
        },
        "button" => Command::Button(text()?),
        "province" => Command::Province(num(args.first())?),
        "continent" => Command::Continent(num(args.first())?),
        "option" => Command::Option(
            args.first().ok_or("missing option")?.to_string(),
            match args.get(1) {
                Some(_) => num(args.get(1))?,
                None => 1,
            },
        ),
        "skirmish" => Command::Skirmish(text()?),
        "deploy" => Command::Deploy(text()?),
        "build" => Command::Build(text()?),
        "support" => Command::Support(text()?),
        "grant" => Command::Grant(num(args.first())?),
        "credits" => Command::Credits(num(args.first())?),
        "centre" | "center" => Command::Centre(num(args.first())?, num(args.get(1))?),
        "zoom" => Command::Zoom(num(args.first())?),
        "screenshot" => Command::Screenshot(text()?.into()),
        "record" => Command::Record(
            args.first().ok_or("missing directory")?.into(),
            num(args.get(1))?,
        ),
        "state" => Command::State((!rest.is_empty()).then(|| rest.into())),
        "expect" => {
            let (path, value) = rest
                .split_once(char::is_whitespace)
                .ok_or("missing value")?;
            Command::Expect(path.into(), value.trim().into())
        }
        "echo" => Command::Echo(rest.into()),
        "hash" => Command::Hash,
        "type" => Command::Type(text()?),
        "seed" => Command::Seed(num(args.first())?),
        "seeds" => Command::Seeds(num(args.first())?),
        "humans" => Command::Humans(num(args.first())?),
        "commands" => Command::Commands(text()?.into()),
        "replay" => Command::Replay(text()?.into()),
        "mine" => Command::Mine {
            kind: num(args.first())?,
            owner: num(args.get(1))?,
            x: num(args.get(2))?,
            y: num(args.get(3))?,
            seen: args.get(4).is_some_and(|a| *a == "seen"),
        },
        "win" => Command::Force(libday_data::objective::Outcome::Won),
        "lose" => Command::Force(libday_data::objective::Outcome::Lost),
        "kill" => Command::Kill(num(args.first())?),
        "join" => Command::Join,
        "as" => {
            let (slot, inner) = rest
                .split_once(char::is_whitespace)
                .ok_or("missing command")?;
            let slot = slot.parse().map_err(|_| format!("bad number {slot}"))?;
            Command::As(slot, Box::new(parse_line(inner.trim())?))
        }
        "synced" => Command::Synced,
        "name" => Command::Name(text()?),
        "host" => Command::Host(match args.first() {
            Some(_) => num(args.first())?,
            None => crate::link::DEFAULT_PORT,
        }),
        "connect" => Command::Connect(text()?),
        "say" => Command::Say(text()?),
        "await" => Command::Await(
            args.first().ok_or("missing path")?.to_string(),
            args.get(1).ok_or("missing value")?.to_string(),
            match args.get(2) {
                Some(_) => num(args.get(2))?,
                None => 60.0,
            },
        ),
        "quit" | "exit" => Command::Quit,
        _ => return Err(format!("unknown command {word}")),
    })
}

/// The owner of unit or building `id` on the battlefield.
fn owner_of(viewer: &Viewer, id: i64) -> std::result::Result<i64, String> {
    let s = &viewer.map.battle.scenario;
    s.units
        .iter()
        .find(|u| u.id == id && u.arrival == 0)
        .map(|u| u.owner)
        .or_else(|| s.buildings.iter().find(|b| b.id == id).map(|b| b.owner))
        .ok_or_else(|| format!("no unit or building {id}"))
}

/// Removes unit or building `id`, credited to `attacker`.
fn kill(viewer: &mut Viewer, id: i64, attacker: i64) -> std::result::Result<(), String> {
    owner_of(viewer, id)?;
    let before = viewer.map.battle.scenario.clone();
    let s = &mut viewer.map.battle.scenario;
    if let Some(i) = s.units.iter().position(|u| u.id == id && u.arrival == 0) {
        s.units.remove(i);
    } else if let Some(i) = s.buildings.iter().position(|b| b.id == id) {
        s.buildings.remove(i);
    }
    viewer.map.battle.tally(&before, attacker);
    viewer.selected = None;
    viewer.preview = None;
    viewer.lock = None;
    viewer.rebuild();
    Ok(())
}

/// What drives a scripted run.
pub struct Runner<'a, 'r> {
    pub canvas: &'a mut Canvas<Window>,
    pub gfx: &'a mut Gfx<'r>,
    pub events: &'a mut EventPump,
    /// Shown in a window: present and pace every frame.
    pub visible: bool,
    pub frame: Duration,
    /// The copies joined to the scripted one (`join`), with their slots.
    pub peers: Vec<(i64, Viewer)>,
}

impl Runner<'_, '_> {
    /// Runs `lines` from `file`; returns `false` when the program should end.
    pub fn run(&mut self, viewer: &mut Viewer, file: &Path, lines: &[Line]) -> Result<bool> {
        self.frame = Duration::from_secs_f64(1.0 / DEFAULT_FPS);
        for (number, command) in lines {
            match self.command(viewer, command) {
                Ok(true) => self.show(viewer)?,
                Ok(false) => return Ok(false),
                Err(e) => return Err(format!("{}:{number}: {e}", file.display()).into()),
            }
        }
        Ok(true)
    }

    /// Runs one command; `Ok(false)` ends the program.
    fn command(&mut self, viewer: &mut Viewer, command: &Command) -> Result<bool> {
        match command {
            Command::Fps(fps) => self.frame = Duration::from_secs_f64(1.0 / fps),
            Command::Wait(n) => {
                for _ in 0..*n {
                    self.step(viewer)?;
                }
            }
            Command::Idle(max) => {
                let mut n = 0;
                while !self.quiet(viewer) {
                    if n == *max {
                        return Err(format!("still busy after {max} frames").into());
                    }
                    self.step(viewer)?;
                    n += 1;
                }
            }
            Command::Key(name) => {
                let key = Keycode::from_name(name).ok_or_else(|| format!("unknown key {name}"))?;
                if !viewer.key(key) {
                    return Ok(false);
                }
            }
            Command::Click { x, y, right } => {
                click(viewer, (*x as f32, *y as f32), *right);
            }
            Command::Move { x, y } => {
                let d = viewer.density;
                viewer.mouse_motion(*x as f32 / d, *y as f32 / d);
            }
            Command::Tile { x, y, right } => {
                let at = tile_point(viewer, *x, *y)?;
                click(viewer, at, *right);
            }
            Command::Hover { x, y } => {
                tile_point(viewer, *x, *y)?;
            }
            Command::Reactions(on) => viewer.reactions = *on,
            Command::Ai(mode) => viewer.ai_mode = *mode,
            Command::Place(i, x, y) => {
                let unit = viewer
                    .map
                    .battle
                    .scenario
                    .units
                    .get_mut(*i)
                    .ok_or("no such unit")?;
                (unit.x, unit.y) = (*x, *y);
                viewer.rebuild();
            }
            Command::Ap(i, ap) => {
                let unit = viewer
                    .map
                    .battle
                    .scenario
                    .units
                    .get_mut(*i)
                    .ok_or("no such unit")?;
                unit.action_points = Some(*ap);
                viewer.rebuild();
            }
            Command::Inspect { x, y } => {
                let (px, py) = tile_point(viewer, *x, *y)?;
                let d = viewer.density;
                viewer.mouse_down(MouseButton::Right, px / d, py / d);
            }
            Command::Button(name) => {
                let layout = viewer.layout();
                let button = layout
                    .buttons(viewer.screen, viewer.players.len())
                    .into_iter()
                    .find(|b| format!("{b:?}").eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("no button {name} on {:?}", viewer.screen))?;
                if !viewer.enabled(button) {
                    return Err(format!("button {name} is disabled").into());
                }
                let r = layout.button(button);
                let centre = ((r.x + r.w / 2) as f32, (r.y + r.h / 2) as f32);
                click(viewer, centre, false);
            }
            Command::Province(n) => {
                let st = viewer.strategic.as_ref().ok_or("no continent overview")?;
                let (mx, my) = st
                    .map
                    .inside(*n)
                    .ok_or_else(|| format!("no province {n}"))?;
                let (x0, y0, s) = viewer.strategic_origin().ok_or("no continent overview")?;
                let at = ((x0 + mx * s + s / 2) as f32, (y0 + my * s + s / 2) as f32);
                click(viewer, at, false);
            }
            Command::Continent(n) => viewer.jump_to_continent(*n)?,
            Command::Option(name, step) => {
                let item = crate::menu::ITEMS
                    .into_iter()
                    .find(|i| !i.is_heading() && i.name().eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("no option {name}"))?;
                viewer.change_option(item, *step);
            }
            Command::Skirmish(name) => {
                // From the main menu (or anywhere else), open the selector first.
                if viewer.screen != libday_view::ui::Screen::Skirmishes {
                    viewer.open_selector()?;
                }
                let lower = name.to_ascii_lowercase();
                let pick = viewer
                    .flow
                    .catalog
                    .iter()
                    .position(|e| {
                        let file = e.file_name.to_ascii_lowercase();
                        file == lower
                            || file.strip_suffix(".mdb") == Some(&lower)
                            || e.title.eq_ignore_ascii_case(name)
                    })
                    .or_else(|| {
                        let n: usize = name.parse().ok()?;
                        (1..=viewer.flow.catalog.len()).contains(&n).then(|| n - 1)
                    })
                    .ok_or_else(|| format!("no skirmish {name}"))?;
                viewer.flow.pick = pick;
                viewer.show_pick();
            }
            Command::Build(name) => {
                if !viewer.base_select_named(name) {
                    return Err(format!("{name} is not in the base editor's lists").into());
                }
            }
            Command::Deploy(name) => {
                let row = viewer
                    .deploy_row_named(name)
                    .ok_or_else(|| format!("{name} is not in the deployment list"))?;
                let rows = libday_view::ui::DEPLOY_ROWS;
                if let Some(setup) = &mut viewer.flow.setup {
                    if row < setup.scroll {
                        setup.scroll = row;
                    } else if row >= setup.scroll + rows {
                        setup.scroll = row + 1 - rows;
                    }
                }
                viewer.deploy_select(row);
            }
            Command::Support(name) => {
                viewer.open_supports();
                if viewer.screen != libday_view::ui::Screen::Automated {
                    return Err("AUTOMATED UNITS is not available".into());
                }
                let row = viewer
                    .support_row_named(name)
                    .ok_or_else(|| format!("{name} is not in the support list"))?;
                let rows = libday_view::ui::DEPLOY_ROWS;
                let scroll = &mut viewer.flow.support_scroll;
                if row < *scroll {
                    *scroll = row;
                } else if row >= *scroll + rows {
                    *scroll = row + 1 - rows;
                }
                viewer.support_select(row);
            }
            Command::Grant(id) => {
                if let Some(setup) = &mut viewer.flow.setup
                    && !setup.automated.contains(id)
                {
                    setup.automated.push(*id);
                }
            }
            Command::Credits(n) => *viewer.credits_mut() = *n,
            Command::Centre(x, y) => viewer.centre_on_tile(*x, *y),
            Command::Zoom(z) => {
                let v = viewer.layout().viewport();
                let centre = viewer.to_map(v.w as f32 / 2.0, v.h as f32 / 2.0);
                viewer.zoom = z * viewer.density;
                viewer.camera = (
                    centre.0 - v.w as f32 / (2.0 * viewer.zoom),
                    centre.1 - v.h as f32 / (2.0 * viewer.zoom),
                );
            }
            Command::Screenshot(path) => self.screenshot(viewer, path)?,
            Command::Record(dir, n) => {
                std::fs::create_dir_all(dir)?;
                for i in 0..*n {
                    self.screenshot(viewer, &dir.join(format!("frame{i:04}.png")))?;
                    self.step(viewer)?;
                }
            }
            Command::State(path) => {
                let text = serde_json::to_string_pretty(&state(viewer))?;
                match path {
                    Some(path) => std::fs::write(path, text + "\n")?,
                    None => println!("{text}"),
                }
            }
            Command::Expect(path, want) => {
                let got = state_at(viewer, path);
                if got.as_deref() != Some(want.as_str()) {
                    let got = got.unwrap_or_else(|| "nothing".into());
                    return Err(format!("expected {path} = {want}, got {got}").into());
                }
            }
            Command::Name(name) => viewer.front.name = name.clone(),
            Command::Host(port) => {
                viewer.front.port = *port;
                viewer.open_sessions();
                viewer.host_game();
                if let Some(status) = viewer.front.status.take() {
                    return Err(status.into());
                }
            }
            Command::Connect(address) => {
                viewer.front.address = address.clone();
                viewer.open_sessions();
                viewer.join_game();
            }
            Command::Say(text) => {
                viewer.front.chat = text.clone();
                viewer.send_chat();
            }
            Command::Await(path, want, seconds) => {
                let until = std::time::Instant::now() + Duration::from_secs_f64(*seconds);
                while state_at(viewer, path).as_deref() != Some(want.as_str()) {
                    if std::time::Instant::now() > until {
                        let got = state_at(viewer, path).unwrap_or_else(|| "nothing".into());
                        return Err(format!(
                            "{path} is still {got}, not {want}, after {seconds} s"
                        )
                        .into());
                    }
                    self.step(viewer)?;
                    if viewer.online() {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
            Command::Echo(text) => println!("{text}"),
            Command::Hash => println!("hash {:016x}", viewer.state_hash()),
            Command::Type(text) => viewer.text_input(text),
            Command::Seed(seed) => viewer.rng = libday_view::scatter::Rand::new(*seed),
            Command::Seeds(seed) => {
                viewer.net.seeds = Some(libday_view::scatter::Rand::new(*seed));
            }
            Command::Humans(n) => viewer.seats = crate::players::Players::new(*n),
            Command::Commands(file) => {
                let text = serde_json::to_string_pretty(&viewer.net.log)?;
                std::fs::write(file, text)?;
            }
            Command::Replay(file) => {
                let text = std::fs::read_to_string(file)?;
                let entries: Vec<crate::command::Entry> = serde_json::from_str(&text)?;
                viewer.net.inbox = Some(entries.into());
            }
            &Command::Mine {
                kind,
                owner,
                x,
                y,
                seen,
            } => {
                let mines = &mut viewer.map.battle.scenario.mines;
                let counter = mines.iter().map(|m| m.counter + 1).max().unwrap_or(1);
                mines.push(libday_data::scenario::Mine {
                    counter,
                    kind,
                    owner,
                    x,
                    y,
                    armed: true,
                    hidden_ai: !seen && owner != 0,
                    hidden: std::array::from_fn(|p| !seen && p as i64 + 1 != owner),
                });
                viewer.rebuild();
            }
            // Cheats act outside commands, so they act on every copy alike.
            Command::Force(outcome) => {
                for v in std::iter::once(&mut *viewer).chain(self.peers.iter_mut().map(|(_, v)| v))
                {
                    v.flow.forced = Some(*outcome);
                }
            }
            Command::Kill(id) => {
                let attacker = if viewer.seats.human(owner_of(viewer, *id)?) {
                    crate::skirmish::AI
                } else {
                    viewer.seats.local
                };
                for v in std::iter::once(&mut *viewer).chain(self.peers.iter_mut().map(|(_, v)| v))
                {
                    kill(v, *id, attacker)?;
                }
            }
            Command::Join => {
                let mut peer = viewer.peer()?;
                let slot = self.peers.len() as i64 + 2;
                let (to_client, to_host) = crate::link::loopback();
                viewer.net.peers.push(crate::link::Peer {
                    slot,
                    link: to_client,
                });
                viewer
                    .net
                    .seeds
                    .get_or_insert_with(|| libday_view::scatter::Rand::new(1));
                viewer.seats = crate::players::Players::online(slot as usize, 1);
                peer.net.host = Some(to_host);
                peer.net.inbox = Some(Default::default());
                self.peers.push((slot, peer));
            }
            Command::As(slot, inner) => {
                if matches!(
                    **inner,
                    Command::Wait(_) | Command::Idle(_) | Command::Fps(_)
                ) {
                    return self.command(viewer, inner);
                }
                if *slot == 1 {
                    return self.command(viewer, inner);
                }
                let i = self
                    .peers
                    .iter()
                    .position(|(s, _)| s == slot)
                    .ok_or_else(|| format!("no copy plays slot {slot}"))?;
                let mut peer = self.peers.remove(i);
                let result = self.command(&mut peer.1, inner);
                self.peers.insert(i, peer);
                return result;
            }
            Command::Synced => {
                let want = viewer.state_hash();
                for (slot, peer) in &self.peers {
                    if let Some(fault) = &peer.net.fault {
                        return Err(format!("slot {slot}: {fault}").into());
                    }
                    if let Some(n) = peer.net.desync {
                        return Err(format!("slot {slot} desynchronised at command {n}").into());
                    }
                    let got = peer.state_hash();
                    if got != want || peer.net.log.len() != viewer.net.log.len() {
                        return Err(format!(
                            "slot {slot}: hash {got:016x} after {} commands, host {want:016x} after {}",
                            peer.net.log.len(),
                            viewer.net.log.len()
                        )
                        .into());
                    }
                }
                if let Some(fault) = &viewer.net.fault {
                    return Err(fault.clone().into());
                }
            }
            Command::Quit => return Ok(false),
        }
        if let Some(on) = viewer.fullscreen.take()
            && self.visible
        {
            self.canvas.window_mut().set_fullscreen(on)?;
        }
        Ok(!viewer.quit)
    }

    /// Whether no copy of the game has anything playing or to run.
    fn quiet(&mut self, viewer: &mut Viewer) -> bool {
        std::iter::once(viewer)
            .chain(self.peers.iter_mut().map(|(_, v)| v))
            .all(|v| {
                v.receive();
                !(v.busy() || v.flow.ai_until.is_some() || v.pending())
            })
    }

    /// Advances the game, and every copy joined to it, by one frame.
    fn step(&mut self, viewer: &mut Viewer) -> Result {
        let frame = self.frame;
        for v in std::iter::once(&mut *viewer).chain(self.peers.iter_mut().map(|(_, v)| v)) {
            // At the instant the support started, so its timing is unchanged.
            crate::moves::preload_support(self.gfx, v);
            v.clock.advance(frame);
            if v.cutscene.is_some() {
                v.video_tick();
            } else {
                v.tick();
                v.battle_tick();
            }
        }
        self.show(viewer)?;
        if self.visible {
            std::thread::sleep(self.frame);
        }
        Ok(())
    }

    /// Redraws the window of a visible run.
    fn show(&mut self, viewer: &Viewer) -> Result {
        if self.visible {
            for event in self.events.poll_iter() {
                if let Event::Quit { .. } = event {
                    return Err("window closed".into());
                }
            }
            crate::render::render(self.canvas, self.gfx, viewer)?;
            self.canvas.window_mut().set_title(&viewer.title())?;
            self.canvas.present();
        }
        Ok(())
    }

    fn screenshot(&mut self, viewer: &mut Viewer, path: &Path) -> Result {
        crate::moves::preload_support(self.gfx, viewer);
        crate::render::render(self.canvas, self.gfx, viewer)?;
        let surface = self
            .canvas
            .read_pixels(None)?
            .convert_format(PixelFormat::RGBA32)?;
        let (w, h) = surface.size();
        let pitch = surface.pitch() as usize;
        let rgba = surface.with_lock(|bytes| {
            (0..h as usize)
                .flat_map(|y| &bytes[y * pitch..y * pitch + w as usize * 4])
                .copied()
                .collect::<Vec<u8>>()
        });
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let file = std::io::BufWriter::new(std::fs::File::create(path)?);
        let mut encoder = png::Encoder::new(file, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&rgba)?;
        Ok(())
    }
}

/// Clicks output pixel `at` with the left (or right) button.
fn click(viewer: &mut Viewer, (x, y): (f32, f32), right: bool) {
    let d = viewer.density;
    let (x, y) = (x / d, y / d);
    let button = if right {
        MouseButton::Right
    } else {
        MouseButton::Left
    };
    viewer.mouse_motion(x, y);
    viewer.mouse_down(button, x, y);
    viewer.mouse_up(x, y);
}

/// Moves the mouse over map tile `(x, y)`, centring the view on it first if
/// it is not well inside the viewport; returns the output pixel.
fn tile_point(viewer: &mut Viewer, x: i64, y: i64) -> std::result::Result<(f32, f32), String> {
    let size = viewer.map.battle.scenario.size;
    let (cx, cy) = tile_center(size, x, y);
    let point = |v: &Viewer| {
        (
            (cx as f32 - v.camera.0) * v.zoom,
            (cy as f32 - v.camera.1) * v.zoom,
        )
    };
    let v = viewer.layout().viewport();
    let margin = 64.0 * viewer.zoom;
    let (px, py) = point(viewer);
    let inside = px >= v.x as f32 + margin
        && py >= v.y as f32 + margin
        && px < (v.x + v.w) as f32 - margin
        && py < (v.y + v.h) as f32 - margin;
    if !inside {
        viewer.centre_on_tile(x as f64 + 0.5, y as f64 + 0.5);
    }
    let at = point(viewer);
    let d = viewer.density;
    viewer.mouse_motion(at.0 / d, at.1 / d);
    if viewer.hover != Some((x, y)) {
        return Err(format!("tile {x},{y} is not on the map view"));
    }
    Ok(at)
}

/// The game state scripts can dump and check.
pub fn state(viewer: &Viewer) -> Value {
    let battle = &viewer.map.battle;
    let scenario = &battle.scenario;
    let units: Vec<Value> = scenario
        .units
        .iter()
        .enumerate()
        .map(|(i, u)| {
            let kind = viewer.defs.unit_tech(u.kind, u.tech);
            json!({
                "index": i,
                "id": u.id,
                "name": kind.map(|k| k.name.as_str()),
                "kind": u.kind,
                "tech": u.tech,
                "owner": u.owner,
                "x": u.x,
                "y": u.y,
                "facing": u.facing,
                "ap": viewer.action_points(i).map(|a| a.0),
                "max_ap": kind.map(|k| k.action_points),
                "neg_ap": u.neg_ap,
                "armor": u.armor.or(kind.map(|k| k.armor)),
                "arrival": u.arrival,
                "order": u.order.id,
                "targets": scenario.order_targets.iter().filter(|t| t.unit == u.id).count(),
                "overwatch": u.overwatch,
                "dimmed": viewer.options.dimmed_units.contains(&i),
            })
        })
        .collect();
    let preview = viewer.preview.as_ref().map(|p| {
        json!({
            "unit": p.unit,
            "target": [p.target.0, p.target.1],
            "affordable": p.affordable,
            "steps": p.steps.iter().map(|s| json!([s.x, s.y, s.cost, s.facing])).collect::<Vec<_>>(),
        })
    });
    let motion = viewer.motions.first().map(|m| {
        json!({
            "unit": m.unit,
            "done": m.done,
            "steps": m.steps.len(),
            "lead": m.lead,
            "walk": m.walk,
            "landing": m.landing.is_some(),
            "positions": m.positions.iter().map(|((x, y), s)| json!([x, y, s])).collect::<Vec<_>>(),
        })
    });
    let lock = viewer.lock.as_ref().map(|l| {
        json!({
            "unit": l.unit,
            "target": format!("{:?}", l.target),
            "pending": l.pending,
            "once": l.once,
        })
    });
    let shot = viewer.shot.as_ref().map(|s| {
        json!({
            "unit": s.unit,
            "target": format!("{:?}", s.target),
            "aim": [s.aim.0, s.aim.1],
            "lands": [s.lands.0, s.lands.1],
            "frame": s.frame(viewer.clock.now()),
            "begin": s.begin,
            "shoot": s.shoot,
            "end": s.end,
            "launch": s.launch,
            "path": s.path.iter().map(|(x, y)| json!([x, y])).collect::<Vec<_>>(),
            "impact_start": s.impact_start,
            "impact_len": s.impact_len,
            "total": s.total,
            "hit": s.hit,
            "damage": s.damage,
            "repeat": s.repeat,
            "explosions": s.explosions.iter().map(|e| json!([e.name, e.first, e.len])).collect::<Vec<_>>(),
            "ruins": s.ruins.iter().map(|r| json!([r.id, r.x, r.y])).collect::<Vec<_>>(),
            "gunners": s.gunners.iter().map(|g| json!([g.unit, g.name, g.start, g.len])).collect::<Vec<_>>(),
        })
    });
    let buildings: Vec<Value> = scenario
        .buildings
        .iter()
        .map(|b| json!({"kind": b.kind, "owner": b.owner, "x": b.x, "y": b.y, "armor": b.armor}))
        .collect();
    let mines: Vec<Value> = scenario
        .mines
        .iter()
        .map(|m| json!({"kind": m.kind, "owner": m.owner, "x": m.x, "y": m.y, "seen": !m.hidden_to(viewer.seats.local)}))
        .collect();
    let ruins: Vec<Value> = scenario
        .ruins
        .iter()
        .map(|r| json!([r.id, r.x, r.y]))
        .collect();
    let setup = viewer.flow.setup.as_ref().map(|s| {
        json!({
            "points": s.points,
            "units": s.units,
            "tech": s.tech,
            "row": s.row,
            "deployed": s.deployed,
        })
    });
    let zone: Vec<Value> = scenario
        .deployments
        .iter()
        .filter(|d| viewer.zone(d.x, d.y))
        .map(|d| json!([d.x, d.y]))
        .collect();
    let objective = scenario
        .objective
        .as_ref()
        .map(|o| json!({"kind": o.kind, "turn_number": o.turn_number}));
    let report =
        viewer.flow.report.as_ref().map(
            |r| json!({"title": r.title, "message": r.message, "rows": r.rows, "tech": r.tech}),
        );
    let players: Vec<Value> = battle
        .players
        .iter()
        .map(|p| json!({"player": p.player, "totals": p.totals(), "rank": p.rank(&viewer.defs)}))
        .collect();
    let automated = json!({
        "list": viewer.support_list(),
        "armed": viewer.armed_support().map(|a| a.id),
        "pickup": viewer.flow.pickup.map(|(x, y)| [x, y]),
        "legs": viewer.supports.len(),
        "ai_budget": viewer.flow.ai_budget,
        "scripted": scenario
            .info
            .supports
            .iter()
            .map(|s| json!({"kind": s.kind, "turn": s.turn, "at": [s.start.0, s.start.1]}))
            .collect::<Vec<_>>(),
        "bridge_sites": libday_view::support::bridge_sites(scenario, &viewer.defs)
            .iter()
            .map(|&(x, y)| {
                let free = libday_view::support::bridgeable(scenario, &viewer.defs, (x, y));
                json!([x, y, free])
            })
            .collect::<Vec<_>>(),
    });
    let inspect = viewer.info.map(|t| {
        let card =
            libday_view::inspect::card_named(scenario, &viewer.defs, t, &|o| viewer.player_name(o));
        json!({"kind": t.name(), "index": t.index(), "lines": card.map(|c| c.lines)})
    });
    let mut state = json!({
        "time_ms": viewer.clock.now().as_millis() as u64,
        "screen": variant(&viewer.screen),
        "phase": variant(&viewer.flow.phase),
        "map": viewer.map.file,
        "title": scenario.info.title,
        "size": scenario.size,
        "turn": battle.clock.turn,
        "player": battle.clock.player,
        "credits": viewer.credits(),
        "reinforcing": viewer.flow.reinforcing,
        "message": viewer.flow.message,
        "error": viewer.error,
        "skirmish": viewer.flow.pick,
        "setup": setup,
        "selected": viewer.selected,
        "hover": viewer.hover.map(|(x, y)| [x, y]),
        "cursor": viewer.battle_cursor().map(|(_, name)| name),
        "fire": viewer.hover_fire().map(|f| f.code()),
        "fire_mode": viewer.fire_mode_on(),
        "zoom": viewer.zoom / viewer.density,
        "camera": [viewer.camera.0, viewer.camera.1],
        "preview": preview,
        "motion": motion,
        "lock": lock,
        "shot": shot,
        "flights": viewer.flights.len(),
        "reaction": viewer.reaction.as_ref().map(|r| r.target),
        "info": viewer.info.and_then(|t| t.unit()),
        "watch": viewer.watch.iter().map(|((x, y), _)| [*x, *y]).collect::<Vec<_>>(),
        "zone": zone,
        "units": units,
        "buildings": buildings,
        "ruins": ruins,
        "mines": mines,
        // `terrain.Y.X`: the artwork of each tile, row by row.
        "terrain": scenario
            .cells
            .chunks(scenario.size.max(1))
            .map(|row| row.iter().map(|c| c.terrain.as_str()).collect::<Vec<_>>())
            .collect::<Vec<_>>(),
        "sounds": viewer.sound.log,
        "outcome": viewer.flow.outcome,
        "objective": objective,
        "report": report,
        "players": players,
        "menu": menu_state(viewer),
    });
    state["local"] = json!(viewer.seats.local);
    state["automated"] = automated;
    state["hash"] = json!(format!("{:016x}", viewer.state_hash()));
    state["desync"] = json!(viewer.net.desync);
    state["reinf_res"] = json!(scenario.info.reinf_res);
    state["hidden"] = json!(viewer.options.hidden_units.len());
    state["front"] = json!({
        "name": viewer.front.name,
        "difficulty": viewer.front.difficulty,
    });
    state["notice"] = json!(viewer.notice);
    state["online"] = viewer.net.online.as_ref().map_or(Value::Null, |o| {
        json!({
            "slot": o.slot,
            "host": o.is_host(),
            "started": o.started,
            "peers": viewer.net.peers.len(),
            "names": o.names.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
            "chat": o.chat.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
            "left": o.left,
            "status": viewer.front.status,
            "fault": viewer.net.fault,
            "commands": viewer.net.log.len(),
        })
    });
    state["video"] = json!(viewer.cutscene.as_ref().map(|c| &c.name));
    state["campaign"] = viewer.campaign.as_ref().map_or(Value::Null, |c| {
        serde_json::to_value(&c.state).unwrap_or(Value::Null)
    });
    state["strategic"] = viewer.strategic.as_ref().map_or(
        Value::Null,
        |st| json!({ "selected": st.selected, "text": st.text, "victory": st.victory }),
    );
    if let Some(st) = &viewer.strategic {
        state["strategic"]["intel"] = st.intel.as_ref().map_or(Value::Null, |intel| {
            json!({
                "province": intel.province,
                "uplink": intel.uplink,
                "preview": intel.preview.as_ref().map(|i| [i.width, i.height]),
                "text": viewer.intel_text(intel),
            })
        });
    }
    // `roads.N` / `walls.N`: `[kind, x, y, owner]`.
    let surfaces = |list: &[libday_data::scenario::Surface]| {
        list.iter()
            .map(|r| [r.kind, r.x, r.y, r.owner])
            .collect::<Vec<_>>()
    };
    state["roads"] = json!(surfaces(&scenario.roads));
    state["walls"] = json!(surfaces(&scenario.walls));
    state["base_editor"] = viewer.base_editor.as_ref().map_or(Value::Null, |e| {
        json!({
            "mode": format!("{:?}", e.mode),
            "defenses": e.defenses,
            "selected": e.selected.map(|i| i.name(&viewer.defs)),
            "list": viewer
                .base_list()
                .iter()
                .map(|i| json!([i.name(&viewer.defs), i.cost(&viewer.defs)]))
                .collect::<Vec<_>>(),
        })
    });
    state["selection"] = json!(viewer.selection());
    state["grouping"] = json!(viewer.grouping);
    state["walking"] = json!(viewer.motions.iter().map(|m| m.unit).collect::<Vec<_>>());
    state["inspect"] = json!(inspect);
    // The tile at the middle of the battle view.
    let v = viewer.layout().viewport();
    let (mx, my) = viewer.to_map(v.w as f32 / 2.0, v.h as f32 / 2.0);
    let (tx, ty) = libday_view::tile_coords(viewer.map.battle.scenario.size, mx as f64, my as f64);
    state["view"] = json!([tx.floor() as i64, ty.floor() as i64]);
    // `tech.buildings`: every technology building and its reward;
    // `tech.captured`: those the player's units hold; `tech.found`: their
    // total.
    state["tech"] = json!({
        "buildings": scenario
            .buildings
            .iter()
            .filter_map(|b| libday_view::tech::reward(&viewer.defs, b).map(|r| json!([b.id, r])))
            .collect::<Vec<_>>(),
        "captured": viewer.flow.tech,
        "found": viewer.tech_found(),
    });
    state
}

/// The OPTIONS table: `values.NAME` (true/false or the selector's text).
fn menu_state(viewer: &Viewer) -> Value {
    use crate::menu::{ITEMS, Value as Shown};
    let values: serde_json::Map<String, Value> = ITEMS
        .into_iter()
        .filter_map(|item| {
            let value = match viewer.option_value(item)? {
                Shown::Check(on) => json!(on),
                Shown::Choice(text, ..) => json!(text),
            };
            Some((item.name(), value))
        })
        .collect();
    json!({
        "values": values,
        "scroll": viewer.menu.scroll,
        "focus": viewer.menu.focus.map(|i| ITEMS[i].name()),
        "over_main": viewer.menu.over_main,
        "question": viewer.menu.question,
        "volume": viewer.settings.volume,
        "scroll_speed": viewer.settings.scroll.name(),
        "move_speed": viewer.settings.moves.name(),
    })
}

/// An enum's variant name without its fields (`Tech { .. }` -> `Tech`).
fn variant(value: &impl std::fmt::Debug) -> String {
    let name = format!("{value:?}");
    let end = name.find([' ', '(', '{']).unwrap_or(name.len());
    name[..end].to_string()
}

/// The value at a dotted `path`; an `id=N` segment picks the array element
/// whose `id` is `N`, a negative index counts from the end.
/// The state at `path` as text (strings unquoted).
fn state_at(viewer: &Viewer, path: &str) -> Option<String> {
    let state = state(viewer);
    Some(match lookup(&state, path)? {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn lookup<'v>(mut value: &'v Value, path: &str) -> Option<&'v Value> {
    for segment in path.split('.') {
        value = match value {
            Value::Array(items) => match segment.strip_prefix("id=") {
                Some(id) => {
                    let id: i64 = id.parse().ok()?;
                    items
                        .iter()
                        .find(|e| e.get("id").and_then(Value::as_i64) == Some(id))?
                }
                None => {
                    let i: i64 = segment.parse().ok()?;
                    let i = if i < 0 { items.len() as i64 + i } else { i };
                    items.get(usize::try_from(i).ok()?)?
                }
            },
            Value::Object(map) => map.get(segment)?,
            _ => return None,
        };
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_and_reports_lines() {
        let lines = parse(
            "# setup\nskirmish Opportunity Knocks\nkey Return\n\ntile 44 18\n\
             click 10 20 right\nrecord out\\walk 12\nexpect units.id=7.facing 2\nstate\n",
        )
        .unwrap();
        assert_eq!(
            lines,
            vec![
                (2, Command::Skirmish("Opportunity Knocks".into())),
                (3, Command::Key("Return".into())),
                (
                    5,
                    Command::Tile {
                        x: 44,
                        y: 18,
                        right: false
                    }
                ),
                (
                    6,
                    Command::Click {
                        x: 10,
                        y: 20,
                        right: true
                    }
                ),
                (7, Command::Record("out\\walk".into(), 12)),
                (8, Command::Expect("units.id=7.facing".into(), "2".into())),
                (9, Command::State(None)),
            ]
        );
        assert_eq!(
            parse("wait 2\nfly 3\n").unwrap_err(),
            "line 2: unknown command fly: fly 3"
        );
        assert!(parse("tile 1\n").is_err());
        assert!(parse("fps 0\n").is_err());
    }

    #[test]
    fn looks_up_dotted_paths() {
        let state = json!({"units": [{"id": 3, "x": 1}, {"id": 7, "x": 5}], "screen": "Deploy"});
        assert_eq!(lookup(&state, "units.1.x"), Some(&json!(5)));
        assert_eq!(lookup(&state, "units.id=3.x"), Some(&json!(1)));
        assert_eq!(lookup(&state, "units.-1.id"), Some(&json!(7)));
        assert_eq!(lookup(&state, "units.-3.id"), None);
        assert_eq!(lookup(&state, "screen"), Some(&json!("Deploy")));
        assert_eq!(lookup(&state, "units.id=9.x"), None);
    }
}
