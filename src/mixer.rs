//! The mixer: makes one prediction of all the models' predictions for the bit to come.
//!
//! Each model has a weight, and the mixed prediction is the weighted sum of the models' stretched predictions.
//! After each bit, every weight moves by the error of the mixed prediction times what its model said: a model
//! that pushed the right way counts for more next time, one that pushed the wrong way for less.
//!
//! What a model is worth depends on where the coding stands, so there are several sets of weights, each chosen
//! by a small context of its own (see `choose`). Each set makes its own mixed prediction and learns from its own
//! error; the mixer's answer is their average.
//!
//! It works for any number of inputs: every weight starts at 1 / inputs, the plain average.
//!
//! The fourth set is chosen by what the match model says (how sure its learnt prediction is, in four classes,
//! the first when it has no match, and which bit it foretells), by the kind of each of the last two bytes (its
//! three high bits), and by the bits of this byte known so far. Where a sure match holds, the other models count
//! for little; where there is none, they are all there is.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;
use crate::models::{order2, order3, order4, order6, pair};

/// How many sets of weights are chosen at each bit, each by its own context.
const SETS: usize = 4;

/// How many contexts each chooser tells apart.
const SIZES: [usize; SETS] = [256 * 256, 256 * 256, 256 * 256, 512 * 256];

/// Where the match model's first prediction is among the inputs: the models come in the order of their names.
const REPEAT: usize = order2::OUTPUTS + order3::OUTPUTS + order4::OUTPUTS + order6::OUTPUTS + pair::OUTPUTS;

/// A weight of 1 is 2^16.
const ONE: i32 = 1 << 16;

/// The error (out of 2^16) times the input (steps of 1/256) is shifted down by this much before it moves a weight.
const RATE: u32 = 13;

/// The rate starts high and falls as a weight set learns: an update moves its weights by `FAST / (count + 1)`
/// times the base rate, the count being how many updates that set (that one context of its chooser) has had,
/// and never by less than `LEAST` times it, a least rate below the base one that keeps adapting. Both are in
/// sixteenths.
const FAST: i64 = 16 * 12;
const LEAST: i64 = 6;
/// The count stops here: past it the rate is the least one for good.
const CAP: u16 = 1023;

pub struct Mixer {
    inputs: usize,
    /// All the weights: for each set, for each of its contexts, one weight per input.
    weights: Vec<i32>,
    /// For each set, for each of its contexts, how many updates its weights have had.
    counts: Vec<u16>,
    /// Where each set's contexts start in `weights`.
    bases: [usize; SETS],
    /// The inputs of the bit being coded.
    seen: Vec<i32>,
    /// Where the weights chosen for this bit start, for each set, and what each set predicted (stretched).
    chosen: [usize; SETS],
    said: [i32; SETS],
}

impl Mixer {
    /// A mixer for `inputs` predictions at each bit.
    pub fn new(inputs: usize) -> Self {
        let mut bases = [0; SETS];
        let mut total = 0;
        let mut slots = 0;
        for set in 0..SETS {
            slots += SIZES[set];
            bases[set] = total;
            total += SIZES[set] * inputs;
        }
        Mixer {
            inputs,
            weights: vec![ONE / inputs.max(1) as i32; total],
            counts: vec![0; slots],
            bases,
            seen: vec![0; inputs],
            chosen: [0; SETS],
            said: [0; SETS],
        }
    }

    /// The context each set of weights is chosen by: one of the last three whole bytes, or the match model's
    /// word with the kinds of the last two, and the bits of the current byte known so far (which also give the
    /// bit's place).
    fn choose(cx: &Context, inputs: &[i32]) -> [usize; SETS] {
        let partial = cx.partial as usize;
        let byte = |back: u32| ((cx.last4 >> (8 * back)) & 0xFF) as usize;
        let said = if REPEAT < inputs.len() { inputs[REPEAT] } else { 0 };
        let sure = ((said.unsigned_abs() as usize + 511) >> 9).min(3) | if said > 0 { 4 } else { 0 };
        let kinds = (byte(1) >> 5) << 3 | (byte(0) >> 5);
        [(byte(2) << 8) | partial, (byte(0) << 8) | partial, (byte(1) << 8) | partial, (sure << 14) | (kinds << 8) | partial]
    }

    /// One stretched prediction from the models' stretched predictions.
    pub fn mix(&mut self, cx: &Context, inputs: &[i32]) -> i32 {
        let n = self.inputs;
        self.seen.copy_from_slice(&inputs[..n]);
        let contexts = Self::choose(cx, inputs);
        let mut sum = 0;
        for set in 0..SETS {
            let at = self.bases[set] + contexts[set] * n;
            self.chosen[set] = at;
            let dot = dot(&self.weights[at..at + n], &self.seen);
            let said = ((dot >> 16) as i32).clamp(-2047, 2047);
            self.said[set] = said;
            sum += said;
        }
        sum / SETS as i32
    }

    /// Told the bit that came, before the context moves on.
    pub fn update(&mut self, cx: &Context, bit: u32) {
        let n = self.inputs;
        for set in 0..SETS {
            let error = ((bit << 16) as i32) - cx.scale.squash_fine(self.said[set]) as i32;
            let at = self.chosen[set];
            let slot = if n == 0 { 0 } else { at / n };
            let count = self.counts[slot];
            if count < CAP {
                self.counts[slot] = count + 1;
            }
            let speed = (FAST / (count as i64 + 1)).max(LEAST);
            train(&mut self.weights[at..at + n], &self.seen, error as i64 * speed);
        }
    }
}

/// The weighted sum of the inputs.
#[inline(never)]
fn dot(weights: &[i32], inputs: &[i32]) -> i64 {
    let mut dot = 0i64;
    for (w, x) in weights.iter().zip(inputs.iter()) {
        dot += *w as i64 * *x as i64;
    }
    dot
}

/// Each weight moves by its input times `step` (the error times the speed).
#[inline(never)]
fn train(weights: &mut [i32], inputs: &[i32], step: i64) {
    for (w, x) in weights.iter_mut().zip(inputs.iter()) {
        *w += ((*x as i64 * step) >> (RATE + 4)) as i32;
    }
}
