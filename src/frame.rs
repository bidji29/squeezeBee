//! The frame: what a message is, and the loop that codes the data one bit at a time.
//!
//! A message is the length of the transformed text (four bytes, the low one first), then the arithmetic coder's
//! bytes. For each bit, in the same order when encoding and when decoding: every model predicts, the mixer makes
//! one prediction of theirs, the refining step has a last look, the coder codes the bit with that probability, and
//! all of them are told the bit. The coder's probability is out of 65536, and learnt (see STEP).

use alloc::vec::Vec;

use crate::coder::{Decoder, Encoder};
use crate::context::Context;
use crate::mixer::Mixer;
use crate::models;
use crate::refine::Refine;
use crate::transform;

/// What the stretched scale says and what comes true are not quite the same, least of all where the prediction
/// is nearly sure, and there a sixteen-bit probability is worth having only if it is the right one. So the frame
/// learns it: the stretched scale is cut every 2^STEP steps, and at each cut it keeps the probability, out of
/// 2^30, that a 1 came when the mixed prediction was nearest that cut. Between two cuts, the line between them.
const STEP: u32 = 6;
const NODES: usize = 4096 >> STEP;

/// Each bit moves its cut's probability 1 / 2^RATE of the way: slowly, there are bits enough.
const RATE: u32 = 12;

struct Predictor {
    cx: Context,
    models: models::Set,
    mixer: Mixer,
    refine: Refine,
    inputs: [i32; models::INPUTS],
    /// The learnt probabilities at the cuts, out of 2^30; they start as the scale's own.
    learnt: [u32; NODES + 1],
    /// The cut below the last mixed prediction, and how far above it the prediction was.
    node: usize,
    part: u32,
}

impl Predictor {
    fn new() -> Self {
        let cx = Context::new();
        let mut learnt = [0; NODES + 1];
        for (node, slot) in learnt.iter_mut().enumerate() {
            *slot = cx.scale.squash_fine(((node as i32) << STEP) - 2048) << 14;
        }
        Predictor {
            cx,
            models: models::Set::new(),
            mixer: Mixer::new(models::INPUTS),
            refine: Refine::new(),
            inputs: [0; models::INPUTS],
            learnt,
            node: 0,
            part: 0,
        }
    }

    /// The probability, out of 65536, that the bit to come is 1; `history` is the whole bytes coded so far.
    /// The refining step works out of 4096: what it changes is carried over to the finer probability.
    fn predict(&mut self, history: &[u8]) -> u32 {
        self.models.predict(&self.cx, history, &mut self.inputs);
        let mixed = self.mixer.mix(&self.cx, &self.inputs);
        let coarse = self.cx.scale.squash(mixed);
        let refined = self.refine.refine(&self.cx, coarse);
        let d = (mixed.clamp(-2048, 2047) + 2048) as u32;
        self.node = (d >> STEP) as usize;
        self.part = d & ((1 << STEP) - 1);
        let whole = 1u32 << STEP;
        let below = self.learnt[self.node] as u64 * (whole - self.part) as u64;
        let above = self.learnt[self.node + 1] as u64 * self.part as u64;
        let seen = ((below + above) >> (STEP + 14)) as u32;
        let p = seen as i32 + (refined as i32 - coarse as i32) * 16;
        p.clamp(1, 65535) as u32
    }

    fn update(&mut self, bit: u32) {
        self.models.update(&self.cx, bit);
        self.mixer.update(&self.cx, bit);
        self.refine.update(&self.cx, bit);
        let near = if self.part * 2 >= (1 << STEP) { self.node + 1 } else { self.node };
        let t = self.learnt[near];
        self.learnt[near] = if bit == 1 { t + (((1 << 30) - t) >> RATE) } else { t - (t >> RATE) };
        self.cx.push(bit);
    }
}

/// The message for the data.
pub fn encode(data: &[u8]) -> Vec<u8> {
    let text = transform::forward(data);
    let mut out = Vec::with_capacity(text.len() / 2 + 64);
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    let mut coder = Encoder::new(out);
    let mut predictor = Predictor::new();
    for (at, &byte) in text.iter().enumerate() {
        for shift in (0..8).rev() {
            let bit = ((byte >> shift) & 1) as u32;
            let p = predictor.predict(&text[..at]);
            coder.code(bit, p);
            predictor.update(bit);
        }
    }
    coder.finish()
}

/// The data, from its message.
pub fn decode(message: &[u8]) -> Vec<u8> {
    if message.len() < 4 {
        return Vec::new();
    }
    let size = u32::from_le_bytes([message[0], message[1], message[2], message[3]]) as usize;
    let mut text = Vec::with_capacity(size);
    let mut coder = Decoder::new(&message[4..]);
    let mut predictor = Predictor::new();
    for _ in 0..size {
        let mut byte = 0;
        for _ in 0..8 {
            let p = predictor.predict(&text);
            let bit = coder.decode(p);
            predictor.update(bit);
            byte = (byte << 1) | bit;
        }
        text.push(byte as u8);
    }
    transform::inverse(text)
}
