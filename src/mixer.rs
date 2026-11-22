//! The mixer: makes one prediction of all the models' predictions for the bit to come.
//!
//! Each model has a weight, and the mixed prediction is the weighted sum of the models' stretched predictions.
//! After each bit, every weight moves by the error of the mixed prediction times what its model said: a model
//! that pushed the right way counts for more next time, one that pushed the wrong way for less.
//!
//! What a model is worth depends on where the coding stands, so there are many sets of weights, and the one used
//! for a bit is chosen by the bit's place in its byte and by the previous whole byte together (see `choose`).
//! The mixer sees every bit in `update`, so it keeps the previous byte itself.
//!
//! It works for any number of inputs: every weight starts at 1 / inputs, the plain average.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

/// How many sets of weights are chosen at each bit, each by its own context.
const SETS: usize = 1;

/// How many contexts each chooser tells apart: the bit's place (8) times the previous byte (256).
const SIZES: [usize; SETS] = [8 * 256];

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
    /// The previous whole byte, the bits of the current byte seen so far, and how many they are (0 to 7).
    previous: usize,
    current: usize,
    place: usize,
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
            previous: 0,
            current: 0,
            place: 0,
        }
    }

    /// The context each set of weights is chosen by.
    fn choose(&self) -> [usize; SETS] {
        [(self.previous << 3) | self.place]
    }

    /// One stretched prediction from the models' stretched predictions.
    pub fn mix(&mut self, _cx: &Context, inputs: &[i32]) -> i32 {
        let n = self.inputs;
        self.seen.copy_from_slice(&inputs[..n]);
        let contexts = self.choose();
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
        self.current = (self.current << 1) | bit as usize;
        self.place += 1;
        if self.place == 8 {
            self.previous = self.current & 0xFF;
            self.current = 0;
            self.place = 0;
        }
    }
}
