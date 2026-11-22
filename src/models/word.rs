//! The word model: the word being written so far (its hash, which the context keeps) and the bits of this byte
//! known so far choose one count. After "ki" it has seen what follows "ki" at the start of a word, whatever
//! came before the word; orders 2 and 3 see only the last letters, and not where the word began.
//!
//! The counts are order 2's kind: 22 bits and a counter of the bits seen, in one 32-bit word, a new count moving
//! fast and an old one slowly. The hash and the bits are spread over 2^BITS counts by a multiplication, as in
//! order 3, and two contexts that land on the same count share it.
//!
//! As in order 2, the counter says what the prediction is worth beside the others': the stretched prediction is
//! scaled from 96/64 for a new count down to 48/64 at LIMIT (measured on the corpus: the best of those tried).

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 1;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 60;

/// The low bits of a word hold the counter, the 22 above them the count.
const SEEN_BITS: u32 = 10;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

/// The table holds 2^BITS counts.
const BITS: u32 = 22;

/// An odd multiplier that sends each 32-bit context to its own 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

pub struct Model {
    counts: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { counts: vec![1 << 31; 1 << BITS], rates, slot: 0 }
    }

    /// The context has the word's hash: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = cx.word.wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        let word = self.counts[self.slot];
        let seen = (word & SEEN_MASK) as i32;
        out[0] = cx.scale.stretch(word >> 20) * (96 - 48 * seen / LIMIT as i32) / 64;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.counts[self.slot];
        let seen = word & SEEN_MASK;
        let p = (word >> SEEN_BITS) as i32;
        let target = (bit << 22) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.counts[self.slot] = ((moved as u32) << SEEN_BITS) | seen;
    }
}
