//! Software mixer: any number of mono 16-bit voices, each at its own sample
//! rate, resampled (linear interpolation) and summed into one output
//! stream. A voice is either complete when started (an effect) or open, fed
//! as it plays (an animation's per-frame sound, later streamed music) and
//! closed when nothing more will come; finished voices are dropped.

use std::collections::VecDeque;

/// Most voices mixed at once; further sounds are dropped until some end.
const MAX_VOICES: usize = 64;

/// A voice handle; stale once the voice has finished.
pub type VoiceId = u64;

struct Voice {
    id: VoiceId,
    /// Samples not played yet (the front one is at `pos`).
    data: VecDeque<i16>,
    /// Fractional position between `data[0]` and `data[1]`.
    pos: f64,
    /// Source samples per output sample.
    step: f64,
    /// More samples may still be appended.
    open: bool,
}

pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    next: VoiceId,
    gain: f32,
}

impl Mixer {
    /// A mixer producing `rate` samples per second, at `gain` (0 to 1).
    pub fn new(rate: u32, gain: f32) -> Self {
        Self {
            rate,
            voices: Vec::new(),
            next: 0,
            gain,
        }
    }

    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
    }

    /// Starts a voice playing `samples` recorded at `rate`; `open` voices
    /// wait for [`Mixer::append`] until [`Mixer::close`]d.
    pub fn play(&mut self, samples: &[i16], rate: u32, open: bool) -> Option<VoiceId> {
        if self.voices.len() == MAX_VOICES || rate == 0 {
            return None;
        }
        self.next += 1;
        self.voices.push(Voice {
            id: self.next,
            data: samples.iter().copied().collect(),
            pos: 0.0,
            step: rate as f64 / self.rate as f64,
            open,
        });
        Some(self.next)
    }

    /// Feeds an open voice; false if it has finished.
    pub fn append(&mut self, id: VoiceId, samples: &[i16]) -> bool {
        match self.voices.iter_mut().find(|v| v.id == id && v.open) {
            Some(v) => {
                v.data.extend(samples);
                true
            }
            None => false,
        }
    }

    /// Lets a voice end once it has played what it has.
    pub fn close(&mut self, id: VoiceId) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
            v.open = false;
        }
    }

    /// Ends a voice now, fading out over `fade` source samples so it does
    /// not click.
    pub fn cut(&mut self, id: VoiceId, fade: usize) {
        if let Some(v) = self.voices.iter_mut().find(|v| v.id == id) {
            v.open = false;
            let n = fade.min(v.data.len());
            v.data.truncate(n);
            for (i, s) in v.data.iter_mut().enumerate() {
                *s = (i64::from(*s) * (n - i) as i64 / (n as i64 + 1)) as i16;
            }
        }
    }

    /// Silences every voice.
    pub fn stop(&mut self) {
        self.voices.clear();
    }

    /// Voices playing or waiting for samples.
    #[cfg(test)]
    pub fn voices(&self) -> usize {
        self.voices.len()
    }

    /// Fills `out` with the next mixed samples.
    pub fn mix(&mut self, out: &mut [f32]) {
        out.fill(0.0);
        for v in &mut self.voices {
            for o in out.iter_mut() {
                let i = v.pos as usize;
                let Some(&a) = v.data.get(i) else {
                    break;
                };
                let b = match v.data.get(i + 1) {
                    Some(&b) => b,
                    // Starved: wait for the next chunk rather than click.
                    None if v.open => break,
                    None => a,
                };
                let t = (v.pos - i as f64) as f32;
                *o += (a as f32 + (b as f32 - a as f32) * t) / 32768.0;
                v.pos += v.step;
            }
            let done = (v.pos as usize).min(v.data.len());
            v.data.drain(..done);
            v.pos -= done as f64;
        }
        self.voices.retain(|v| v.open || !v.data.is_empty());
        for o in out {
            *o = (*o * self.gain).clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_voices_sum_and_end() {
        let mut m = Mixer::new(100, 1.0);
        m.play(&[16384; 4], 100, false).unwrap();
        m.play(&[8192; 8], 100, false).unwrap();
        let mut out = [0.0; 6];
        m.mix(&mut out);
        assert_eq!(out, [0.75, 0.75, 0.75, 0.75, 0.25, 0.25]);
        assert_eq!(m.voices(), 1);
        m.mix(&mut out);
        assert_eq!(out, [0.25, 0.25, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn many_effects_never_exhaust_the_mixer() {
        let mut m = Mixer::new(100, 1.0);
        let mut out = [0.0; 10];
        for _ in 0..1000 {
            assert!(m.play(&[100; 5], 50, false).is_some());
            assert!(m.play(&[100; 5], 75, false).is_some());
            m.mix(&mut out);
        }
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn resamples_and_waits_for_open_voices() {
        let mut m = Mixer::new(100, 1.0);
        let v = m.play(&[0, 16384], 50, true).unwrap();
        let mut out = [0.0; 4];
        m.mix(&mut out);
        // Interpolated at half speed, then starved until fed.
        assert_eq!(out, [0.0, 0.25, 0.0, 0.0]);
        assert!(m.append(v, &[16384]));
        m.close(v);
        m.mix(&mut out);
        assert_eq!(out, [0.5; 4]);
        assert_eq!(m.voices(), 0);
        assert!(!m.append(v, &[1]));
    }

    #[test]
    fn cut_fades_out_and_ends() {
        let mut m = Mixer::new(100, 1.0);
        let v = m.play(&[16384; 50], 100, true).unwrap();
        m.cut(v, 3);
        let mut out = [0.0; 5];
        m.mix(&mut out);
        assert_eq!(out, [0.375, 0.25, 0.125, 0.0, 0.0]);
        assert_eq!(m.voices(), 0);
    }

    #[test]
    fn clamps_the_sum() {
        let mut m = Mixer::new(10, 1.0);
        m.play(&[30000], 10, false);
        m.play(&[30000], 10, false);
        let mut out = [0.0; 1];
        m.mix(&mut out);
        assert_eq!(out, [1.0]);
    }
}
