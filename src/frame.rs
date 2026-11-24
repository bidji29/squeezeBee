//! The frame: what a message is, and the loop that codes the data one bit at a time.
//!
//! A message is the length of the transformed text (four bytes, the low one first), one byte that says which
//! weights the last step works with, then the arithmetic coder's bytes. For each bit, in the same order when encoding and when decoding: every model predicts, the mixer makes
//! one prediction of theirs, the refining step has a last look, the coder codes the bit with that probability, and
//! all of them are told the bit. The coder's probability is out of 65536, and learnt (see STEP).
//!
//! The frame's last step adds two corrections to what it learnt: the refining step's, and its own, from the six
//! bytes before (see DEEP_BITS). How much each one counts is not written here: the encoder takes two passes. It
//! first codes the start of the text for nobody, counting what every pair of weights of a small grid would have
//! cost, and then codes the whole text with the cheapest pair, whose number is the fifth byte of the message.

use alloc::vec;
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

/// The six bytes before, with the bits of this byte, choose a line of DEEP_CUTS cuts among 2^DEEP_BITS (a hash:
/// two contexts that land on the same line share it). A line says what the mixed prediction comes to in that
/// context: the stretched scale is cut every 2^DEEP_STEP steps (few cuts: a line is seldom seen, and must learn
/// fast), each cut a probability out of 2^22 with, in the low ten bits, how many bits it has learnt from, as
/// the refining step keeps its own. What the line says, less what the frame learnt without a context, is the
/// frame's own correction.
const DEEP_BITS: u32 = 23;
const DEEP_STEP: u32 = 9;
const DEEP_CUTS: usize = (4096 >> DEEP_STEP) + 1;

/// A cut that has seen n bits moves 1 / (n + 1.5) of the way, times its share of the line, until n is DEEP_LIMIT.
const DEEP_LIMIT: u32 = 255;

/// The refining step's say is averaged with the mixer's own, on the stretched scale: the refined prediction
/// counts for SHARE_REFINED eighths, the unrefined for the rest, and the average is squashed back.
const SHARE_REFINED: i32 = 7;

/// The stretched scale in finer steps, for that average: 2^FINE of them for each of its own.
const FINE: u32 = 4;

/// The pairs of weights the first pass tries: CHOICES of them, four weights for each correction.
const CHOICES: usize = 16;

/// The first pass reads one byte in SHARE of the text, from its start.
const SHARE: usize = 4;

/// The weights of a choice: the refining step's correction (steps of 1/4096) counts for the first, the frame's
/// own (steps of 1/65536) for the second, in sixteenths.
fn weights(choice: usize) -> (i32, i32) {
    (10 + 2 * (choice & 3) as i32, 8 + 2 * ((choice >> 2) & 3) as i32)
}

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
    /// The lines of the six bytes before, and one over n + 1.5, out of 2^16, for each count.
    deep: Vec<u32>,
    over: Vec<u32>,
    /// For the bit being coded: where its line's first cut is, and how far past it the prediction was.
    deep_at: usize,
    deep_part: u32,
    /// For the bit being coded: what the frame learnt, and the two corrections.
    seen: i32,
    refined: i32,
    deeper: i32,
    /// The weights of the two corrections.
    choice: usize,
    /// The stretched scale of a probability out of 65536, in steps of 1/2^FINE of its own (see FINE).
    logit: Vec<i32>,
}

impl Predictor {
    fn new(choice: usize) -> Self {
        let cx = Context::new();
        let mut deep = vec![0u32; DEEP_CUTS << DEEP_BITS];
        for (at, slot) in deep.iter_mut().enumerate() {
            *slot = cx.scale.squash_fine((((at % DEEP_CUTS) as i32) << DEEP_STEP) - 2048) << 16 | 2;
        }
        let mut over = vec![0u32; DEEP_LIMIT as usize + 1];
        for (n, slot) in over.iter_mut().enumerate() {
            *slot = (1 << 17) / (2 * n as u32 + 3);
        }
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
            deep,
            over,
            deep_at: 0,
            deep_part: 0,
            seen: 0,
            refined: 0,
            deeper: 0,
            choice,
            logit: logits(&cx_scale_fine()),
        }
    }

    /// The probability, out of 65536, of a point of the stretched scale in steps of 1/2^FINE: the line between the
    /// two squashed steps around it.
    fn squash_finer(&self, x: i32) -> i32 {
        let x = x.clamp(-2048 << FINE, (2047 << FINE) - 1);
        let d = x >> FINE;
        let part = x & ((1 << FINE) - 1);
        let lo = self.cx.scale.squash_fine(d) as i32;
        let hi = self.cx.scale.squash_fine(d + 1) as i32;
        lo + (((hi - lo) * part + (1 << (FINE - 1))) >> FINE)
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
        self.seen = ((below + above) >> (STEP + 14)) as i32;
        // The average, on the stretched scale, of the refined prediction and the mixer's own: what the refining
        // step moved, stretched, is given its share, added to the mixed prediction, and squashed back.
        let at = |p: u32| self.logit[(p.clamp(1, 4095) << 4 | 8) as usize];
        let moved = at(refined) - at(coarse);
        let base = mixed.clamp(-2048, 2047) << FINE;
        let averaged = self.squash_finer(base + moved * SHARE_REFINED / 8) - self.squash_finer(base);
        // In steps of 1/65536, where the weights of a choice take steps of 1/4096: they count it in sixteenths.
        self.refined = averaged;
        let hash = ((cx_six(&self.cx).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32)
            ^ self.cx.partial.wrapping_mul(0xC2B2_AE35);
        self.deep_at = (hash >> (32 - DEEP_BITS)) as usize * DEEP_CUTS + (d >> DEEP_STEP) as usize;
        self.deep_part = d & ((1 << DEEP_STEP) - 1);
        let below = (self.deep[self.deep_at] >> 16) * ((1 << DEEP_STEP) - self.deep_part);
        let above = (self.deep[self.deep_at + 1] >> 16) * self.deep_part;
        self.deeper = ((below + above) >> DEEP_STEP) as i32 - self.seen;
        self.weigh(self.choice)
    }

    /// The probability for the bit to come, had the corrections the weights of this choice.
    fn weigh(&self, choice: usize) -> u32 {
        let (refined, deeper) = weights(choice);
        (self.seen + (self.refined * refined + self.deeper * deeper) / 16).clamp(1, 65535) as u32
    }

    fn update(&mut self, bit: u32) {
        self.models.update(&self.cx, bit);
        self.mixer.update(&self.cx, bit);
        self.refine.update(&self.cx, bit);
        let near = if self.part * 2 >= (1 << STEP) { self.node + 1 } else { self.node };
        let t = self.learnt[near];
        self.learnt[near] = if bit == 1 { t + (((1 << 30) - t) >> RATE) } else { t - (t >> RATE) };
        let whole = 1i64 << DEEP_STEP;
        let part = self.deep_part as i64;
        let target = if bit == 1 { (1i64 << 22) - 1 } else { 0 };
        for (slot, share) in self.deep[self.deep_at..self.deep_at + 2].iter_mut().zip([whole - part, part]) {
            let count = *slot & 1023;
            let p = (*slot >> 10) as i64;
            let p = p + (((target - p) * self.over[count as usize] as i64 * share) >> (16 + DEEP_STEP));
            *slot = (p as u32) << 10 | if count < DEEP_LIMIT { count + 1 } else { count };
        }
        self.cx.push(bit);
    }
}

/// The squashed scale out of 65536, step by step (-2048 to 2047).
fn cx_scale_fine() -> Vec<u32> {
    let scale = crate::logistic::Scale::new();
    (-2048..2048).map(|d| scale.squash_fine(d)).collect()
}

/// For each probability out of 65536, its point on the stretched scale in steps of 1/2^FINE: between the two
/// steps whose squashed probabilities hold it, the line between them.
fn logits(fine: &[u32]) -> Vec<i32> {
    let mut table = vec![0i32; 65536];
    let mut d = 0usize;
    for (p, slot) in table.iter_mut().enumerate() {
        let p = p as u32;
        while d + 1 < fine.len() && fine[d + 1] <= p {
            d += 1;
        }
        let lo = fine[d];
        let point = ((d as i32) - 2048) << FINE;
        *slot = if d + 1 < fine.len() && fine[d + 1] > lo && p >= lo {
            point + (((p - lo) << FINE) / (fine[d + 1] - lo)) as i32
        } else {
            point
        };
    }
    table
}

/// The six whole bytes before.
fn cx_six(cx: &Context) -> u64 {
    cx.last8 & 0xFFFF_FFFF_FFFF
}

/// What a bit costs, in 2^-16 of a bit, when it was given the probability p out of 65536 (index p; 0 is not
/// asked): the logarithm, with the four operations alone, a binary digit at each squaring.
fn costs() -> Vec<u32> {
    let mut table = vec![0u32; 65536];
    for (p, slot) in table.iter_mut().enumerate().skip(1) {
        let mut x = p as f64;
        let mut cost = 0u32;
        while x < 32768.0 {
            x *= 2.0;
            cost += 1 << 16;
        }
        x /= 32768.0;
        cost += 1 << 16;
        for digit in (0..16).rev() {
            x *= x;
            if x >= 2.0 {
                x /= 2.0;
                cost -= 1 << digit;
            }
        }
        *slot = cost;
    }
    table
}

/// What one pass over the text does with each bit: count what every choice would cost (the first pass), code
/// it, or read it back.
enum Way<'a> {
    Measure(Vec<u32>, [u64; CHOICES]),
    Encode(Encoder),
    Decode(Decoder<'a>),
}

/// One pass over `size` bytes, bit by bit, in the same order for every way: the one loop the three passes share.
/// The text is read from `text`, or, when decoding (`text` is then empty), read back into `back`.
fn pass(text: &[u8], back: &mut Vec<u8>, size: usize, choice: usize, way: &mut Way) {
    let mut predictor = Predictor::new(choice);
    for at in 0..size {
        let known = if text.is_empty() { 0 } else { text[at] as u32 };
        let mut byte = 0;
        for shift in (0..8).rev() {
            let p = predictor.predict(if text.is_empty() { back } else { &text[..at] });
            let mut bit = (known >> shift) & 1;
            match way {
                Way::Measure(costs, spent) => {
                    for (choice, sum) in spent.iter_mut().enumerate() {
                        let p = predictor.weigh(choice);
                        *sum += costs[if bit == 1 { p } else { 65536 - p } as usize] as u64;
                    }
                }
                Way::Encode(coder) => coder.code(bit, p),
                Way::Decode(coder) => bit = coder.decode(p),
            }
            predictor.update(bit);
            byte = (byte << 1) | bit;
        }
        if text.is_empty() {
            back.push(byte as u8);
        }
    }
}

/// The message for the data. The first pass reads the start of the text and keeps the choice of weights that
/// would have coded it in the fewest bits.
pub fn encode(data: &[u8]) -> Vec<u8> {
    let text = transform::forward(data);
    let mut way = Way::Measure(costs(), [0; CHOICES]);
    pass(&text, &mut Vec::new(), text.len() / SHARE, 0, &mut way);
    let mut choice = 0;
    if let Way::Measure(_, spent) = way {
        for at in 1..CHOICES {
            if spent[at] < spent[choice] {
                choice = at;
            }
        }
    }
    let mut out = Vec::with_capacity(text.len() / 2 + 64);
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.push(choice as u8);
    let mut way = Way::Encode(Encoder::new(out));
    pass(&text, &mut Vec::new(), text.len(), choice, &mut way);
    match way {
        Way::Encode(coder) => coder.finish(),
        _ => Vec::new(),
    }
}

/// The data, from its message.
pub fn decode(message: &[u8]) -> Vec<u8> {
    if message.len() < 5 {
        return Vec::new();
    }
    let size = u32::from_le_bytes([message[0], message[1], message[2], message[3]]) as usize;
    let mut text = Vec::with_capacity(size);
    let mut way = Way::Decode(Decoder::new(&message[5..]));
    pass(&[], &mut text, size, message[4] as usize % CHOICES, &mut way);
    transform::inverse(text)
}
