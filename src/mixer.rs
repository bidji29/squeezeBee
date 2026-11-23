//! The mixer: makes one prediction of all the models' predictions for the bit to come.
//!
//! Each model has a weight, and the mixed prediction is the weighted sum of the models' stretched predictions.
//! After each bit, every weight moves by the error of the mixed prediction times what its model said: a model
//! that pushed the right way counts for more next time, one that pushed the wrong way for less.
//!
//! What a model is worth depends on where the coding stands, so there are several sets of weights, each chosen
//! by a small context of its own (see `choose`). Each set makes its own mixed prediction and learns from its own
//! error. A small second mixer, with weights chosen by the bit's place, weighs the sets' predictions on the
//! stretched scale and learns from the error of its own answer, which is the mixer's.
//!
//! It works for any number of inputs: every weight starts at 1 / inputs, the plain average.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

/// How many sets of weights are chosen at each bit, each by its own context.
const SETS: usize = 5;

/// How many contexts each chooser tells apart.
const SIZES: [usize; SETS] = [256 * 256, 256 * 256, 256 * 256, 256, 8];

/// A weight of 1 is 2^16.
const ONE: i32 = 1 << 16;

/// The error (out of 2^16) times the input (steps of 1/256) is shifted down by this much before it moves a weight.
const RATE: u32 = 13;

/// The same for the second mixer's weights.
const RATE2: u32 = 17;

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
    /// The second mixer: for each bit's place, one weight per set; the place chosen for this bit, and its answer.
    final_weights: Vec<i32>,
    final_at: usize,
    final_said: i32,
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
            final_weights: vec![ONE / SETS as i32; 8 * SETS],
            final_at: 0,
            final_said: 0,
        }
    }

    /// The context each set of weights is chosen by: one of the last three whole bytes and the bits of the
    /// current byte known so far (which also give the bit's place); the last whole byte alone; the bit's place alone.
    fn choose(cx: &Context) -> [usize; SETS] {
        let partial = cx.partial as usize;
        let byte = |back: u32| ((cx.last4 >> (8 * back)) & 0xFF) as usize;
        [(byte(2) << 8) | partial, (byte(0) << 8) | partial, (byte(1) << 8) | partial, byte(0), cx.bit as usize]
    }

    /// One stretched prediction from the models' stretched predictions.
    pub fn mix(&mut self, cx: &Context, inputs: &[i32]) -> i32 {
        let n = self.inputs;
        self.seen.copy_from_slice(&inputs[..n]);
        let contexts = Self::choose(cx);
        for set in 0..SETS {
            let at = self.bases[set] + contexts[set] * n;
            self.chosen[set] = at;
            self.said[set] = dot(&self.weights[at..at + n], &self.seen);
        }
        let at = cx.bit as usize * SETS;
        self.final_at = at;
        self.final_said = dot(&self.final_weights[at..at + SETS], &self.said);
        self.final_said
    }

    /// Told the bit that came, before the context moves on.
    pub fn update(&mut self, cx: &Context, bit: u32) {
        let n = self.inputs;
        for set in 0..SETS {
            let error = ((bit << 16) as i32) - cx.scale.squash_fine(self.said[set]) as i32;
            let at = self.chosen[set];
            train(&mut self.weights[at..at + n], &self.seen, error, RATE);
        }
        let error = ((bit << 16) as i32) - cx.scale.squash_fine(self.final_said) as i32;
        let at = self.final_at;
        train(&mut self.final_weights[at..at + SETS], &self.said, error, RATE2);
    }
}

/// The weighed sum of the inputs, back on the stretched scale.
#[inline(never)]
fn dot(weights: &[i32], inputs: &[i32]) -> i32 {
    let mut sum = 0i64;
    for (w, x) in weights.iter().zip(inputs.iter()) {
        sum += *w as i64 * *x as i64;
    }
    ((sum >> 16) as i32).clamp(-2047, 2047)
}

/// Each weight moves by the error times its input.
#[inline(never)]
fn train(weights: &mut [i32], inputs: &[i32], error: i32, rate: u32) {
    for (w, x) in weights.iter_mut().zip(inputs.iter()) {
        *w += (*x * error) >> rate;
    }
}
