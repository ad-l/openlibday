//! Smacker 2 (`SMK2`) video, as used for unit and building animations.
//!
//! Layout (little-endian): a 104-byte header (signature, width, height,
//! frame count, frame rate, flags, 7 audio buffer sizes, Huffman tree
//! block size, the four big-tree table sizes, 7 audio rates, a reserved
//! word), one u32 size per frame (low two bits are flags, bit 0 marks a
//! key frame), one type byte per frame (bit 0 = palette chunk, bits 1..7 =
//! audio tracks), the packed Huffman trees, then the frames. Flag bit 0
//! adds a trailing "ring" frame that loops back to the first image.
//!
//! A frame holds an optional palette delta chunk, one length-prefixed
//! chunk per audio track, then the video bit stream. Video is coded in
//! 4x4 blocks (mono, full, skip, fill) through four 16-bit Huffman
//! "big trees" (MMAP, MCLR, FULL, TYPE). Each big tree has three escape
//! leaves that return its three most recently decoded values, reset to 0
//! at the start of every frame. Bits are read least-significant first.
//!
//! The encoder re-codes video only: palette and audio chunks, frame types
//! and key-frame flags are copied from an existing file, so edits keep the
//! original sound and palette.

use thiserror::Error;

const HEADER: usize = 104;
const MAX_TREE_DEPTH: usize = 500;
const RING_FRAME: u32 = 1;
const BLOCK_RUNS: [usize; 64] = {
    let mut runs = [0usize; 64];
    let mut i = 0;
    while i < 59 {
        runs[i] = i + 1;
        i += 1;
    }
    runs[59] = 128;
    runs[60] = 256;
    runs[61] = 512;
    runs[62] = 1024;
    runs[63] = 2048;
    runs
};
const SMK_PAL: [u8; 64] = [
    0x00, 0x04, 0x08, 0x0C, 0x10, 0x14, 0x18, 0x1C, 0x20, 0x24, 0x28, 0x2C, 0x30, 0x34, 0x38, 0x3C,
    0x41, 0x45, 0x49, 0x4D, 0x51, 0x55, 0x59, 0x5D, 0x61, 0x65, 0x69, 0x6D, 0x71, 0x75, 0x79, 0x7D,
    0x82, 0x86, 0x8A, 0x8E, 0x92, 0x96, 0x9A, 0x9E, 0xA2, 0xA6, 0xAA, 0xAE, 0xB2, 0xB6, 0xBA, 0xBE,
    0xC3, 0xC7, 0xCB, 0xCF, 0xD3, 0xD7, 0xDB, 0xDF, 0xE3, 0xE7, 0xEB, 0xEF, 0xF3, 0xF7, 0xFB, 0xFF,
];

const MONO: u16 = 0;
const FULL: u16 = 1;
const SKIP: u16 = 2;
const FILL: u16 = 3;

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SmackerError {
    #[error("not a Smacker 2 file")]
    BadSignature,
    #[error("Smacker file is truncated")]
    Truncated,
    #[error("invalid Smacker header: {0}")]
    InvalidHeader(&'static str),
    #[error("invalid Huffman tree: {0}")]
    InvalidTree(&'static str),
    #[error("invalid frame {frame}: {reason}")]
    InvalidFrame { frame: usize, reason: &'static str },
    #[error("cannot encode: {0}")]
    Encode(String),
}

type Result<T> = std::result::Result<T, SmackerError>;

/// 256 RGB entries.
pub type Palette = [[u8; 3]; 256];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    /// Frame count, excluding the ring frame.
    pub frames: u32,
    /// Positive: milliseconds per frame; negative: units of 10 microseconds.
    pub rate: i32,
    pub flags: u32,
    pub trees_size: u32,
    /// Allocation sizes of the MMAP, MCLR, FULL and TYPE tables.
    pub table_sizes: [u32; 4],
    pub audio_rates: [u32; 7],
}

impl Header {
    pub fn has_ring_frame(&self) -> bool {
        self.flags & RING_FRAME != 0
    }

    /// Frame duration in microseconds.
    pub fn frame_micros(&self) -> u64 {
        match self.rate {
            r if r > 0 => r as u64 * 1000,
            r if r < 0 => r.unsigned_abs() as u64 * 10,
            _ => 100_000,
        }
    }
}

/// The byte ranges of one stored frame.
#[derive(Clone, Debug)]
struct FrameParts<'a> {
    palette: Option<&'a [u8]>,
    audio: Vec<&'a [u8]>,
    video: &'a [u8],
}

/// A parsed Smacker file borrowing its bytes.
#[derive(Debug)]
pub struct Smacker<'a> {
    pub header: Header,
    raw_header: &'a [u8],
    frame_sizes: Vec<u32>,
    frame_types: Vec<u8>,
    trees: &'a [u8],
    frames: Vec<FrameParts<'a>>,
}

fn u32_at(data: &[u8], pos: usize) -> u32 {
    u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap())
}

impl<'a> Smacker<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        if data.len() < HEADER {
            return Err(SmackerError::Truncated);
        }
        if &data[..4] != b"SMK2" {
            return Err(SmackerError::BadSignature);
        }
        let mut audio_rates = [0u32; 7];
        for (i, rate) in audio_rates.iter_mut().enumerate() {
            *rate = u32_at(data, 72 + i * 4);
        }
        let header = Header {
            width: u32_at(data, 4),
            height: u32_at(data, 8),
            frames: u32_at(data, 12),
            rate: u32_at(data, 16) as i32,
            flags: u32_at(data, 20),
            trees_size: u32_at(data, 52),
            table_sizes: [
                u32_at(data, 56),
                u32_at(data, 60),
                u32_at(data, 64),
                u32_at(data, 68),
            ],
            audio_rates,
        };
        if header.width == 0 || header.height == 0 || header.width > 4096 || header.height > 4096 {
            return Err(SmackerError::InvalidHeader("dimensions"));
        }
        if !header.width.is_multiple_of(4) || !header.height.is_multiple_of(4) {
            return Err(SmackerError::InvalidHeader(
                "dimensions are not multiples of 4",
            ));
        }
        if header.frames == 0 || header.frames > 100_000 {
            return Err(SmackerError::InvalidHeader("frame count"));
        }
        let total = header.frames as usize + usize::from(header.has_ring_frame());
        let types_at = HEADER + total * 4;
        let trees_at = types_at + total;
        let frames_at = trees_at + header.trees_size as usize;
        if data.len() < frames_at {
            return Err(SmackerError::Truncated);
        }
        let frame_sizes: Vec<u32> = (0..total).map(|i| u32_at(data, HEADER + i * 4)).collect();
        let frame_types = data[types_at..trees_at].to_vec();
        let mut frames = Vec::with_capacity(total);
        let mut pos = frames_at;
        for (index, (&size, &kind)) in frame_sizes.iter().zip(&frame_types).enumerate() {
            let size = (size & !3) as usize;
            let raw = data
                .get(pos..pos + size)
                .ok_or(SmackerError::InvalidFrame {
                    frame: index,
                    reason: "truncated",
                })?;
            pos += size;
            let fail = |reason| SmackerError::InvalidFrame {
                frame: index,
                reason,
            };
            let mut cursor = 0;
            let palette = if kind & 1 != 0 {
                let len = *raw.first().ok_or(fail("missing palette"))? as usize * 4;
                if len == 0 || len > raw.len() {
                    return Err(fail("invalid palette chunk"));
                }
                cursor = len;
                Some(&raw[..len])
            } else {
                None
            };
            let mut audio = Vec::new();
            for track in 0..7 {
                if kind & (2 << track) != 0 {
                    if cursor + 4 > raw.len() {
                        return Err(fail("missing audio chunk"));
                    }
                    let len = u32_at(raw, cursor) as usize;
                    if len < 4 || cursor + len > raw.len() {
                        return Err(fail("invalid audio chunk"));
                    }
                    audio.push(&raw[cursor..cursor + len]);
                    cursor += len;
                }
            }
            frames.push(FrameParts {
                palette,
                audio,
                video: &raw[cursor..],
            });
        }
        Ok(Self {
            header,
            raw_header: &data[..HEADER],
            frame_sizes,
            frame_types,
            trees: &data[trees_at..frames_at],
            frames,
        })
    }

    /// Stored frames, including the ring frame.
    pub fn stored_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn has_audio(&self) -> bool {
        self.frames.iter().any(|f| !f.audio.is_empty())
    }

    /// Decodes audio track `track` (0..7) of every frame except the ring
    /// frame, or `None` when the track is absent.
    pub fn decode_audio(&self, track: usize) -> Result<Option<Audio>> {
        let flags = *self
            .header
            .audio_rates
            .get(track)
            .ok_or(SmackerError::InvalidHeader("audio track"))?;
        if flags & AUDIO_PRESENT == 0 {
            return Ok(None);
        }
        let mut audio = Audio {
            rate: flags & 0x00FF_FFFF,
            channels: if flags & AUDIO_STEREO != 0 { 2 } else { 1 },
            samples: Vec::new(),
            offsets: vec![0],
        };
        let sixteen = flags & AUDIO_16BIT != 0;
        for (index, frame) in self
            .frames
            .iter()
            .take(self.header.frames as usize)
            .enumerate()
        {
            let slot = (0..track)
                .filter(|t| self.frame_types[index] & (2 << t) != 0)
                .count();
            if self.frame_types[index] & (2 << track) == 0 {
                audio.offsets.push(audio.samples.len());
                continue;
            }
            let chunk = &frame.audio[slot][4..];
            if flags & AUDIO_COMPRESSED != 0 {
                decode_audio_chunk(chunk, audio.channels == 2, sixteen, &mut audio.samples).ok_or(
                    SmackerError::InvalidFrame {
                        frame: index,
                        reason: "invalid audio chunk",
                    },
                )?;
            } else if sixteen {
                audio.samples.extend(
                    chunk
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|&s| i16::from_le_bytes(s)),
                );
            } else {
                audio
                    .samples
                    .extend(chunk.iter().map(|&s| (i16::from(s) - 128) << 8));
            }
            audio.offsets.push(audio.samples.len());
        }
        Ok(Some(audio))
    }

    /// True when stored frame `index` is flagged as a key frame.
    pub fn is_key_frame(&self, index: usize) -> bool {
        index == 0 || self.frame_sizes[index] & 1 != 0
    }

    pub fn decoder(&self) -> Result<Decoder<'_, 'a>> {
        Decoder::new(self)
    }

    /// Decodes every frame except the ring frame.
    pub fn decode_all(&self) -> Result<Vec<Frame>> {
        let mut decoder = self.decoder()?;
        let mut out = Vec::with_capacity(self.header.frames as usize);
        for _ in 0..self.header.frames {
            out.push(decoder.next_frame()?.expect("frame count").clone());
        }
        Ok(out)
    }
}

/// A decoded 8-bit frame with its palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub palette: Palette,
    pub pixels: Vec<u8>,
}

impl Frame {
    /// The RGB of the most frequent pixel.
    ///
    /// Animations carry no transparency metadata: each file paints its
    /// background in an arbitrary key colour (dark red, magenta, grey, ...)
    /// that differs between files and palette indices. The most frequent
    /// colour of the first frame identifies it for every installed
    /// animation except `MPAR\P3\MPAR3M3.SMK`, whose background is dithered.
    /// Match it by RGB in later frames, since palettes can change.
    pub fn dominant_colour(&self) -> [u8; 3] {
        let mut counts = [0usize; 256];
        for &p in &self.pixels {
            counts[p as usize] += 1;
        }
        let index = (0..256)
            .max_by_key(|&i| (counts[i], std::cmp::Reverse(i)))
            .unwrap_or(0);
        self.palette[index]
    }

    fn contains(&self, colour: [u8; 3]) -> bool {
        let mut seen = [false; 256];
        for &p in &self.pixels {
            seen[p as usize] = true;
        }
        (0..256).any(|i| seen[i] && self.palette[i] == colour)
    }
}

/// The background key colour of each frame: the first frame's dominant
/// colour, replaced by a later frame's dominant colour whenever a frame no
/// longer contains the current key at all. Some full-screen animations
/// switch background part-way through (the nuke's `HNKE01` turns from
/// orange to grey at frame 11, the gene bomb's `TBMB01` to pale cyan at 46).
pub fn key_colours(frames: &[Frame]) -> Vec<[u8; 3]> {
    let mut key = None;
    frames
        .iter()
        .map(|f| {
            let k = match key {
                Some(k) if f.contains(k) => k,
                _ => f.dominant_colour(),
            };
            key = Some(k);
            k
        })
        .collect()
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn bit(&mut self) -> Option<bool> {
        let byte = *self.data.get(self.pos >> 3)?;
        let bit = (byte >> (self.pos & 7)) & 1 != 0;
        self.pos += 1;
        Some(bit)
    }

    fn bits(&mut self, count: u32) -> Option<u32> {
        let mut value = 0;
        for i in 0..count {
            value |= u32::from(self.bit()?) << i;
        }
        Some(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Node<T> {
    /// Inner node; the 0-branch follows immediately, the 1-branch is here.
    Branch(u32),
    Leaf(T),
    /// Big-tree escape leaf returning a recently decoded value.
    Escape(u8),
}

fn walk<T: Copy>(nodes: &[Node<T>], bits: &mut BitReader) -> Option<Node<T>> {
    let mut i = 0;
    loop {
        match nodes[i] {
            Node::Branch(right) => i = if bits.bit()? { right as usize } else { i + 1 },
            leaf => return Some(leaf),
        }
    }
}

fn read_byte_tree(bits: &mut BitReader) -> Result<Vec<Node<u8>>> {
    fn node(bits: &mut BitReader, out: &mut Vec<Node<u8>>, depth: usize) -> Result<()> {
        if depth > 32 || out.len() > 1024 {
            return Err(SmackerError::InvalidTree("byte tree too deep"));
        }
        let eof = SmackerError::InvalidTree("truncated byte tree");
        if bits.bit().ok_or(eof.clone())? {
            let at = out.len();
            out.push(Node::Branch(0));
            node(bits, out, depth + 1)?;
            out[at] = Node::Branch(out.len() as u32);
            node(bits, out, depth + 1)
        } else {
            out.push(Node::Leaf(bits.bits(8).ok_or(eof)? as u8));
            Ok(())
        }
    }
    let mut out = Vec::new();
    node(bits, &mut out, 0)?;
    Ok(out)
}

const AUDIO_COMPRESSED: u32 = 1 << 31;
const AUDIO_PRESENT: u32 = 1 << 30;
const AUDIO_16BIT: u32 = 1 << 29;
const AUDIO_STEREO: u32 = 1 << 28;

/// Decoded PCM, interleaved signed 16-bit samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audio {
    pub rate: u32,
    pub channels: u8,
    pub samples: Vec<i16>,
    /// Where each frame's samples start in `samples`, plus the end.
    pub offsets: Vec<usize>,
}

/// Decodes one Huffman/DPCM compressed audio chunk (after its length word):
/// u32 unpacked size, then LSB-first bits: data, stereo and 16-bit flags,
/// one byte tree per output byte lane, the initial samples (last channel
/// first; 16-bit values byte-swapped), then deltas that wrap around.
fn decode_audio_chunk(chunk: &[u8], stereo: bool, sixteen: bool, out: &mut Vec<i16>) -> Option<()> {
    let size = u32::from_le_bytes(chunk.get(..4)?.try_into().ok()?) as usize;
    let mut bits = BitReader::new(&chunk[4..]);
    if !bits.bit()? {
        return Some(());
    }
    if bits.bit()? != stereo || bits.bit()? != sixteen {
        return None;
    }
    let channels = 1 + usize::from(stereo);
    let width = 1 + usize::from(sixteen);
    let mut trees = Vec::new();
    for _ in 0..channels * width {
        bits.bit()?;
        trees.push(read_byte_tree(&mut bits).ok()?);
        bits.bit()?;
    }
    let byte = |bits: &mut BitReader, tree: usize| match walk(&trees[tree], bits)? {
        Node::Leaf(v) => Some(v),
        _ => None,
    };
    let count = size / width;
    if sixteen {
        let mut pred = [0u16; 2];
        for p in pred[..channels].iter_mut().rev() {
            *p = (bits.bits(16)? as u16).swap_bytes();
        }
        out.extend(pred[..channels].iter().map(|&p| p as i16));
        for i in channels..count {
            let c = i % channels;
            let lo = byte(&mut bits, 2 * c)?;
            let hi = byte(&mut bits, 2 * c + 1)?;
            pred[c] = pred[c].wrapping_add(u16::from_le_bytes([lo, hi]));
            out.push(pred[c] as i16);
        }
    } else {
        let mut pred = [0u8; 2];
        for p in pred[..channels].iter_mut().rev() {
            *p = bits.bits(8)? as u8;
        }
        let to16 = |v: u8| (i16::from(v) - 128) << 8;
        out.extend(pred[..channels].iter().map(|&p| to16(p)));
        for i in channels..count {
            let c = i % channels;
            pred[c] = pred[c].wrapping_add(byte(&mut bits, c)?);
            out.push(to16(pred[c]));
        }
    }
    Some(())
}

/// A decoded 16-bit "big tree" with its recent-value cache.
#[derive(Clone, Debug)]
struct BigTree {
    nodes: Vec<Node<u16>>,
    recent: [u16; 3],
}

impl BigTree {
    fn read(bits: &mut BitReader) -> Result<Self> {
        let eof = || SmackerError::InvalidTree("truncated tree header");
        if !bits.bit().ok_or_else(eof)? {
            return Ok(Self {
                nodes: vec![Node::Leaf(0)],
                recent: [0; 3],
            });
        }
        let mut bytes = Vec::with_capacity(2);
        for _ in 0..2 {
            if bits.bit().ok_or_else(eof)? {
                bytes.push(read_byte_tree(bits)?);
                bits.bit().ok_or_else(eof)?;
            } else {
                bytes.push(vec![Node::Leaf(0)]);
            }
        }
        let mut escapes = [0u16; 3];
        for escape in &mut escapes {
            *escape = bits.bits(16).ok_or_else(eof)? as u16;
        }
        let mut tree = Self {
            nodes: Vec::new(),
            recent: [0; 3],
        };
        let mut slots = [None::<usize>; 3];
        tree.read_node(bits, &bytes, &escapes, &mut slots, 0)?;
        bits.bit().ok_or_else(eof)?;
        Ok(tree)
    }

    fn read_node(
        &mut self,
        bits: &mut BitReader,
        bytes: &[Vec<Node<u8>>],
        escapes: &[u16; 3],
        slots: &mut [Option<usize>; 3],
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_TREE_DEPTH || self.nodes.len() > 1 << 20 {
            return Err(SmackerError::InvalidTree("big tree too deep"));
        }
        let eof = || SmackerError::InvalidTree("truncated big tree");
        if bits.bit().ok_or_else(eof)? {
            let at = self.nodes.len();
            self.nodes.push(Node::Branch(0));
            self.read_node(bits, bytes, escapes, slots, depth + 1)?;
            self.nodes[at] = Node::Branch(self.nodes.len() as u32);
            return self.read_node(bits, bytes, escapes, slots, depth + 1);
        }
        let mut half = [0u16; 2];
        for (value, tree) in half.iter_mut().zip(bytes) {
            match walk(tree, bits).ok_or_else(eof)? {
                Node::Leaf(v) => *value = u16::from(v),
                _ => unreachable!("byte trees have no escapes"),
            }
        }
        let value = half[0] | half[1] << 8;
        let leaf = match escapes.iter().position(|&e| e == value) {
            Some(slot) => {
                // A later leaf with the same escape takes over the slot; the
                // earlier one keeps the constant 0 it was initialised with.
                if let Some(old) = slots[slot].replace(self.nodes.len()) {
                    self.nodes[old] = Node::Leaf(0);
                }
                Node::Escape(slot as u8)
            }
            None => Node::Leaf(value),
        };
        self.nodes.push(leaf);
        Ok(())
    }

    fn get(&mut self, bits: &mut BitReader) -> Option<u16> {
        let value = match walk(&self.nodes, bits)? {
            Node::Leaf(v) => v,
            Node::Escape(slot) => self.recent[slot as usize],
            Node::Branch(_) => unreachable!(),
        };
        if value != self.recent[0] {
            self.recent = [value, self.recent[0], self.recent[1]];
        }
        Some(value)
    }
}

/// Sequential frame decoder.
pub struct Decoder<'s, 'a> {
    smk: &'s Smacker<'a>,
    trees: [BigTree; 4],
    frame: Frame,
    next: usize,
    consumed_bits: usize,
}

impl<'s, 'a> Decoder<'s, 'a> {
    fn new(smk: &'s Smacker<'a>) -> Result<Self> {
        let mut bits = BitReader::new(smk.trees);
        let trees = [
            BigTree::read(&mut bits)?,
            BigTree::read(&mut bits)?,
            BigTree::read(&mut bits)?,
            BigTree::read(&mut bits)?,
        ];
        let size = (smk.header.width * smk.header.height) as usize;
        Ok(Self {
            smk,
            trees,
            frame: Frame {
                palette: [[0; 3]; 256],
                pixels: vec![0; size],
            },
            next: 0,
            consumed_bits: 0,
        })
    }

    /// Index of the next stored frame.
    pub fn position(&self) -> usize {
        self.next
    }

    /// Decodes the next stored frame (including the ring frame).
    pub fn next_frame(&mut self) -> Result<Option<&Frame>> {
        let Some(parts) = self.smk.frames.get(self.next) else {
            return Ok(None);
        };
        let index = self.next;
        self.next += 1;
        if let Some(chunk) = parts.palette {
            apply_palette(&mut self.frame.palette, &chunk[1..]).ok_or(
                SmackerError::InvalidFrame {
                    frame: index,
                    reason: "invalid palette",
                },
            )?;
        }
        self.decode_video(parts.video)
            .ok_or(SmackerError::InvalidFrame {
                frame: index,
                reason: "truncated video",
            })?;
        Ok(Some(&self.frame))
    }

    fn decode_video(&mut self, data: &[u8]) -> Option<()> {
        let mut bits = BitReader::new(data);
        for tree in &mut self.trees {
            tree.recent = [0; 3];
        }
        let [mmap, mclr, full, kind] = &mut self.trees;
        let width = self.smk.header.width as usize;
        let bw = width / 4;
        let blocks = bw * self.smk.header.height as usize / 4;
        let pixels = &mut self.frame.pixels;
        let mut block = 0;
        while block < blocks {
            let code = kind.get(&mut bits)?;
            let run = BLOCK_RUNS[((code >> 2) & 0x3F) as usize];
            let end = (block + run).min(blocks);
            for b in block..end {
                let origin = (b / bw) * width * 4 + (b % bw) * 4;
                match code & 3 {
                    MONO => {
                        let colors = mclr.get(&mut bits)?;
                        let mut map = mmap.get(&mut bits)?;
                        let (hi, lo) = ((colors >> 8) as u8, colors as u8);
                        for y in 0..4 {
                            for x in 0..4 {
                                pixels[origin + y * width + x] = if map & 1 != 0 { hi } else { lo };
                                map >>= 1;
                            }
                        }
                    }
                    FULL => {
                        for y in 0..4 {
                            let row = origin + y * width;
                            let right = full.get(&mut bits)?;
                            let left = full.get(&mut bits)?;
                            pixels[row + 2..row + 4].copy_from_slice(&right.to_le_bytes());
                            pixels[row..row + 2].copy_from_slice(&left.to_le_bytes());
                        }
                    }
                    SKIP => {}
                    _ => {
                        let color = (code >> 8) as u8;
                        for y in 0..4 {
                            let row = origin + y * width;
                            pixels[row..row + 4].fill(color);
                        }
                    }
                }
            }
            block = end;
        }
        self.consumed_bits = bits.pos;
        Some(())
    }
}

/// Applies a palette delta (the chunk after its size byte).
fn apply_palette(palette: &mut Palette, chunk: &[u8]) -> Option<()> {
    let old = *palette;
    let mut bytes = chunk.iter().copied();
    let mut index = 0usize;
    while index < 256 {
        let t = bytes.next()?;
        if t & 0x80 != 0 {
            index += (t & 0x7F) as usize + 1;
        } else if t & 0x40 != 0 {
            let offset = bytes.next()? as usize;
            let count = (t & 0x3F) as usize + 1;
            if offset + count > 256 {
                return None;
            }
            for i in 0..count {
                if index >= 256 {
                    break;
                }
                palette[index] = old[offset + i];
                index += 1;
            }
        } else {
            let g = bytes.next()?;
            let b = bytes.next()?;
            palette[index] = [
                SMK_PAL[(t & 0x3F) as usize],
                SMK_PAL[(g & 0x3F) as usize],
                SMK_PAL[(b & 0x3F) as usize],
            ];
            index += 1;
        }
    }
    Some(())
}

// ---------------------------------------------------------------- encoder

struct BitWriter {
    out: Vec<u8>,
    bits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            bits: 0,
        }
    }

    fn bit(&mut self, bit: bool) {
        if self.bits == 0 {
            self.out.push(0);
        }
        if bit {
            *self.out.last_mut().unwrap() |= 1 << self.bits;
        }
        self.bits = (self.bits + 1) & 7;
    }

    fn bits(&mut self, value: u32, count: u32) {
        for i in 0..count {
            self.bit((value >> i) & 1 != 0);
        }
    }

    fn code(&mut self, &(code, len): &(u32, u8)) {
        for i in (0..len).rev() {
            self.bit((code >> i) & 1 != 0);
        }
    }

    fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// Huffman code lengths for `weights` (all > 0), limited to `limit` bits.
fn code_lengths(weights: &[u64], limit: u8) -> Vec<u8> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    if weights.len() == 1 {
        return vec![0];
    }
    let mut weights = weights.to_vec();
    loop {
        let n = weights.len();
        let mut parent = vec![usize::MAX; 2 * n - 1];
        let mut heap: BinaryHeap<Reverse<(u64, usize)>> = weights
            .iter()
            .enumerate()
            .map(|(i, &w)| Reverse((w, i)))
            .collect();
        let mut next = n;
        while heap.len() > 1 {
            let Reverse((wa, a)) = heap.pop().unwrap();
            let Reverse((wb, b)) = heap.pop().unwrap();
            parent[a] = next;
            parent[b] = next;
            heap.push(Reverse((wa + wb, next)));
            next += 1;
        }
        let mut depth = vec![0u8; 2 * n - 1];
        for i in (0..2 * n - 2).rev() {
            depth[i] = depth[parent[i]] + 1;
        }
        let lengths = depth[..n].to_vec();
        if lengths.iter().all(|&l| l <= limit) {
            return lengths;
        }
        for w in &mut weights {
            *w = (*w >> 1) + 1;
        }
    }
}

/// A canonical prefix code over sorted symbols, with its serialised tree.
struct Code<T> {
    symbols: Vec<T>,
    codes: Vec<(u32, u8)>,
}

impl<T: Copy + Ord> Code<T> {
    fn new(mut weighted: Vec<(T, u64)>, limit: u8) -> Self {
        weighted.sort_by_key(|&(s, _)| s);
        let weights: Vec<u64> = weighted.iter().map(|&(_, w)| w.max(1)).collect();
        let lengths = code_lengths(&weights, limit);
        let mut order: Vec<usize> = (0..weighted.len()).collect();
        order.sort_by_key(|&i| (lengths[i], weighted[i].0));
        let mut codes = vec![(0u32, 0u8); weighted.len()];
        let mut code = 0u32;
        let mut previous = 0u8;
        for (n, &i) in order.iter().enumerate() {
            if n > 0 {
                code = (code + 1) << (lengths[i] - previous);
            } else {
                code <<= lengths[i];
            }
            previous = lengths[i];
            codes[i] = (code, lengths[i]);
        }
        Self {
            symbols: weighted.into_iter().map(|(s, _)| s).collect(),
            codes,
        }
    }

    fn get(&self, symbol: T) -> (u32, u8) {
        self.codes[self.symbols.binary_search(&symbol).expect("symbol in code")]
    }

    /// Writes the tree in pre-order (1 = branch, 0 = leaf + payload).
    fn write_tree(&self, out: &mut BitWriter, leaf: &mut dyn FnMut(&mut BitWriter, T)) {
        let mut order: Vec<usize> = (0..self.symbols.len()).collect();
        order.sort_by_key(|&i| {
            let (code, len) = self.codes[i];
            // Left-aligned code orders leaves by pre-order position.
            (u64::from(code) << (40 - u32::from(len)), len)
        });
        let mut prefix_len = 0u8;
        let mut first = true;
        for &i in &order {
            let (code, len) = self.codes[i];
            // Branch bits for every new level between the shared prefix and this leaf.
            let shared = if first { 0 } else { prefix_len };
            for _ in shared..len {
                out.bit(true);
            }
            out.bit(false);
            leaf(out, self.symbols[i]);
            first = false;
            // The next leaf hangs off the deepest 0-branch on this path.
            let trailing_ones = (!code).trailing_zeros().min(u32::from(len)) as u8;
            prefix_len = len - trailing_ones;
        }
    }
}

/// Classified 4x4 block.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Block {
    Skip,
    Fill(u8),
    Mono { colors: u16, map: u16 },
    Full([u16; 8]),
}

impl Block {
    fn kind(self) -> (u16, u16) {
        match self {
            Block::Mono { .. } => (MONO, 0),
            Block::Full(_) => (FULL, 0),
            Block::Skip => (SKIP, 0),
            Block::Fill(c) => (FILL, u16::from(c)),
        }
    }
}

fn classify(pixels: &[u8], previous: Option<&[u8]>, width: usize, origin: usize) -> Block {
    let mut block = [0u8; 16];
    for y in 0..4 {
        block[y * 4..y * 4 + 4]
            .copy_from_slice(&pixels[origin + y * width..origin + y * width + 4]);
    }
    if let Some(previous) = previous
        && (0..4).all(|y| {
            let row = origin + y * width;
            previous[row..row + 4] == block[y * 4..y * 4 + 4]
        })
    {
        return Block::Skip;
    }
    let lo = *block.iter().min().unwrap();
    let hi = *block.iter().max().unwrap();
    if lo == hi {
        return Block::Fill(lo);
    }
    if block.iter().all(|&p| p == lo || p == hi) {
        let map = block
            .iter()
            .enumerate()
            .fold(0u16, |m, (i, &p)| m | (u16::from(p == hi) << i));
        return Block::Mono {
            colors: u16::from(hi) << 8 | u16::from(lo),
            map,
        };
    }
    let mut full = [0u16; 8];
    for y in 0..4 {
        let row = &block[y * 4..y * 4 + 4];
        full[y * 2] = u16::from_le_bytes([row[2], row[3]]);
        full[y * 2 + 1] = u16::from_le_bytes([row[0], row[1]]);
    }
    Block::Full(full)
}

/// Symbols of one frame, per stream: MMAP, MCLR, FULL, TYPE.
#[derive(Default)]
struct FrameSymbols {
    tokens: Vec<(usize, u16)>,
}

fn frame_symbols(blocks: &[Block]) -> FrameSymbols {
    let mut out = FrameSymbols::default();
    let mut i = 0;
    while i < blocks.len() {
        let kind = blocks[i].kind();
        let mut n = blocks[i..].iter().take_while(|b| b.kind() == kind).count();
        while n > 0 {
            let run_index = BLOCK_RUNS.iter().rposition(|&r| r <= n).unwrap();
            let run = BLOCK_RUNS[run_index];
            out.tokens
                .push((3, kind.0 | (run_index as u16) << 2 | kind.1 << 8));
            for block in &blocks[i..i + run] {
                match *block {
                    Block::Mono { colors, map } => {
                        out.tokens.push((1, colors));
                        out.tokens.push((0, map));
                    }
                    Block::Full(values) => out.tokens.extend(values.iter().map(|&v| (2, v))),
                    _ => {}
                }
            }
            i += run;
            n -= run;
        }
    }
    out
}

/// Encoded big tree: header bits plus the code used for frame data.
struct BigCode {
    code: Option<Code<u16>>,
}

fn write_big_tree(out: &mut BitWriter, counts: &std::collections::HashMap<u16, u64>) -> BigCode {
    if counts.is_empty() {
        out.bit(false);
        return BigCode { code: None };
    }
    out.bit(true);
    // Three escape values that never occur, so escape leaves are never
    // needed; they are still placed in the tree like the original encoder.
    let mut escapes = Vec::with_capacity(3);
    let mut candidate = 0u32;
    while escapes.len() < 3 {
        let value = candidate as u16;
        if !counts.contains_key(&value) {
            escapes.push(value);
        }
        candidate += 1;
    }
    let mut weighted: Vec<(u16, u64)> = counts.iter().map(|(&s, &w)| (s, w)).collect();
    weighted.extend(escapes.iter().map(|&e| (e, 0)));
    let code = Code::new(weighted, 24);
    let mut low = std::collections::HashMap::<u8, u64>::new();
    let mut high = std::collections::HashMap::<u8, u64>::new();
    for &symbol in &code.symbols {
        *low.entry(symbol as u8).or_default() += 1;
        *high.entry((symbol >> 8) as u8).or_default() += 1;
    }
    let mut byte_codes = Vec::with_capacity(2);
    for bytes in [low, high] {
        out.bit(true);
        let byte_code = Code::new(bytes.into_iter().collect(), 16);
        byte_code.write_tree(out, &mut |w, v| w.bits(u32::from(v), 8));
        out.bit(false);
        byte_codes.push(byte_code);
    }
    for &escape in &escapes {
        out.bits(u32::from(escape), 16);
    }
    code.write_tree(out, &mut |w, v| {
        w.code(&byte_codes[0].get(v as u8));
        w.code(&byte_codes[1].get((v >> 8) as u8));
    });
    out.bit(false);
    BigCode { code: Some(code) }
}

/// Re-encodes the video of `original` with new 8-bit frames.
///
/// `frames` must hold one `width * height` index buffer per frame, ring
/// frame excluded (it repeats the first frame). Palette and audio chunks,
/// frame types and key-frame flags are copied from `original`.
pub fn encode(original: &Smacker, frames: &[Vec<u8>]) -> Result<Vec<u8>> {
    use std::collections::HashMap;
    let header = &original.header;
    let (width, height) = (header.width as usize, header.height as usize);
    if frames.len() != header.frames as usize {
        return Err(SmackerError::Encode(format!(
            "expected {} frames, got {}",
            header.frames,
            frames.len()
        )));
    }
    if let Some(bad) = frames.iter().position(|f| f.len() != width * height) {
        return Err(SmackerError::Encode(format!(
            "frame {bad} is not {width}x{height}"
        )));
    }
    let mut sequence: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    if header.has_ring_frame() {
        sequence.push(&frames[0]);
    }
    let bw = width / 4;
    let blocks = bw * height / 4;
    let mut symbols = Vec::with_capacity(sequence.len());
    for (index, pixels) in sequence.iter().enumerate() {
        let previous = (!original.is_key_frame(index)).then(|| sequence[index - 1]);
        let classified: Vec<Block> = (0..blocks)
            .map(|b| classify(pixels, previous, width, (b / bw) * width * 4 + (b % bw) * 4))
            .collect();
        symbols.push(frame_symbols(&classified));
    }
    let mut counts: [HashMap<u16, u64>; 4] = Default::default();
    for frame in &symbols {
        for &(stream, value) in &frame.tokens {
            *counts[stream].entry(value).or_default() += 1;
        }
    }
    let mut tree_bits = BitWriter::new();
    let codes: Vec<BigCode> = counts
        .iter()
        .map(|c| write_big_tree(&mut tree_bits, c))
        .collect();
    let mut trees = tree_bits.finish();
    while !trees.len().is_multiple_of(4) {
        trees.push(0);
    }
    let mut table_sizes = [0u32; 4];
    for (size, code) in table_sizes.iter_mut().zip(&codes) {
        // Table entries are nodes plus leaves plus three escape slots.
        let leaves = code.code.as_ref().map_or(1, |c| c.symbols.len());
        *size = ((2 * leaves - 1 + 3) * 4) as u32;
    }

    let mut out = original.raw_header.to_vec();
    out[52..56].copy_from_slice(&(trees.len() as u32).to_le_bytes());
    for (i, size) in table_sizes.iter().enumerate() {
        out[56 + i * 4..60 + i * 4].copy_from_slice(&size.to_le_bytes());
    }
    let mut bodies = Vec::with_capacity(sequence.len());
    for (index, frame) in symbols.iter().enumerate() {
        let parts = &original.frames[index];
        let mut body = Vec::new();
        if let Some(palette) = parts.palette {
            body.extend_from_slice(palette);
        }
        for chunk in &parts.audio {
            body.extend_from_slice(chunk);
        }
        let mut video = BitWriter::new();
        for &(stream, value) in &frame.tokens {
            let code = codes[stream].code.as_ref().expect("stream has a tree");
            video.code(&code.get(value));
        }
        body.extend(video.finish());
        while body.len() % 4 != 0 {
            body.push(0);
        }
        bodies.push(body);
    }
    for (index, body) in bodies.iter().enumerate() {
        let size = body.len() as u32 | (original.frame_sizes[index] & 3);
        out.extend_from_slice(&size.to_le_bytes());
    }
    out.extend_from_slice(&original.frame_types);
    out.extend(trees);
    for body in bodies {
        out.extend(body);
    }
    Ok(out)
}

/// Round-trips every animation of a real install (`LIBDAY_ROOT`); slow, so ignored by default.
#[cfg(test)]
mod real_data {
    use super::*;

    #[test]
    #[ignore = "needs LIBDAY_ROOT; run with --ignored --release"]
    fn every_unit_animation_re_encodes_losslessly() {
        let Ok(root) = std::env::var("LIBDAY_ROOT") else {
            return;
        };
        let data = std::fs::read(format!("{root}/ANMSUNIT/ANMSUNIT.FF")).unwrap();
        let mff = crate::mff::Mff::parse(&data).unwrap();
        for entry in mff.entries() {
            let smk = Smacker::parse(mff.get(&entry.name).unwrap()).unwrap();
            let frames = smk.decode_all().unwrap();
            let pixels: Vec<Vec<u8>> = frames.iter().map(|f| f.pixels.clone()).collect();
            let again = Smacker::parse(&encode(&smk, &pixels).unwrap())
                .unwrap()
                .decode_all()
                .unwrap();
            assert_eq!(again.len(), frames.len(), "{}", entry.name);
            for (a, b) in again.iter().zip(&frames) {
                assert!(
                    a.pixels == b.pixels && a.palette == b.palette,
                    "{}",
                    entry.name
                );
            }
        }
    }

    #[test]
    #[ignore = "needs LIBDAY_ROOT; run with --ignored --release --nocapture"]
    fn every_unit_animation_audio_decodes() {
        let Ok(root) = std::env::var("LIBDAY_ROOT") else {
            return;
        };
        let data = std::fs::read(format!("{root}/ANMSUNIT/ANMSUNIT.FF")).unwrap();
        let mff = crate::mff::Mff::parse(&data).unwrap();
        let mut by_kind = std::collections::BTreeMap::<char, (usize, usize)>::new();
        for entry in mff.entries() {
            let smk = Smacker::parse(mff.get(&entry.name).unwrap()).unwrap();
            let audio = smk.decode_audio(0).unwrap();
            let stem = entry.name.trim_end_matches(".SMK");
            let kind = stem.chars().rev().nth(1).unwrap_or('?');
            let slot = by_kind.entry(kind).or_default();
            slot.0 += 1;
            if audio.is_some_and(|a| a.samples.iter().any(|&s| s.unsigned_abs() > 512)) {
                slot.1 += 1;
            }
        }
        println!("{by_kind:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_dpcm_audio_chunk() {
        // 8-bit mono, four samples: start at 0x90, deltas from a two-leaf
        // tree (0 -> +1, 1 -> -2).
        let mut w = BitWriter::new();
        w.bits(0b001, 3); // data, mono, 8-bit
        w.bit(true); // tree present
        w.bit(true); // branch
        w.bit(false);
        w.bits(1, 8);
        w.bit(false);
        w.bits(0xFE, 8);
        w.bit(false); // tree end
        w.bits(0x90, 8);
        w.bit(false);
        w.bit(true);
        w.bit(false);
        let mut chunk = 4u32.to_le_bytes().to_vec();
        chunk.extend(w.finish());
        let mut out = Vec::new();
        decode_audio_chunk(&chunk, false, false, &mut out).unwrap();
        let bytes: Vec<u8> = out.iter().map(|&s| ((s >> 8) + 128) as u8).collect();
        assert_eq!(bytes, [0x90, 0x91, 0x8F, 0x90]);
    }
    #[test]
    fn dominant_colour_is_most_frequent_rgb() {
        let mut palette = [[0u8; 3]; 256];
        palette[7] = [255, 0, 174];
        palette[9] = [10, 20, 30];
        let frame = Frame {
            palette,
            pixels: vec![9, 7, 7, 7, 9, 0],
        };
        assert_eq!(frame.dominant_colour(), [255, 0, 174]);
    }

    #[test]
    fn key_colour_follows_a_background_switch() {
        let mut palette = [[0u8; 3]; 256];
        palette[1] = [174, 77, 0];
        palette[2] = [93, 93, 93];
        palette[3] = [255, 255, 255];
        let frame = |pixels: Vec<u8>| Frame { palette, pixels };
        let frames = [
            frame(vec![1, 1, 1, 3]),
            // Mostly explosion, but the background is still there.
            frame(vec![3, 3, 3, 1]),
            frame(vec![2, 2, 2, 3]),
            frame(vec![3, 2, 3, 3]),
        ];
        assert_eq!(
            key_colours(&frames),
            [[174, 77, 0], [174, 77, 0], [93, 93, 93], [93, 93, 93]]
        );
    }

    #[test]
    fn huffman_lengths_respect_limit() {
        let weights: Vec<u64> = (0..40).map(|i| 1u64 << i.min(30)).collect();
        let lengths = code_lengths(&weights, 12);
        assert!(lengths.iter().all(|&l| l <= 12));
        let kraft: f64 = lengths.iter().map(|&l| 0.5f64.powi(l as i32)).sum();
        assert!((kraft - 1.0).abs() < 1e-9);
    }

    #[test]
    fn byte_tree_round_trip() {
        let code = Code::new(vec![(5u8, 10), (9, 1), (200, 3), (7, 7)], 16);
        let mut w = BitWriter::new();
        code.write_tree(&mut w, &mut |w, v| w.bits(u32::from(v), 8));
        for symbol in [5u8, 9, 200, 7, 5] {
            w.code(&code.get(symbol));
        }
        let data = w.finish();
        let mut r = BitReader::new(&data);
        let tree = read_byte_tree(&mut r).unwrap();
        for symbol in [5u8, 9, 200, 7, 5] {
            assert_eq!(walk(&tree, &mut r), Some(Node::Leaf(symbol)));
        }
    }

    /// Builds a minimal SMK2 shell (header, frame tables, empty frames)
    /// to serve as the template for `encode`.
    fn template(width: u32, height: u32, frames: u32, ring: bool) -> Vec<u8> {
        let total = frames as usize + usize::from(ring);
        let mut data = vec![0u8; HEADER];
        data[..4].copy_from_slice(b"SMK2");
        data[4..8].copy_from_slice(&width.to_le_bytes());
        data[8..12].copy_from_slice(&height.to_le_bytes());
        data[12..16].copy_from_slice(&frames.to_le_bytes());
        data[16..20].copy_from_slice(&(-6666i32).to_le_bytes());
        data[20..24].copy_from_slice(&u32::from(ring).to_le_bytes());
        // Frame 0 carries a palette chunk: 4 entries then a skip to 256.
        let mut palette = vec![0u8];
        for i in 0..4u8 {
            palette.extend_from_slice(&[i * 10, i * 5, 63 - i]);
        }
        palette.extend_from_slice(&[0xFF, 0xFB]);
        while palette.len() % 4 != 0 {
            palette.push(0);
        }
        palette[0] = (palette.len() / 4) as u8;
        let audio = [8u8, 0, 0, 0, 1, 2, 3, 4];
        for i in 0..total {
            let size = if i == 0 { palette.len() + 8 } else { 8 };
            data.extend_from_slice(&(size as u32).to_le_bytes());
        }
        for i in 0..total {
            data.push(if i == 0 { 1 | 2 } else { 2 });
        }
        for i in 0..total {
            if i == 0 {
                data.extend_from_slice(&palette);
            }
            data.extend_from_slice(&audio);
        }
        data
    }

    #[test]
    fn encode_decode_round_trip() {
        let (w, h) = (16u32, 8u32);
        let template = template(w, h, 3, true);
        let original = Smacker::parse(&template).unwrap();
        let size = (w * h) as usize;
        let mut frames = vec![vec![0u8; size]; 3];
        for (i, pixel) in frames[0].iter_mut().enumerate() {
            *pixel = (i % 7) as u8 * 3;
        }
        frames[1] = frames[0].clone();
        frames[1][..4].fill(9);
        frames[1][16..20].copy_from_slice(&[1, 2, 1, 2]);
        frames[2] = vec![200; size];
        let encoded = encode(&original, &frames).unwrap();
        let smk = Smacker::parse(&encoded).unwrap();
        assert!(smk.has_audio());
        let decoded = smk.decode_all().unwrap();
        for (frame, expected) in decoded.iter().zip(&frames) {
            assert_eq!(&frame.pixels, expected);
        }
        assert_eq!(
            decoded[0].palette[1],
            [SMK_PAL[10], SMK_PAL[5], SMK_PAL[62]]
        );
        let mut decoder = smk.decoder().unwrap();
        for _ in 0..3 {
            decoder.next_frame().unwrap();
        }
        assert_eq!(decoder.next_frame().unwrap().unwrap().pixels, frames[0]);
        assert!(decoder.next_frame().unwrap().is_none());
        assert_eq!(encode(&smk, &frames).unwrap(), encoded);
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(
            Smacker::parse(b"SMK2").unwrap_err(),
            SmackerError::Truncated
        );
        let mut data = template(16, 8, 1, false);
        data[..4].copy_from_slice(b"SMK4");
        assert_eq!(
            Smacker::parse(&data).unwrap_err(),
            SmackerError::BadSignature
        );
    }
}
