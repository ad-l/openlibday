//! Cutscene videos (`VIDEO\*.SMK`: 640x400 Smacker files at 15 frames per
//! second with a mono soundtrack). One plays over whatever screen comes
//! next, which waits, frozen, until the video ends or Escape skips it.
//!
//! The original plays `FH2Intro` at start-up (unless the `Intro` profile
//! key is 0) and from the main menu's INTRO button; the continent's
//! `Intro` video when a campaign starts (`0x47ef17`) and when the next
//! continent begins (`0x47f07e`); `CounterA` before a counterattack
//! (`0x47f37a`); and `FH2End` or `FH2Lose` when the campaign is won or
//! lost (`0x4115ed`).

use std::time::Duration;

use libday_data::Dir;
use libday_formats::smacker::Stream;
use sdl3::pixels::Color;
use sdl3::render::{Canvas, FRect};
use sdl3::video::Window;

use crate::Viewer;
use crate::hud::{self, Gfx, Result};
use crate::mixer::VoiceId;

/// Output pixels per video pixel at interface density 1.
const ZOOM: f32 = 2.0;

pub(crate) struct Cutscene {
    /// The name it was asked by (`Intro1`, `CounterA`, ...).
    pub name: String,
    stream: Stream,
    /// Distinguishes plays of the same video (texture cache).
    serial: u64,
    started: Duration,
    frame: Duration,
    frames: usize,
    voice: Option<(VoiceId, u32)>,
}

impl Cutscene {
    /// Frames shown so far (the decoded one is `shown - 1`).
    pub(crate) fn shown(&self) -> usize {
        self.stream.position()
    }
}

impl Viewer {
    /// Starts video `name` (`VIDEO\<name>.SMK`) over the current screen;
    /// does nothing if it cannot be read.
    pub(crate) fn play_video(&mut self, name: &str) {
        if name.is_empty() {
            return;
        }
        self.end_video();
        let file = format!("{}.SMK", name.to_ascii_uppercase());
        let stream = match self
            .game
            .read(Dir::Video, &file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| Stream::new(bytes).map_err(|e| e.to_string()))
        {
            Ok(stream) => stream,
            Err(e) => {
                eprintln!("openlibday: video {file}: {e}");
                return;
            }
        };
        let header = stream.header.clone();
        let voice = match stream.audio(0) {
            Ok(Some(audio)) if audio.channels == 1 => self
                .sound
                .track(&file, &audio.samples, audio.rate)
                .map(|v| (v, audio.rate)),
            _ => None,
        };
        self.video_serial += 1;
        let mut cutscene = Cutscene {
            name: name.to_owned(),
            stream,
            serial: self.video_serial,
            started: self.clock.now(),
            frame: Duration::from_micros(header.frame_micros()),
            frames: header.frames as usize,
            voice,
        };
        if cutscene.stream.next_frame().is_err() {
            return;
        }
        self.pressed = None;
        self.drag = None;
        self.press = None;
        self.cutscene = Some(cutscene);
    }

    /// Stops the playing video (Escape, or a new one).
    pub(crate) fn end_video(&mut self) {
        if let Some(c) = self.cutscene.take()
            && let Some((voice, rate)) = c.voice
        {
            self.sound.cut(voice, rate);
        }
    }

    /// Decodes up to the frame due now, ending the video after its last
    /// frame; returns whether the picture changed.
    pub(crate) fn video_tick(&mut self) -> bool {
        let Some(c) = self.cutscene.as_mut() else {
            return false;
        };
        let due = (self.clock.since(c.started).as_micros() / c.frame.as_micros().max(1)) as usize;
        if due >= c.frames {
            // The soundtrack has played out with the last frame.
            self.cutscene = None;
            return true;
        }
        let before = c.shown();
        while c.shown() <= due {
            if c.stream.next_frame().is_err() {
                self.end_video();
                return true;
            }
        }
        c.shown() != before
    }

    /// Time until the next video frame is due.
    pub(crate) fn video_wait(&self) -> Option<Duration> {
        let c = self.cutscene.as_ref()?;
        let next = c.frame * c.shown() as u32;
        Some(next.saturating_sub(self.clock.since(c.started)))
    }
}

/// Draws the playing video centred on black, at [`ZOOM`] times the
/// interface density or as large as fits.
pub(crate) fn draw(canvas: &mut Canvas<Window>, gfx: &mut Gfx, viewer: &Viewer) -> Result {
    let Some(c) = &viewer.cutscene else {
        return Ok(());
    };
    canvas.set_draw_color(Color::RGB(0, 0, 0));
    canvas.clear();
    let (w, h) = (c.stream.header.width as f32, c.stream.header.height as f32);
    let key = (c.serial, c.shown());
    if gfx.video.as_ref().is_none_or(|(k, _)| *k != key) {
        let frame = c.stream.frame();
        let rgba = frame
            .pixels
            .iter()
            .flat_map(|&p| {
                let [r, g, b] = frame.palette[p as usize];
                [r, g, b, 255]
            })
            .collect();
        let image = libday_view::Image {
            width: w as usize,
            height: h as usize,
            rgba,
        };
        gfx.video = hud::texture(gfx.creator(), &image).map(|t| (key, t));
    }
    let (ow, oh) = viewer.output;
    let zoom = (ZOOM * viewer.density).min(ow / w).min(oh / h);
    let (dw, dh) = (w * zoom, h * zoom);
    if let Some((_, texture)) = &gfx.video {
        let at = FRect::new(((ow - dw) / 2.0).floor(), ((oh - dh) / 2.0).floor(), dw, dh);
        canvas.copy(texture, None, Some(at))?;
    }
    Ok(())
}
