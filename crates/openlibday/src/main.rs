//! openlibday application entry point.
//!
//! Milestone 0: open a window and draw a mosaic of terrain tiles decoded from
//! the original `TERRAINS.DBI`, proving the data → texture → screen path.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::error::Error;

use libday_data::{Dir, GameData};
use libday_formats::Dbi;
use sdl3::event::Event;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use sdl3::pixels::{Color, PixelFormat};
use sdl3::render::{BlendMode, FRect, ScaleMode, Texture, TextureCreator};
use sdl3::sys::render::SDL_LOGICAL_PRESENTATION_LETTERBOX;
use sdl3::video::WindowContext;

const LOGICAL_W: u32 = 1024;
const LOGICAL_H: u32 = 768;
const TILE_W: f32 = 54.0;
const TILE_H: f32 = 56.0;
const MAP_SIZE: i32 = 24;

fn sprite_texture<'r>(
    creator: &'r TextureCreator<WindowContext>,
    dbi: &Dbi,
    name: &str,
) -> Result<Texture<'r>, Box<dyn Error>> {
    let sprite = dbi.sprite(name)?;
    let mut texture = creator.create_texture_static(
        PixelFormat::RGBA32,
        sprite.width as u32,
        sprite.height as u32,
    )?;
    texture.update(None, &sprite.to_rgba(&dbi.palette), sprite.width * 4)?;
    texture.set_blend_mode(BlendMode::Blend);
    texture.set_scale_mode(ScaleMode::Nearest);
    Ok(texture)
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = std::env::args().nth(1);
    let game = match root {
        Some(root) => GameData::open(root)?,
        None => GameData::from_env()?,
    };
    let terrains = game.read(Dir::Imgs, "TERRAINS.DBI")?;
    let dbi = Dbi::parse(&terrains)?;

    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let window = video
        .window("openlibday", LOGICAL_W, LOGICAL_H)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    let mut canvas = window.into_canvas();
    canvas.set_logical_size(LOGICAL_W, LOGICAL_H, SDL_LOGICAL_PRESENTATION_LETTERBOX)?;
    let creator = canvas.texture_creator();

    // M0 mosaic: cycle through one terrain family (no map loading yet).
    let names: Vec<&String> = {
        let family: Vec<_> = dbi.names().iter().filter(|n| n.starts_with("AR")).collect();
        if family.is_empty() {
            dbi.names().iter().collect()
        } else {
            family
        }
    };
    let mut textures: HashMap<usize, Texture> = HashMap::new();
    let (mut ox, mut oy) = (LOGICAL_W as f32 / 2.0, 80.0);
    let mut drag: Option<(f32, f32)> = None;
    let mut events = sdl.event_pump()?;

    'running: loop {
        for event in events.poll_iter() {
            match event {
                Event::Quit { .. }
                | Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(key), ..
                } => match key {
                    Keycode::Left => ox += 54.0,
                    Keycode::Right => ox -= 54.0,
                    Keycode::Up => oy += 28.0,
                    Keycode::Down => oy -= 28.0,
                    _ => {}
                },
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    x,
                    y,
                    ..
                } => drag = Some((x, y)),
                Event::MouseButtonUp {
                    mouse_btn: MouseButton::Left,
                    ..
                } => drag = None,
                Event::MouseMotion { x, y, .. } => {
                    if let Some((px, py)) = drag {
                        ox += x - px;
                        oy += y - py;
                        drag = Some((x, y));
                    }
                }
                _ => {}
            }
        }

        canvas.set_draw_color(Color::RGB(0, 0, 0));
        canvas.clear();
        for sum in 0..(2 * MAP_SIZE - 1) {
            for x in 0..MAP_SIZE {
                let y = sum - x;
                if !(0..MAP_SIZE).contains(&y) {
                    continue;
                }
                let index = ((x * 7 + y * 13) as usize) % names.len();
                if let Entry::Vacant(slot) = textures.entry(index) {
                    slot.insert(sprite_texture(&creator, &dbi, names[index])?);
                }
                let cx = ox + (x - y) as f32 * 27.0;
                let cy = oy + (x + y) as f32 * 14.0;
                let dst = FRect::new(cx - 27.0, cy - 42.0, TILE_W, TILE_H);
                canvas.copy(&textures[&index], None, Some(dst))?;
            }
        }
        canvas.present();
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    Ok(())
}
