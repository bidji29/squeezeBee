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
        self.seen.copy_from_slice(&inputs[..n]);
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
