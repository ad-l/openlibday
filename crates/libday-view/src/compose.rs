//! Software compositing of a [`Scene`] into one RGBA image.

use std::collections::HashMap;

use crate::scene::Scene;
use crate::{Art, Sheet, map_pixels, sprite_origin, tile_center};

/// Straight RGBA8 pixels, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(width: usize, height: usize, background: [u8; 4]) -> Self {
        Self {
            width,
            height,
            rgba: background.repeat(width * height),
        }
    }

    fn put(&mut self, x: i32, y: i32, colour: [u8; 4]) {
        if x >= 0 && y >= 0 && (x as usize) < self.width && (y as usize) < self.height {
            let at = (y as usize * self.width + x as usize) * 4;
            self.rgba[at..at + 4].copy_from_slice(&colour);
        }
    }

    /// Draws sprite pixels at `(left, top)`; alpha 0 pixels are skipped.
    pub fn blit(&mut self, left: i32, top: i32, width: usize, height: usize, rgba: &[u8]) {
        for row in 0..height {
            for col in 0..width {
                let at = (row * width + col) * 4;
                let pixel = &rgba[at..at + 4];
                if pixel[3] != 0 {
                    self.put(
                        left + col as i32,
                        top + row as i32,
                        pixel.try_into().unwrap(),
                    );
                }
            }
        }
    }

    /// One-pixel line (Bresenham).
    pub fn line(&mut self, (mut x0, mut y0): (i32, i32), (x1, y1): (i32, i32), colour: [u8; 4]) {
        let (dx, dy) = ((x1 - x0).abs(), -(y1 - y0).abs());
        let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        loop {
            self.put(x0, y0, colour);
            if (x0, y0) == (x1, y1) {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }
}

/// Grid outline colour (matches the map editor).
pub const GRID_COLOUR: [u8; 4] = [0x60, 0x70, 0x80, 0xff];

/// Corners (top, right, bottom, left) of tile `(x, y)`'s diamond.
pub fn diamond(size: usize, x: i64, y: i64) -> [(i32, i32); 4] {
    let (cx, cy) = tile_center(size, x, y);
    [(cx, cy - 14), (cx + 27, cy), (cx, cy + 14), (cx - 27, cy)]
}

/// An RGBA sprite as `(width, height, pixels)`.
type Rgba = (usize, usize, Vec<u8>);

/// Renders the whole map at 1:1 on a black background.
pub fn compose(scene: &Scene, art: &Art, grid: bool) -> Image {
    let (width, height) = map_pixels(scene.size);
    let mut image = Image::new(width as usize, height as usize, [0, 0, 0, 255]);
    let mut cache: HashMap<(Sheet, &str), Option<Rgba>> = HashMap::new();
    for item in &scene.items {
        let sprite = cache
            .entry((item.sheet, item.sprite.as_str()))
            .or_insert_with(|| art.rgba(item.sheet, &item.sprite));
        if let Some((w, h, rgba)) = sprite {
            let (left, top) = sprite_origin(scene.size, item.x, item.y);
            image.blit(left, top, *w, *h, rgba);
        }
    }
    if grid {
        for y in 0..scene.size as i64 {
            for x in 0..scene.size as i64 {
                let corners = diamond(scene.size, x, y);
                for i in 0..4 {
                    image.line(corners[i], corners[(i + 1) % 4], GRID_COLOUR);
                }
            }
        }
    }
    image
}
