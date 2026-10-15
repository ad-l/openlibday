//! Animation sound. Unit and building animations carry their sound as a
//! Smacker audio track, one chunk per frame, which the original plays as
//! each frame is shown; so here each newly shown frame of a playing
//! animation queues its chunk, on one SDL stream per animation so that
//! simultaneous animations mix.

use std::collections::HashMap;
use std::time::Duration;

use libday_formats::{Mff, Smacker};
use libday_view::movement::WALK_POSITION_MS;
use sdl3::AudioSubsystem;
use sdl3::audio::{AudioDevice, AudioFormat, AudioSpec, AudioStreamOwner};

/// Most streams mixed at once.
const MAX_STREAMS: usize = 16;
/// Most skipped frames caught up with when the display lags.
const MAX_CATCH_UP: usize = 3;
/// Silence queued before a sound, so frame timing jitter does not starve
/// the stream between chunks.
const LEAD_MS: u64 = 20;
/// Instances remembered for scripts.
const LOG: usize = 32;

/// An animation's sound, split by frame.
struct Clip {
    rate: u32,
    samples: Vec<i16>,
    /// Sample range of each frame (`frames + 1` offsets).
    offsets: Vec<usize>,
}

impl Clip {
    fn load(ff: &[u8], name: &str) -> Option<Clip> {
        let smk = Smacker::parse(Mff::parse(ff).ok()?.get(name).ok()?).ok()?;
        // Every animation sound is mono.
        let audio = smk.decode_audio(0).ok()??;
        let audible = audio.samples.iter().any(|&s| s.unsigned_abs() > 256);
        (audio.channels == 1 && audio.rate > 0 && audible).then_some(Clip {
            rate: audio.rate,
            samples: audio.samples,
            offsets: audio.offsets,
        })
    }

    fn frame(&self, f: usize) -> &[i16] {
        let n = self.offsets.len() - 1;
        if n == 0 {
            return &[];
        }
        let f = f % n;
        &self.samples[self.offsets[f]..self.offsets[f + 1]]
    }
}

/// A playing animation instance: what is shown, from which slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Playing {
    /// Distinguishes simultaneous animations (walker, projectile, ...).
    pub slot: usize,
    pub name: String,
    /// When the sequence it belongs to started; a new start is a new
    /// instance even with the same name.
    pub started: Duration,
    /// Frame counter (wraps around the animation's length).
    pub frame: usize,
}

struct Active {
    playing: Playing,
    stream: Option<usize>,
}

struct Output {
    audio: AudioSubsystem,
    device: AudioDevice,
    /// Streams and the sample rate each was made for.
    streams: Vec<(u32, AudioStreamOwner)>,
}

pub struct Sound {
    audio: Option<AudioSubsystem>,
    output: Option<Output>,
    /// Opening the device failed: stay silent.
    failed: bool,
    clips: HashMap<String, Option<Clip>>,
    active: Vec<Active>,
    /// 0 (mute) to 100.
    volume: u32,
    /// Names of the latest sound instances started (for scripts).
    pub log: Vec<String>,
}

impl Sound {
    /// `audio` is `None` without a sound device (headless runs): sounds are
    /// only logged.
    pub fn new(audio: Option<AudioSubsystem>, volume: u32) -> Self {
        Self {
            audio,
            output: None,
            failed: false,
            clips: HashMap::new(),
            active: Vec::new(),
            volume: volume.min(100),
            log: Vec::new(),
        }
    }

    /// Queues the sound of each newly shown frame of `now`, the animations
    /// on screen.
    pub fn update(&mut self, ff: Option<&[u8]>, now: Vec<Playing>) {
        let Some(ff) = ff else {
            return;
        };
        let mut next: Vec<Active> = Vec::with_capacity(now.len());
        for p in now {
            let previous = self.active.iter().position(|a| {
                a.playing.slot == p.slot
                    && a.playing.name == p.name
                    && a.playing.started == p.started
                    && a.playing.frame <= p.frame
            });
            let fresh = previous.is_none();
            let (frames, stream) = match previous {
                Some(i) => {
                    let a = self.active.swap_remove(i);
                    let from =
                        (a.playing.frame + 1).max((p.frame + 1).saturating_sub(MAX_CATCH_UP));
                    (from..=p.frame, a.stream)
                }
                None => (p.frame..=p.frame, None),
            };
            let stream = if frames.is_empty() {
                stream
            } else {
                let busy: Vec<usize> = next
                    .iter()
                    .chain(&self.active)
                    .filter_map(|a| a.stream)
                    .collect();
                self.queue(ff, &p, frames, stream, fresh, &busy)
            };
            next.push(Active { playing: p, stream });
        }
        self.active = next;
    }

    fn queue(
        &mut self,
        ff: &[u8],
        p: &Playing,
        frames: std::ops::RangeInclusive<usize>,
        stream: Option<usize>,
        fresh: bool,
        busy: &[usize],
    ) -> Option<usize> {
        let clip = self
            .clips
            .entry(p.name.clone())
            .or_insert_with(|| Clip::load(ff, &p.name))
            .as_ref()?;
        if fresh {
            if self.log.len() == LOG {
                self.log.remove(0);
            }
            self.log.push(p.name.clone());
        }
        if self.volume == 0 || self.failed {
            return stream;
        }
        if self.output.is_none() {
            self.output = self.audio.as_ref().and_then(Output::open);
            self.failed = self.output.is_none();
        }
        let output = self.output.as_mut()?;
        let index = match stream {
            Some(i) => i,
            None if fresh => output.idle_stream(clip.rate, busy, self.volume)?,
            None => return None,
        };
        let mut data = Vec::new();
        if fresh {
            data.resize((clip.rate as u64 * LEAD_MS / 1000) as usize, 0);
        }
        for f in frames {
            data.extend_from_slice(clip.frame(f));
        }
        output.streams[index].1.put_data_i16(&data).ok()?;
        Some(index)
    }

    /// Stops every sound.
    pub fn stop(&mut self) {
        self.active.clear();
        if let Some(o) = &self.output {
            for (_, s) in &o.streams {
                let _ = s.clear();
            }
        }
    }
}

fn gain(volume: u32) -> f32 {
    volume as f32 / 100.0
}

impl Output {
    fn open(audio: &AudioSubsystem) -> Option<Output> {
        let spec = AudioSpec::new(Some(44100), Some(2), Some(AudioFormat::S16LE));
        let device = audio
            .open_playback_device(&spec)
            .map_err(|e| eprintln!("openlibday: no sound: {e}"))
            .ok()?;
        Some(Output {
            audio: audio.clone(),
            device,
            streams: Vec::new(),
        })
    }

    /// A stream for `rate` with nothing left to play, other than `busy`
    /// ones.
    fn idle_stream(&mut self, rate: u32, busy: &[usize], volume: u32) -> Option<usize> {
        let idle = |i: usize| -> bool {
            let (r, s) = &self.streams[i];
            *r == rate && !busy.contains(&i) && s.queued_bytes().unwrap_or(0) == 0
        };
        if let Some(i) = (0..self.streams.len()).find(|&i| idle(i)) {
            return Some(i);
        }
        if self.streams.len() == MAX_STREAMS {
            return None;
        }
        let stream = self.audio.new_playback_stream(&spec(rate), None).ok()?;
        let _ = stream.set_gain(gain(volume));
        self.device.bind_stream(&stream).ok()?;
        self.streams.push((rate, stream));
        Some(self.streams.len() - 1)
    }
}

/// Smacker chunks hold one frame of 1/15 s; playing them a touch slower
/// keeps them in step with frames shown every [`WALK_POSITION_MS`].
fn spec(rate: u32) -> AudioSpec {
    let rate = rate as u64 * 1000 / 15 / WALK_POSITION_MS;
    AudioSpec::new(Some(rate as i32), Some(1), Some(AudioFormat::S16LE))
}
