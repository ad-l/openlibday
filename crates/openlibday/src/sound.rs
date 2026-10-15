//! Animation sound. Unit and building animations carry their sound as a
//! Smacker audio track, one chunk per frame, which the original plays as
//! each frame is shown; so here each newly shown frame of a playing
//! animation feeds its chunk to that animation's open mixer voice.
//! Interface events play the `SOUNDS\*.WAV` effects the original names
//! (`SELECT`, `DEPLOY`, ...). Everything mixes in software into one SDL
//! stream, so any number of sounds overlap.

use std::collections::HashMap;
use std::time::Duration;

use libday_data::{Dir, GameData};
use libday_formats::{Mff, Smacker, Wav};
use libday_view::movement::WALK_POSITION_MS;
use sdl3::AudioSubsystem;
use sdl3::audio::{AudioCallback, AudioFormat, AudioSpec, AudioStream, AudioStreamWithCallback};

use crate::mixer::{Mixer, VoiceId};

/// Output sample rate.
const RATE: u32 = 44100;
/// Most skipped frames caught up with when the display lags.
const MAX_CATCH_UP: usize = 3;
/// Silence queued before an animation's sound, so frame timing jitter does
/// not starve its voice between chunks.
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
    voice: Option<VoiceId>,
}

/// Interface sound effects, by the original's names (the file names in
/// `SOUNDS\`, except `INVALID`, which is `INVALIDE.WAV`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Effect {
    /// A unit is selected.
    Select,
    /// A unit is deployed.
    Deploy,
    /// A deployed unit is recalled.
    Recall,
    /// A refused action (deploying outside the zone, without resources).
    Invalid,
}

impl Effect {
    fn file(self) -> &'static str {
        match self {
            Effect::Select => "SELECT.WAV",
            Effect::Deploy => "DEPLOY.WAV",
            Effect::Recall => "RECALL.WAV",
            Effect::Invalid => "INVALIDE.WAV",
        }
    }
}

/// Runs on SDL's audio thread, with the stream locked.
struct Feed {
    mixer: Mixer,
    buf: Vec<f32>,
}

impl AudioCallback<f32> for Feed {
    fn callback(&mut self, stream: &mut AudioStream, requested: i32) {
        self.buf.resize(requested.max(0) as usize, 0.0);
        self.mixer.mix(&mut self.buf);
        let _ = stream.put_data_f32(&self.buf);
    }
}

struct Output {
    stream: AudioStreamWithCallback<Feed>,
}

pub struct Sound {
    audio: Option<AudioSubsystem>,
    output: Option<Output>,
    /// Opening the device failed: stay silent.
    failed: bool,
    clips: HashMap<String, Option<Clip>>,
    effects: HashMap<Effect, Option<Wav>>,
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
            effects: HashMap::new(),
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
            let (frames, voice) = match previous {
                Some(i) => {
                    let a = self.active.swap_remove(i);
                    let from =
                        (a.playing.frame + 1).max((p.frame + 1).saturating_sub(MAX_CATCH_UP));
                    (from..=p.frame, a.voice)
                }
                None => (p.frame..=p.frame, None),
            };
            let voice = if frames.is_empty() {
                voice
            } else {
                self.queue(ff, &p, frames, voice, fresh)
            };
            next.push(Active { playing: p, voice });
        }
        // Animations gone from screen finish what they have queued.
        let ended: Vec<VoiceId> = self.active.iter().filter_map(|a| a.voice).collect();
        self.active = next;
        if !ended.is_empty()
            && let Some(mut feed) = self.output.as_mut().and_then(|o| o.stream.lock())
        {
            for v in ended {
                feed.mixer.close(v);
            }
        }
    }

    fn queue(
        &mut self,
        ff: &[u8],
        p: &Playing,
        frames: std::ops::RangeInclusive<usize>,
        voice: Option<VoiceId>,
        fresh: bool,
    ) -> Option<VoiceId> {
        let clip = self
            .clips
            .entry(p.name.clone())
            .or_insert_with(|| Clip::load(ff, &p.name))
            .as_ref()?;
        if fresh {
            push_log(&mut self.log, &p.name);
        }
        let volume = self.volume;
        let output = open(&mut self.output, &mut self.failed, &self.audio, volume)?;
        if voice.is_none() && !fresh {
            return None;
        }
        let mut data = Vec::new();
        if fresh {
            data.resize((clip.rate as u64 * LEAD_MS / 1000) as usize, 0);
        }
        for f in frames {
            data.extend_from_slice(clip.frame(f));
        }
        let mut feed = output.stream.lock()?;
        match voice {
            Some(v) => feed.mixer.append(v, &data).then_some(v),
            None => feed.mixer.play(&data, anim_rate(clip.rate), true),
        }
    }

    /// Plays an interface sound effect.
    pub fn effect(&mut self, game: &GameData, effect: Effect) {
        let wav = self.effects.entry(effect).or_insert_with(|| {
            let bytes = game.read(Dir::Sounds, effect.file()).ok()?;
            Wav::parse(&bytes).ok().filter(|w| w.channels == 1)
        });
        let Some(wav) = wav else {
            return;
        };
        push_log(&mut self.log, effect.file());
        let volume = self.volume;
        if let Some(output) = open(&mut self.output, &mut self.failed, &self.audio, volume)
            && let Some(mut feed) = output.stream.lock()
        {
            feed.mixer.play(&wav.samples, wav.rate, false);
        }
    }

    /// Stops every sound.
    pub fn stop(&mut self) {
        self.active.clear();
        if let Some(mut feed) = self.output.as_mut().and_then(|o| o.stream.lock()) {
            feed.mixer.stop();
        }
    }
}

/// The output, opened on first use; `None` when muted or without sound.
fn open<'a>(
    output: &'a mut Option<Output>,
    failed: &mut bool,
    audio: &Option<AudioSubsystem>,
    volume: u32,
) -> Option<&'a mut Output> {
    if volume == 0 || *failed {
        return None;
    }
    if output.is_none() {
        *output = audio.as_ref().and_then(|a| Output::open(a, volume));
        *failed = output.is_none();
    }
    output.as_mut()
}

fn push_log(log: &mut Vec<String>, name: &str) {
    if log.len() == LOG {
        log.remove(0);
    }
    log.push(name.to_owned());
}

impl Output {
    fn open(audio: &AudioSubsystem, volume: u32) -> Option<Output> {
        let spec = AudioSpec::new(Some(RATE as i32), Some(1), Some(AudioFormat::F32LE));
        let feed = Feed {
            mixer: Mixer::new(RATE, volume as f32 / 100.0),
            buf: Vec::new(),
        };
        let stream = audio
            .default_playback_device()
            .open_playback_stream_with_callback(&spec, feed)
            .and_then(|s| s.resume().map(|()| s))
            .map_err(|e| eprintln!("openlibday: no sound: {e}"))
            .ok()?;
        Some(Output { stream })
    }
}

/// The stream rate for animation sound recorded at `rate`: Smacker chunks
/// hold one frame of 1/15 s; playing them a touch slower keeps them in step
/// with frames shown every [`WALK_POSITION_MS`].
fn anim_rate(rate: u32) -> u32 {
    (rate as u64 * 1000 / 15 / WALK_POSITION_MS) as u32
}
