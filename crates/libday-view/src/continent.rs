//! The continent (strategic) map: a `COLANMS\Cn.SMK` animation whose first
//! frames loop the sea and whose later frames hold each province's
//! overlays, black transparent: the selected look (`first + 2(n-1)`) and
//! the captured look (`first + 2(n-1) + 1`).

use std::collections::BTreeMap;

use libday_formats::Smacker;
use libday_formats::smacker::Palette;

use crate::Image;

/// A province overlay cropped to its opaque box.
#[derive(Clone, Debug)]
pub struct Overlay {
    /// Top-left corner on the map.
    pub x: i32,
    pub y: i32,
    pub image: Image,
}

impl Overlay {
    /// Whether map pixel `(x, y)` is opaque.
    pub fn covers(&self, x: i32, y: i32) -> bool {
        let (ix, iy) = (x - self.x, y - self.y);
        ix >= 0
            && iy >= 0
            && (ix as usize) < self.image.width
            && (iy as usize) < self.image.height
            && self.image.rgba[(iy as usize * self.image.width + ix as usize) * 4 + 3] != 0
    }
}

/// How a province is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Selected = 0,
    Captured = 1,
}

/// The decoded continent map.
#[derive(Clone, Debug)]
pub struct ContinentMap {
    pub width: usize,
    pub height: usize,
    /// Duration of a sea frame.
    pub frame_micros: u64,
    /// The sea loop, indexed.
    sea: Vec<(Box<Palette>, Vec<u8>)>,
    /// Both looks of each province, by number.
    overlays: BTreeMap<i64, [Option<Overlay>; 2]>,
}

impl ContinentMap {
    /// Decodes `bytes`: `frames` sea frames from `first`, then two overlays
    /// per province of `provinces` (1-based numbers).
    pub fn from_smacker(
        bytes: &[u8],
        first: usize,
        frames: usize,
        provinces: &[i64],
    ) -> Result<Self, libday_formats::SmackerError> {
        let smk = Smacker::parse(bytes)?;
        let (width, height) = (smk.header.width as usize, smk.header.height as usize);
        let wanted = |i: usize| -> Option<(i64, Look)> {
            let k = i.checked_sub(first + frames)?;
            let n = (k / 2) as i64 + 1;
            let look = if k % 2 == 0 {
                Look::Selected
            } else {
                Look::Captured
            };
            provinces.contains(&n).then_some((n, look))
        };
        let last = first + frames + 2 * provinces.iter().copied().max().unwrap_or(0) as usize;
        let mut decoder = smk.decoder()?;
        let mut sea = Vec::new();
        let mut overlays: BTreeMap<i64, [Option<Overlay>; 2]> = BTreeMap::new();
        while decoder.position() < last {
            let index = decoder.position();
            let Some(frame) = decoder.next_frame()? else {
                break;
            };
            if (first..first + frames).contains(&index) {
                sea.push((Box::new(frame.palette), frame.pixels.clone()));
            } else if let Some((n, look)) = wanted(index) {
                let overlay = crop(&frame.palette, &frame.pixels, width, height);
                overlays.entry(n).or_default()[look as usize] = overlay;
            }
        }
        Ok(Self {
            width,
            height,
            frame_micros: smk.header.frame_micros(),
            sea,
            overlays,
        })
    }

    pub fn sea_frames(&self) -> usize {
        self.sea.len()
    }

    /// Sea frame `i` (wrapping) as RGBA.
    pub fn sea(&self, i: usize) -> Image {
        let mut image = Image::new(self.width, self.height, [0, 0, 0, 255]);
        if let Some((palette, pixels)) = self.sea.get(i % self.sea.len().max(1)) {
            for (out, &p) in image.rgba.as_chunks_mut::<4>().0.iter_mut().zip(pixels) {
                let [r, g, b] = palette[p as usize];
                *out = [r, g, b, 255];
            }
        }
        image
    }

    /// The sea colour at the map's corner, to fill around it.
    pub fn edge(&self) -> [u8; 3] {
        self.sea
            .first()
            .and_then(|(palette, pixels)| Some(palette[*pixels.first()? as usize]))
            .unwrap_or([0, 0, 64])
    }

    pub fn overlay(&self, province: i64, look: Look) -> Option<&Overlay> {
        self.overlays.get(&province)?[look as usize].as_ref()
    }

    pub fn provinces(&self) -> impl Iterator<Item = i64> + '_ {
        self.overlays.keys().copied()
    }

    /// The province under map pixel `(x, y)`.
    pub fn hit(&self, x: i32, y: i32) -> Option<i64> {
        self.overlays
            .iter()
            .find_map(|(&n, looks)| looks.iter().flatten().any(|o| o.covers(x, y)).then_some(n))
    }

    /// A map pixel inside province `n` (the middle of its opaque pixels).
    pub fn inside(&self, n: i64) -> Option<(i32, i32)> {
        let o = self.overlay(n, Look::Selected)?;
        let w = o.image.width;
        let opaque: Vec<(i32, i32)> = (0..w * o.image.height)
            .filter(|i| o.image.rgba[i * 4 + 3] != 0)
            .map(|i| (o.x + (i % w) as i32, o.y + (i / w) as i32))
            .collect();
        let (sx, sy) = opaque
            .iter()
            .fold((0i64, 0i64), |(a, b), &(x, y)| (a + x as i64, b + y as i64));
        let len = opaque.len().max(1) as i64;
        let centre = ((sx / len) as i32, (sy / len) as i32);
        if o.covers(centre.0, centre.1) {
            return Some(centre);
        }
        opaque
            .into_iter()
            .min_by_key(|&(x, y)| (x - centre.0).pow(2) + (y - centre.1).pow(2))
    }
}

/// The non-black pixels of a frame, cropped to their box.
fn crop(palette: &Palette, pixels: &[u8], width: usize, height: usize) -> Option<Overlay> {
    let opaque = |x: usize, y: usize| palette[pixels[y * width + x] as usize] != [0, 0, 0];
    let (mut x0, mut y0, mut x1, mut y1) = (width, height, 0, 0);
    for y in 0..height {
        for x in 0..width {
            if opaque(x, y) {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    if x0 >= x1 {
        return None;
    }
    let (w, h) = (x1 - x0, y1 - y0);
    let mut image = Image::new(w, h, [0, 0, 0, 0]);
    for y in 0..h {
        for x in 0..w {
            if opaque(x0 + x, y0 + y) {
                let [r, g, b] = palette[pixels[(y0 + y) * width + x0 + x] as usize];
                image.rgba[(y * w + x) * 4..][..4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }
    Some(Overlay {
        x: x0 as i32,
        y: y0 as i32,
        image,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c1() -> Option<ContinentMap> {
        let root = std::env::var_os("LIBDAY_ROOT")?;
        let game = libday_data::GameData::open(root).ok()?;
        let bytes = game.read(libday_data::Dir::ColAnms, "C1.SMK").ok()?;
        Some(ContinentMap::from_smacker(&bytes, 0, 30, &[1, 2, 3, 4, 5, 6, 7]).unwrap())
    }

    #[test]
    fn c1_overlays_and_hits() {
        let Some(map) = c1() else {
            return;
        };
        assert_eq!((map.width, map.height, map.sea_frames()), (840, 728, 30));
        assert_eq!(
            map.provinces().collect::<Vec<_>>(),
            (1..=7).collect::<Vec<_>>()
        );
        let o = map.overlay(1, Look::Selected).unwrap();
        assert_eq!((o.x, o.y), (364, 316), "{:?}", map.inside(2));
        for n in 1..=7 {
            let (x, y) = map.inside(n).unwrap();
            assert_eq!(map.hit(x, y), Some(n));
        }
        assert_eq!(map.hit(10, 10), None);
    }
}
