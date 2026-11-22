//! Order 2: the two bytes before, and the bits of this byte known so far, choose one probability among 2^24.
//! Each is a count of 22 bits kept with a small counter of the bits it has seen (10 bits, in one 32-bit word).
//! A new count moves fast: its first bit takes it two thirds of the way, the next ones less and less, each
//! 1 / (seen + 1.5) of the way, until the counter reaches LIMIT and the rate stays there.
//!
//! The counter also says how much this model's word is worth beside the longer contexts': where a count has
//! seen little, the longer contexts have seen less and this one is listened to; where it has seen much, they
//! know better. So the stretched prediction is scaled down, from 56/64 for a new count to 24/64 at LIMIT.
//!
//! A model is a file of this folder. The build finds it by its name, with no list to add it to. It gives:
//! `OUTPUTS`, how many predictions it makes at each bit; `Model::new()`; `predict`, which writes its predictions,
//! stretched, for the bit to come; `update`, which is told that bit before the context moves on.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 1;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 60;

/// The low bits of a word hold the counter, the 22 above them the count.
const SEEN_BITS: u32 = 10;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

pub struct Model {
    counts: Vec<u32>,
    /// 65536 / (seen + 1.5), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { counts: vec![1 << 31; 1 << 24], rates, slot: 0 }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last two, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        self.slot = (((cx.last4 & 0xFFFF) as usize) << 8) | cx.partial as usize;
        let word = self.counts[self.slot];
        let seen = (word & SEEN_MASK) as i32;
        out[0] = cx.scale.stretch(word >> 20) * (56 - 32 * seen / LIMIT as i32) / 64;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.counts[self.slot];
        let seen = word & SEEN_MASK;
        let p = (word >> SEEN_BITS) as i32;
        // The count goes towards 0 or towards the top of its 22 bits.
        let target = (bit << 22) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.counts[self.slot] = ((moved as u32) << SEEN_BITS) | seen;
    }
}
