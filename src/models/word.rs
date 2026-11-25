//! The word model: the word being written so far (its hash, which the context keeps) and the bits of this byte
//! known so far choose one slot. After "ki" it has seen what follows "ki" at the start of a word, whatever
//! came before the word; orders 2 and 3 see only the last letters, and not where the word began.
//!
//! A slot is one 32-bit word that holds two counts, which adapt at two speeds, and a counter, as order 3's does:
//! - on top, a fast count of FAST_BITS bits, which moves 1 / 2^FAST of the way towards each bit it sees: it
//!   follows what the context did lately;
//! - below it, a slow count of SLOW_BITS bits, which moves fast while it is new and slower as evidence grows:
//!   1 / (seen + 1.5) of the way, until the counter reaches LIMIT and the rate stays there;
//! - at the bottom, the counter of the bits seen, 6 bits.
//! The hash and the bits are spread over 2^BITS slots by a multiplication, as in order 3, and two contexts that
//! land on the same slot share it.
//!
//! The model gives both counts as predictions, stretched, and the mixer weighs them. As in order 2, the counter
//! says what the predictions are worth beside the others': both are scaled from 96/64 for a new slot down to
//! 48/64 at LIMIT (measured on the corpus: the best of those tried, as is FAST).

use alloc::vec;
use alloc::vec::Vec;

use super::order4::{moved, rates};
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The fast count moves 1 / 2^FAST of the way.
const FAST: u32 = 3;

/// The counter stops here: from then on the slow count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 60;

const SEEN_BITS: u32 = 6;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;
const SLOW_BITS: u32 = 16;
const FAST_SHIFT: u32 = SEEN_BITS + SLOW_BITS;
const FAST_BITS: u32 = 32 - FAST_SHIFT;

/// The table holds 2^BITS slots.
const BITS: u32 = 22;

/// An odd multiplier that sends each 32-bit context to its own 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

/// A slot that has seen nothing: both counts at one half.
const EMPTY: u32 = (1 << 31) | (1 << (FAST_SHIFT - 1));

pub struct Model {
    slots: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
}

impl Model {
    pub fn new() -> Self {
        Model { slots: vec![EMPTY; 1 << BITS], rates: rates(LIMIT), slot: 0 }
    }

    /// The context has the word's hash: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = cx.word.wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        let word = self.slots[self.slot];
        let seen = (word & SEEN_MASK) as i32;
        let worth = 96 - 48 * seen / LIMIT as i32;
        // Both counts as probabilities out of 4096.
        out[0] = cx.scale.stretch((word >> (FAST_SHIFT - 12)) & 0xFFF) * worth / 64;
        out[1] = cx.scale.stretch(word >> (32 - 12)) * worth / 64;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.slots[self.slot];
        let fast = word >> FAST_SHIFT;
        let fast = if bit == 1 { fast + (((1 << FAST_BITS) - fast) >> FAST) } else { fast - (fast >> FAST) };
        // The slow count and the counter move as every model's do.
        self.slots[self.slot] = (fast << FAST_SHIFT) | moved(word, bit, &self.rates, SLOW_BITS, SEEN_BITS, LIMIT);
    }
}
