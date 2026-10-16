//! Base building between campaign battles: the BUILDINGS and DEFENSES
//! lists, building, MOVE and RECYCLE, and CAMPAIGN back to the continent.

use std::collections::HashSet;

use libday_data::Scenario;
use libday_data::scenario::{Building, Mine, Surface};
use libday_view::Scene;
use libday_view::base::{self, Item, Refusal};
use libday_view::support::building_at;
use libday_view::ui::{BASE_ROWS, Button, Screen};

use crate::Viewer;
use crate::campaign::BaseLayout;
use crate::players::CAMPAIGN_PLAYER;

/// String ids (`LIBDAYEN.DLL`).
const CANT_MOVE_HQ: u32 = 391;
const CANT_RECYCLE_HQ: u32 = 392;
const NOT_YOURS: u32 = 400;
const RECYCLE_WARNING: u32 = 401;
const BUILT: u32 = 403;
const RECYCLED: u32 = 404;
const MOVED: u32 = 405;
const BUILT_FOR: u32 = 406;
const START: u32 = 407;
const END: u32 = 408;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Build,
    Move,
    Recycle,
}

/// A structure of the base.
#[derive(Clone, Debug)]
enum Thing {
    Building(Building),
    Road(Surface),
    Wall(Surface),
    Mine(Mine),
}

/// Identifies a structure across moves: a building by id, the others by
/// type and tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Building(i64),
    Tile(Item, (i64, i64)),
}

impl Thing {
    fn item(&self) -> Item {
        match self {
            Thing::Building(b) => Item::Building(b.kind),
            Thing::Road(r) => Item::Road(r.kind),
            Thing::Wall(w) => Item::Wall(w.kind),
            Thing::Mine(m) => Item::Mine(m.kind),
        }
    }

    fn at(&self) -> (i64, i64) {
        match self {
            Thing::Building(b) => (b.x, b.y),
            Thing::Road(r) | Thing::Wall(r) => (r.x, r.y),
            Thing::Mine(m) => (m.x, m.y),
        }
    }

    fn owner(&self) -> i64 {
        match self {
            Thing::Building(b) => b.owner,
            Thing::Road(r) | Thing::Wall(r) => r.owner,
            Thing::Mine(m) => m.owner,
        }
    }

    fn key(&self) -> Key {
        match self {
            Thing::Building(b) => Key::Building(b.id),
            _ => Key::Tile(self.item(), self.at()),
        }
    }

    fn moved(mut self, (x, y): (i64, i64)) -> Self {
        match &mut self {
            Thing::Building(b) => (b.x, b.y) = (x, y),
            Thing::Road(r) | Thing::Wall(r) => (r.x, r.y) = (x, y),
            Thing::Mine(m) => (m.x, m.y) = (x, y),
        }
        self
    }

    fn add(self, s: &mut Scenario) {
        match self {
            Thing::Building(b) => s.buildings.push(b),
            Thing::Road(r) => s.roads.push(r),
            Thing::Wall(w) => s.walls.push(w),
            Thing::Mine(m) => s.mines.push(m),
        }
    }
}

/// Where a click would put things, and whether each may go there.
struct Plan {
    scenario: Scenario,
    things: Vec<(Thing, Result<(), Refusal>)>,
}

pub struct BaseEditor {
    /// DEFENSES (else BUILDINGS) is listed.
    pub defenses: bool,
    pub scroll: usize,
    pub selected: Option<Item>,
    pub mode: Mode,
    /// The structure MOVE lifted, and whether it was built this session.
    picked: Option<(Thing, bool)>,
    /// A road or wall segment's first tile.
    start: Option<(i64, i64)>,
    /// Built this session: recycled for their full cost.
    placed: Vec<Key>,
    /// Recycling asked for confirmation (a structure of an earlier session).
    confirm: Option<Key>,
    /// The shipped base map, for [`BaseLayout::capture`].
    original: Option<Scenario>,
    /// The scene shows a placement preview.
    ghosted: bool,
}

impl Viewer {
    /// Base building on the base map (`CBaseEditor`).
    pub(crate) fn open_base_editor(&mut self) {
        let Some(battle) = self.load_base(false) else {
            return;
        };
        let original = battle.source.load(&self.game).ok();
        self.show_campaign_map(battle);
        self.base_editor = Some(BaseEditor {
            defenses: false,
            scroll: 0,
            selected: None,
            mode: Mode::Build,
            picked: None,
            start: None,
            placed: Vec::new(),
            confirm: None,
            original,
            ghosted: false,
        });
        let s = &self.map.battle.scenario;
        let hq = s
            .buildings
            .iter()
            .find(|b| b.owner == CAMPAIGN_PLAYER && b.kind == base::HQ);
        if let Some(b) = hq {
            let (w, h) = Item::Building(b.kind).size(&self.defs);
            let centre = (b.x as f64 + w as f64 / 2.0, b.y as f64 + h as f64 / 2.0);
            self.centre_on_tile(centre.0, centre.1);
        }
        self.screen = self.home();
    }

    /// CAMPAIGN: keeps the base and returns to the continent overview.
    fn close_base_editor(&mut self) {
        self.base_put_back();
        let layout = self.base_editor.as_ref().and_then(|e| {
            let original = e.original.as_ref()?;
            Some(BaseLayout::capture(
                &self.map.battle.scenario,
                original,
                &self.defs,
            ))
        });
        if let (Some(session), Some(layout)) = (&mut self.campaign, layout) {
            session.state.base = Some(layout);
        }
        self.base_editor = None;
        self.save_campaign();
        self.open_strategic();
    }

    /// What the shown list offers.
    pub(crate) fn base_list(&self) -> Vec<Item> {
        let (Some(e), Some(session)) = (&self.base_editor, &self.campaign) else {
            return Vec::new();
        };
        let available = session
            .continent()
            .map(|c| c.buildings.clone())
            .unwrap_or_default();
        let owned = self.owned_buildings(&self.map.battle.scenario);
        base::offered(&self.defs, &available, &owned, e.defenses)
    }

    pub(crate) fn base_enabled(&self, button: Button) -> bool {
        let Some(e) = &self.base_editor else {
            return false;
        };
        match button {
            Button::BaseUp => e.scroll > 0,
            Button::BaseDown => e.scroll + BASE_ROWS < self.base_list().len(),
            _ => true,
        }
    }

    pub(crate) fn base_activate(&mut self, button: Button) {
        if button == Button::BaseMap {
            self.screen = Screen::Overview;
            return;
        }
        if button == Button::Campaign {
            self.close_base_editor();
            return;
        }
        if button == Button::BaseArmy {
            self.base_put_back();
            self.open_army();
            return;
        }
        self.base_put_back();
        let rows = self.base_list().len();
        let Some(e) = &mut self.base_editor else {
            return;
        };
        e.start = None;
        e.confirm = None;
        match button {
            Button::BaseBuildings | Button::BaseDefenses => {
                e.defenses = button == Button::BaseDefenses;
                e.scroll = 0;
                e.selected = None;
                e.mode = Mode::Build;
            }
            Button::BaseUp => e.scroll = e.scroll.saturating_sub(1),
            Button::BaseDown => e.scroll = (e.scroll + 1).min(rows.saturating_sub(BASE_ROWS)),
            Button::Recycle | Button::BaseMove => {
                let mode = if button == Button::Recycle {
                    Mode::Recycle
                } else {
                    Mode::Move
                };
                e.mode = if e.mode == mode { Mode::Build } else { mode };
            }
            _ => {}
        }
        self.flow.message = None;
        self.base_refresh();
    }

    /// Selects row `row` of the list (on screen).
    pub(crate) fn base_select_row(&mut self, row: usize) {
        let list = self.base_list();
        self.base_put_back();
        let Some(e) = &mut self.base_editor else {
            return;
        };
        if let Some(&item) = list.get(e.scroll + row) {
            e.selected = Some(item);
            e.mode = Mode::Build;
            e.start = None;
            e.confirm = None;
        }
        self.flow.message = self.base_prompt();
        self.base_refresh();
    }

    /// Scrolls the list by `rows`.
    pub(crate) fn base_scroll(&mut self, rows: isize) {
        let last = self.base_list().len().saturating_sub(BASE_ROWS);
        if let Some(e) = &mut self.base_editor {
            e.scroll = e.scroll.saturating_add_signed(rows).min(last);
        }
    }

    /// Selects the listed item named `name` (scripts), from either list.
    pub(crate) fn base_select_named(&mut self, name: &str) -> bool {
        for defenses in [false, true] {
            if let Some(e) = &mut self.base_editor {
                e.defenses = defenses;
                e.scroll = 0;
            }
            let list = self.base_list();
            if let Some(i) = list
                .iter()
                .position(|it| it.name(&self.defs).eq_ignore_ascii_case(name))
            {
                if let Some(e) = &mut self.base_editor {
                    e.scroll = i.saturating_sub(BASE_ROWS - 1);
                }
                let row = i - self.base_editor.as_ref().map_or(0, |e| e.scroll);
                self.base_select_row(row);
                return true;
            }
        }
        false
    }

    /// Escape: drops a lifted structure or a segment's start, else the menu.
    pub(crate) fn base_escape(&mut self) {
        let busy = self
            .base_editor
            .as_ref()
            .is_some_and(|e| e.picked.is_some() || e.start.is_some());
        if !busy {
            self.open_menu();
            return;
        }
        self.base_put_back();
        if let Some(e) = &mut self.base_editor {
            e.start = None;
        }
        self.flow.message = None;
        self.base_refresh();
    }

    /// Returns a lifted structure to where it was.
    fn base_put_back(&mut self) {
        let Some((thing, fresh)) = self.base_editor.as_mut().and_then(|e| e.picked.take()) else {
            return;
        };
        if fresh && let Some(e) = &mut self.base_editor {
            e.placed.push(thing.key());
        }
        thing.add(&mut self.map.battle.scenario);
        self.rebuild();
    }

    fn base_message(&mut self, id: u32, fallback: &str, name: &str, credits: i64) {
        let text = self
            .string(id, fallback)
            .replace("%1!d!", &credits.to_string())
            .replace("%2!d!", &credits.to_string())
            .replace("%1", name);
        self.flow.message = Some(text);
    }

    fn refusal(&mut self, r: Refusal) {
        let text = self.string(r.string(), r.fallback());
        self.flow.message = Some(text);
    }

    /// A new structure of the player's.
    fn new_thing(&self, s: &Scenario, item: Item, (x, y): (i64, i64)) -> Thing {
        let surface = |list: &[Surface], kind: i64, armor: i64| Surface {
            counter: list.iter().map(|r| r.counter + 1).max().unwrap_or(1),
            kind,
            owner: CAMPAIGN_PLAYER,
            x,
            y,
            armor,
            broken: false,
        };
        match item {
            Item::Building(kind) => Thing::Building(Building {
                counter: s.buildings.iter().map(|b| b.counter + 1).max().unwrap_or(1),
                id: s.buildings.iter().map(|b| b.id + 1).max().unwrap_or(1),
                kind,
                owner: CAMPAIGN_PLAYER,
                armor: self.defs.buildings.get(&kind).map_or(1, |b| b.armor),
                x,
                y,
                ai_target: false,
            }),
            Item::Road(kind) => Thing::Road(surface(
                &s.roads,
                kind,
                self.defs.roads.get(&kind).map_or(1, |r| r.armor),
            )),
            Item::Wall(kind) => Thing::Wall(surface(
                &s.walls,
                kind,
                self.defs.walls.get(&kind).map_or(1, |r| r.armor),
            )),
            Item::Mine(kind) => Thing::Mine(Mine {
                counter: s.mines.iter().map(|m| m.counter + 1).max().unwrap_or(1),
                kind,
                owner: CAMPAIGN_PLAYER,
                x,
                y,
                armed: true,
                // Hidden from the AI, seen by the player.
                hidden_ai: true,
                hidden: [false; 4],
            }),
        }
    }

    /// Whether `item` may be built on `at` of `s` with `credits` left.
    fn buildable(
        &self,
        s: &Scenario,
        item: Item,
        at: (i64, i64),
        credits: i64,
    ) -> Result<(), Refusal> {
        base::check(s, &self.defs, CAMPAIGN_PLAYER, item, at)?;
        if let Item::Building(kind) = item
            && base::DEPLOY_CENTRES.contains(&kind)
            && s.buildings
                .iter()
                .filter(|b| b.owner == CAMPAIGN_PLAYER && base::DEPLOY_CENTRES.contains(&b.kind))
                .count()
                >= base::MAX_DEPLOY_CENTRES
        {
            return Err(Refusal::DeployCentres);
        }
        if item.cost(&self.defs) > credits {
            return Err(Refusal::Credits);
        }
        Ok(())
    }

    /// What a click on `at` would place: the lifted structure, or the
    /// selected item (a segment from its start, stopping at the first tile
    /// refused).
    fn base_plan(&self, at: (i64, i64)) -> Option<Plan> {
        let e = self.base_editor.as_ref()?;
        let mut scenario = self.map.battle.scenario.clone();
        if let Some((thing, _)) = &e.picked {
            let item = thing.item();
            let r = base::check(&scenario, &self.defs, CAMPAIGN_PLAYER, item, at);
            let thing = thing.clone().moved(at);
            thing.clone().add(&mut scenario);
            return Some(Plan {
                scenario,
                things: vec![(thing, r)],
            });
        }
        if e.mode != Mode::Build {
            return None;
        }
        let item = e.selected?;
        let tiles = match e.start {
            Some(start) if item.segment() => base::segment(start, at),
            _ => vec![at],
        };
        let mut credits = self.credits();
        let mut refused = None;
        let mut things = Vec::new();
        for t in tiles {
            let r = match refused {
                Some(r) => Err(r),
                None => self.buildable(&scenario, item, t, credits),
            };
            match r {
                Ok(()) => credits -= item.cost(&self.defs),
                Err(r) => refused = Some(r),
            }
            let thing = self.new_thing(&scenario, item, t);
            thing.clone().add(&mut scenario);
            things.push((thing, r));
        }
        Some(Plan { scenario, things })
    }

    /// A click on map tile `at`.
    pub(crate) fn base_click(&mut self, x: i64, y: i64) {
        let at = (x, y);
        let Some(e) = &self.base_editor else {
            return;
        };
        let (mode, picked) = (e.mode, e.picked.is_some());
        if picked {
            self.base_drop(at);
        } else {
            match mode {
                Mode::Build => self.base_build(at),
                Mode::Move => self.base_lift(at),
                Mode::Recycle => self.base_recycle(at),
            }
        }
        self.base_refresh();
    }

    fn base_build(&mut self, at: (i64, i64)) {
        let Some(e) = &self.base_editor else {
            return;
        };
        let Some(item) = e.selected else {
            return;
        };
        if item.segment() && e.start.is_none() {
            match self.buildable(&self.map.battle.scenario, item, at, self.credits()) {
                Ok(()) => {
                    if let Some(e) = &mut self.base_editor {
                        e.start = Some(at);
                    }
                    self.base_message(END, "Click on ending point of segment", "", 0);
                }
                Err(r) => self.refusal(r),
            }
            return;
        }
        let Some(plan) = self.base_plan(at) else {
            return;
        };
        if let Some(e) = &mut self.base_editor {
            e.start = None;
        }
        let built: Vec<Thing> = plan
            .things
            .iter()
            .take_while(|(_, r)| r.is_ok())
            .map(|(t, _)| t.clone())
            .collect();
        if built.is_empty() {
            if let Some(&(_, Err(r))) = plan.things.first().as_ref().map(|(t, r)| (t, *r)).as_ref()
            {
                self.refusal(r);
            }
            return;
        }
        let cost = item.cost(&self.defs) * built.len() as i64;
        *self.credits_mut() -= cost;
        for thing in built {
            if let Some(e) = &mut self.base_editor {
                e.placed.push(thing.key());
            }
            thing.add(&mut self.map.battle.scenario);
        }
        let name = item.name(&self.defs);
        if cost > 0 {
            self.base_message(
                BUILT_FOR,
                "%1 has been built for %2!d! credits",
                &name,
                cost,
            );
        } else {
            self.base_message(BUILT, "%1 has been built", &name, 0);
        }
        self.effect(crate::sound::Effect::Build);
        self.rebuild();
    }

    /// The structure on `at` the player may move or recycle: its index in
    /// its list (mines first, then buildings, walls and roads).
    fn structure_on(&self, at: (i64, i64)) -> Option<Thing> {
        let s = &self.map.battle.scenario;
        if let Some(m) = s.mines.iter().find(|m| (m.x, m.y) == at) {
            return Some(Thing::Mine(m.clone()));
        }
        if let Some(i) = building_at(s, &self.defs, at) {
            return Some(Thing::Building(s.buildings[i].clone()));
        }
        if let Some(w) = s.walls.iter().find(|w| (w.x, w.y) == at) {
            return Some(Thing::Wall(w.clone()));
        }
        s.roads
            .iter()
            .find(|r| (r.x, r.y) == at)
            .map(|r| Thing::Road(r.clone()))
    }

    /// Removes `thing` from the map.
    fn remove(&mut self, thing: &Thing) {
        let s = &mut self.map.battle.scenario;
        let key = thing.key();
        match thing {
            Thing::Building(b) => s.buildings.retain(|o| o.id != b.id),
            Thing::Road(_) => s.roads.retain(|r| Thing::Road(r.clone()).key() != key),
            Thing::Wall(_) => s.walls.retain(|w| Thing::Wall(w.clone()).key() != key),
            Thing::Mine(_) => s.mines.retain(|m| Thing::Mine(m.clone()).key() != key),
        }
        if let Some(e) = &mut self.base_editor {
            e.placed.retain(|&k| k != key);
        }
    }

    /// Why the player may not move or recycle `thing`.
    fn untouchable(&mut self, thing: &Thing, hq: (u32, &str)) -> bool {
        if thing.owner() != CAMPAIGN_PLAYER {
            self.base_message(NOT_YOURS, "Can't recycle other's structure", "", 0);
            return true;
        }
        if thing.item() == Item::Building(base::HQ) {
            self.base_message(hq.0, hq.1, "", 0);
            return true;
        }
        false
    }

    fn base_lift(&mut self, at: (i64, i64)) {
        let Some(thing) = self.structure_on(at) else {
            return;
        };
        if self.untouchable(&thing, (CANT_MOVE_HQ, "Can't move HQ")) {
            return;
        }
        let fresh = self
            .base_editor
            .as_ref()
            .is_some_and(|e| e.placed.contains(&thing.key()));
        self.remove(&thing);
        if let Some(e) = &mut self.base_editor {
            e.picked = Some((thing, fresh));
        }
        self.flow.message = None;
        self.rebuild();
    }

    fn base_drop(&mut self, at: (i64, i64)) {
        let Some(plan) = self.base_plan(at) else {
            return;
        };
        let Some((thing, r)) = plan.things.into_iter().next() else {
            return;
        };
        if let Err(r) = r {
            self.refusal(r);
            return;
        }
        let Some((_, fresh)) = self.base_editor.as_mut().and_then(|e| e.picked.take()) else {
            return;
        };
        if fresh && let Some(e) = &mut self.base_editor {
            e.placed.push(thing.key());
        }
        let name = thing.item().name(&self.defs);
        thing.add(&mut self.map.battle.scenario);
        self.base_message(MOVED, "%1 has been moved", &name, 0);
        self.effect(crate::sound::Effect::Click);
        self.rebuild();
    }

    fn base_recycle(&mut self, at: (i64, i64)) {
        let Some(thing) = self.structure_on(at) else {
            return;
        };
        if self.untouchable(&thing, (CANT_RECYCLE_HQ, "Can't recycle HQ")) {
            return;
        }
        let key = thing.key();
        let Some(e) = &mut self.base_editor else {
            return;
        };
        let fresh = e.placed.contains(&key);
        if !fresh && e.confirm != Some(key) {
            // The original asks in a dialog; here a second click confirms.
            e.confirm = Some(key);
            self.base_message(
                RECYCLE_WARNING,
                "You might not get all your money back if you recycle this. Do you want to continue anyway?",
                "",
                0,
            );
            return;
        }
        e.confirm = None;
        let item = thing.item();
        let refund = if fresh {
            item.cost(&self.defs)
        } else {
            item.recycle(&self.defs)
        };
        self.remove(&thing);
        *self.credits_mut() += refund;
        let name = item.name(&self.defs);
        self.base_message(RECYCLED, "%1 has been recycled", &name, 0);
        self.effect(crate::sound::Effect::Cashin);
        self.rebuild();
    }

    /// The prompt for the first click of a segment.
    pub(crate) fn base_prompt(&self) -> Option<String> {
        let e = self.base_editor.as_ref()?;
        (e.mode == Mode::Build && e.start.is_none() && e.selected.is_some_and(|i| i.segment()))
            .then(|| self.string(START, "Click on starting point"))
    }

    /// Redraws the map with the placement under the mouse previewed, in red
    /// where it is refused.
    pub(crate) fn base_refresh(&mut self) {
        let plan = self.hover.and_then(|at| self.base_plan(at));
        let Some(plan) = plan else {
            if let Some(e) = &mut self.base_editor
                && std::mem::take(&mut e.ghosted)
            {
                self.map.scene = self.scene_of(&self.map.battle.scenario);
            }
            return;
        };
        let plain: HashSet<_> = self
            .scene_of(&self.map.battle.scenario)
            .items
            .into_iter()
            .map(|i| (i.layer, i.sprite, i.x, i.y))
            .collect();
        let mut scene = self.scene_of(&plan.scenario);
        let all = plan
            .things
            .iter()
            .any(|(t, r)| r.is_err() && matches!(t, Thing::Building(_)));
        let red: HashSet<(i64, i64)> = plan
            .things
            .iter()
            .filter(|(_, r)| r.is_err())
            .map(|(t, _)| t.at())
            .collect();
        for item in &mut scene.items {
            let new = !plain.contains(&(item.layer, item.sprite.clone(), item.x, item.y));
            if new && (all || red.contains(&(item.x, item.y))) {
                item.sprite = format!("{}{}", crate::render::RED, item.sprite);
            }
        }
        self.map.scene = scene;
        if let Some(e) = &mut self.base_editor {
            e.ghosted = true;
        }
    }

    pub(crate) fn scene_of(&self, s: &Scenario) -> Scene {
        libday_view::build(s, &self.defs, &self.art, &self.options)
    }
}
