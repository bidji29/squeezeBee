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
//! The first set is chosen by the bits of the byte known so far (so by the bit's place) and by how much the models
//! agree: the spread of their stretched predictions, largest minus smallest, in a few buckets.
//!
//! It works for any number of inputs: every weight starts at 1 / inputs, the plain average.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

/// How many sets of weights are chosen at each bit, each by its own context.
const SETS: usize = 3;

/// How many contexts each chooser tells apart.
const SIZES: [usize; SETS] = [256 * BUCKETS, 256 * 8, 256 * 8];

/// How many buckets the spread of the inputs falls into.
const BUCKETS: usize = 4;

/// The spread (largest stretched input minus smallest) is shifted down by this much to give its bucket.
const SPREAD_SHIFT: u32 = 10;

/// A weight of 1 is 2^16.
const ONE: i32 = 1 << 16;

/// The error (out of 2^16) times the input (steps of 1/256) is shifted down by this much before it moves a weight.
const RATE: u32 = 13;

pub struct Mixer {
    inputs: usize,
    /// All the weights: for each set, for each of its contexts, one weight per input.
    weights: Vec<i32>,
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
        for set in 0..SETS {
            bases[set] = total;
            total += SIZES[set] * inputs;
        }
        Mixer {
            inputs,
            weights: vec![ONE / inputs.max(1) as i32; total],
            bases,
            seen: vec![0; inputs],
            chosen: [0; SETS],
            said: [0; SETS],
        }
    }

    /// The context each set of weights is chosen by.
    fn choose(cx: &Context, inputs: &[i32]) -> [usize; SETS] {
        let mut low = i32::MAX;
        let mut high = i32::MIN;
        for &x in inputs {
            low = low.min(x);
            high = high.max(x);
        }
        let spread = if high > low { (high - low) as usize } else { 0 };
        let bucket = (spread >> SPREAD_SHIFT).min(BUCKETS - 1);
        let last = (cx.last4 & 0xFF) as usize;
        let before = ((cx.last4 >> 8) & 0xFF) as usize;
        [cx.partial as usize * BUCKETS + bucket, (last << 3) | cx.bit as usize, (before << 3) | cx.bit as usize]
    }

    /// One stretched prediction from the models' stretched predictions.
    pub fn mix(&mut self, cx: &Context, inputs: &[i32]) -> i32 {
        let n = self.inputs;
        self.seen.copy_from_slice(&inputs[..n]);
        let contexts = Self::choose(cx, &self.seen);
        let mut sum = 0;
        for set in 0..SETS {
            let at = self.bases[set] + contexts[set] * n;
            self.chosen[set] = at;
            let mut dot = 0i64;
            for (w, x) in self.weights[at..at + n].iter().zip(self.seen.iter()) {
                dot += *w as i64 * *x as i64;
            }
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
            for (w, x) in self.weights[at..at + n].iter_mut().zip(self.seen.iter()) {
                *w += (*x * error) >> RATE;
            }
        }
    }
}
