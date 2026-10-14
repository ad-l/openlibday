//! Interface text: the bundled Liberation Sans (SIL OFL 1.1, see
//! `assets/fonts/LICENSE-LiberationFonts.txt`) rasterised with `fontdue`.

use fontdue::{Font, FontSettings};

use crate::Image;

static REGULAR: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../assets/fonts/LiberationSans-Bold.ttf");

pub struct Fonts {
    regular: Font,
    bold: Font,
}

impl Default for Fonts {
    fn default() -> Self {
        Self::new()
    }
}

impl Fonts {
    pub fn new() -> Self {
        let load = |bytes: &[u8]| {
            Font::from_bytes(bytes, FontSettings::default()).expect("bundled font is valid")
        };
        Self {
            regular: load(REGULAR),
            bold: load(BOLD),
        }
    }

    fn font(&self, bold: bool) -> &Font {
        if bold { &self.bold } else { &self.regular }
    }

    /// Advance width of `text` at `px` pixels per em.
    pub fn measure(&self, text: &str, px: f32, bold: bool) -> f32 {
        let font = self.font(bold);
        let mut width = 0.0;
        let mut previous = None;
        for c in text.chars() {
            if let Some(p) = previous {
                width += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            width += font.metrics(c, px).advance_width;
            previous = Some(c);
        }
        width
    }

    /// Line height (ascent to descent) at `px`.
    pub fn line_height(&self, px: f32) -> i32 {
        self.regular
            .horizontal_line_metrics(px)
            .map_or(px.ceil(), |m| (m.ascent - m.descent).ceil()) as i32
    }

    /// Renders one line of `colour` text on a transparent background.
    pub fn render(&self, text: &str, px: f32, bold: bool, colour: [u8; 3]) -> Image {
        let font = self.font(bold);
        let ascent = font
            .horizontal_line_metrics(px)
            .map_or(px, |m| m.ascent)
            .round() as i32;
        let width = self.measure(text, px, bold).ceil().max(1.0) as usize + 1;
        let height = self.line_height(px).max(1) as usize;
        let mut image = Image::new(width, height, [colour[0], colour[1], colour[2], 0]);
        let mut pen = 0.0f32;
        let mut previous = None;
        for c in text.chars() {
            if let Some(p) = previous {
                pen += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            let (metrics, coverage) = font.rasterize(c, px);
            let left = pen.round() as i32 + metrics.xmin;
            let top = ascent - metrics.height as i32 - metrics.ymin;
            for row in 0..metrics.height {
                for col in 0..metrics.width {
                    let (x, y) = (left + col as i32, top + row as i32);
                    if x < 0 || y < 0 || x as usize >= width || y as usize >= height {
                        continue;
                    }
                    let at = (y as usize * width + x as usize) * 4 + 3;
                    let a = coverage[row * metrics.width + col];
                    image.rgba[at] = image.rgba[at].max(a);
                }
            }
            pen += metrics.advance_width;
            previous = Some(c);
        }
        image
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_visible_text() {
        let fonts = Fonts::new();
        let image = fonts.render("Trooper x1", 12.0, false, [255, 255, 255]);
        assert!(image.width > 40 && image.width < 80, "{}", image.width);
        assert!(image.rgba.chunks(4).any(|p| p[3] > 200));
        assert!(fonts.measure("WW", 12.0, true) > fonts.measure("ii", 12.0, true));
        assert_eq!(fonts.measure("", 12.0, false), 0.0);
    }
}
