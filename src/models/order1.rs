//! Order 1: the one byte before, and the bits of this byte known so far, choose one probability among 2^16, in a
//! direct table with no hash (256 contexts of 256 counts). Each is a count of 22 bits kept with a small counter of
//! the bits it has seen (10 bits, in one 32-bit word), as order 2 keeps its own: a new count moves fast, each bit
//! 1 / (seen + 1.5) of the way, until the counter reaches LIMIT and the rate stays there.
//!
//! A low order is cheap, and says something where the longer contexts have not been seen yet: early in the text,
//! and after a rare pair of bytes. Its counts fill fast and are kept long (a slow rate); where they have seen
//! much, the longer contexts are listened to more: the stretched prediction is scaled down, from 96/64 for a new
//! count to 64/64 at LIMIT.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 1;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 1000;

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
        Model { counts: vec![1 << 31; 1 << 16], rates, slot: 0 }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last one, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        self.slot = (((cx.last4 & 0xFF) as usize) << 8) | cx.partial as usize;
        let word = self.counts[self.slot];
        let seen = (word & SEEN_MASK) as i32;
        out[0] = cx.scale.stretch(word >> 20) * (96 - 32 * seen / LIMIT as i32) / 64;
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
