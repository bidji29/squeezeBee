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
//! Each set also has one more input, a constant (`BIAS`), with its own weight learnt by the same rule as the
//! others: it lets a set lean towards 0 or 1 by itself where its context calls for it. Its weight starts at 0.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

/// How many sets of weights are chosen at each bit, each by its own context.
const SETS: usize = 3;

/// How many contexts each chooser tells apart.
const SIZES: [usize; SETS] = [256 * 256; SETS];

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

/// The constant input each set has besides the models' (4, in the inputs' steps of 1/256).
const BIAS: i32 = 1024;

pub struct Mixer {
    inputs: usize,
    /// All the weights: for each set, for each of its contexts, one weight per input, then the bias's.
    weights: Vec<i32>,
    /// For each set, for each of its contexts, how many updates its weights have had.
    counts: Vec<u16>,
    /// Where each set's contexts start in `weights`.
    bases: [usize; SETS],
    /// The inputs of the bit being coded, then the bias.
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
            total += SIZES[set] * (inputs + 1);
        }
        let mut weights = vec![ONE / inputs.max(1) as i32; total];
        for bias in weights.iter_mut().skip(inputs).step_by(inputs + 1) {
            *bias = 0;
        }
        let mut seen = vec![0; inputs + 1];
        seen[inputs] = BIAS;
        Mixer {
            inputs,
            weights,
            counts: vec![0; slots],
            bases,
            seen,
            chosen: [0; SETS],
            said: [0; SETS],
        }
    }

    /// The context each set of weights is chosen by: one of the last three whole bytes, and the bits of the
    /// current byte known so far (which also give the bit's place).
    fn choose(cx: &Context) -> [usize; SETS] {
        let partial = cx.partial as usize;
        let byte = |back: u32| ((cx.last4 >> (8 * back)) & 0xFF) as usize;
        [(byte(2) << 8) | partial, (byte(0) << 8) | partial, (byte(1) << 8) | partial]
    }

    /// One stretched prediction from the models' stretched predictions.
    pub fn mix(&mut self, cx: &Context, inputs: &[i32]) -> i32 {
        let n = self.inputs;
        self.seen[..n].copy_from_slice(&inputs[..n]);
        let n = n + 1;
        let contexts = Self::choose(cx);
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
        let n = self.inputs + 1;
        for set in 0..SETS {
            let error = ((bit << 16) as i32) - cx.scale.squash_fine(self.said[set]) as i32;
            let at = self.chosen[set];
            let slot = at / n;
            let count = self.counts[slot];
            if count < CAP {
                self.counts[slot] = count + 1;
            }
            let speed = (FAST / (count as i64 + 1)).max(LEAST);
            for (w, x) in self.weights[at..at + n].iter_mut().zip(self.seen.iter()) {
                *w += ((*x as i64 * error as i64 * speed) >> (RATE + 4)) as i32;
            }
        }
    }
}
