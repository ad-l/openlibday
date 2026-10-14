//! Data-gated checks of the battle interface artwork and the minimap.
//! Skipped (with a note) when `LIBDAY_ROOT` is not set.

use libday_data::GameData;
use libday_view::ui::{Button, State, frame};
use libday_view::{Art, minimap};

fn game() -> Option<GameData> {
    match GameData::from_env() {
        Ok(game) => Some(game),
        Err(err) => {
            eprintln!("skipping: {err}");
            None
        }
    }
}

fn pixel(image: &libday_view::Image, x: usize, y: usize) -> [u8; 4] {
    let at = (y * image.width + x) * 4;
    image.rgba[at..at + 4].try_into().unwrap()
}

#[test]
fn interface_elements_decode() {
    let Some(game) = game() else { return };
    let ui = libday_view::load_ui(&game).unwrap();
    let sidebar = ui.get(frame::SIDEBAR).unwrap();
    assert_eq!((sidebar.width, sidebar.height), (136, 768));
    // The minimap well is black and opaque.
    assert_eq!(pixel(sidebar, 60, 100), [0, 0, 0, 255]);
    let goal = ui.get(Button::Goal.frame()).unwrap();
    assert_eq!((goal.width, goal.height), (53, 17));
    assert!(goal.rgba.chunks(4).all(|p| p[3] == 255));
    // Toggle box colours: off is red, on is green.
    let off = ui
        .get(Button::Overwatch.frame_for(State::Normal, false))
        .unwrap();
    let on = ui
        .get(Button::Overwatch.frame_for(State::Normal, true))
        .unwrap();
    let (r, g) = (pixel(off, 9, 9), pixel(on, 9, 9));
    assert!(r[0] > 200 && r[1] < 60, "{r:?}");
    assert!(g[1] > 200 && g[0] < 60, "{g:?}");
    for f in [
        frame::BOTTOM_BAR,
        frame::GOAL_DIALOG,
        frame::OVERVIEW_SIDEBAR,
        696,
    ] {
        assert!(ui.get(f).is_some(), "frame {f}");
    }
}

#[test]
fn minimap_of_a_skirmish() {
    let Some(game) = game() else { return };
    let defs = game.definitions().unwrap();
    let art = Art::load(&game, &defs).unwrap();
    let file = game.skirmishes().unwrap()[0].file_name.clone();
    let scenario = game.load_scenario(&file).unwrap();
    let scene = libday_view::build(&scenario, &defs, &art, &Default::default());
    let colours = minimap::tile_colours(&scenario, &defs);
    assert_eq!(colours.width, scenario.size);
    let top = minimap::top_down(&scene, &art, 4);
    assert_eq!(top.width, scenario.size * 4);
    // Every tile has terrain, so the picture has no black holes.
    let black = top.rgba.chunks(4).filter(|p| p[..3] == [0, 0, 0]).count();
    assert!(black < top.rgba.len() / 4 / 50, "{black} black pixels");
    let markers = minimap::markers(&scenario, &defs);
    let players = minimap::players(&scenario, &markers);
    assert_eq!(players.last(), Some(&0));
}
