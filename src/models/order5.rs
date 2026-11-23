//! Order 5: the five bytes before, and the bits of this byte known so far, choose one count. It is order 4's
//! model with one byte more of context, as order 4 is order 3's: the five bytes are hashed to 32 bits, with
//! the bits of this byte, and a hash sends them to one of 2^BITS slots, and each slot keeps, beside its count,
//! CHECK_BITS more bits of the hash. A context that finds another's check in its slot says nothing (0, no
//! opinion either way) and takes the slot over when the bit is known: a collision is not taken for evidence.
//!
//! A slot is one 32-bit word: the check on top, a count of 16 bits, and a counter of the bits seen at the
//! bottom. As in order 2, a new count moves fast (its first bit takes it two thirds of the way) and an old one
//! slowly, 1 / (seen + 1.5) of the way until the counter reaches LIMIT.
//!
//! The model gives two predictions: the count's, stretched, and the same scaled by how much the count has
//! seen, so that the mixer can weigh a settled context and a new one apart.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 30;

/// The table holds 2^BITS slots.
const BITS: u32 = 24;

const CHECK_BITS: u32 = 8;
const SEEN_BITS: u32 = 8;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

/// A slot nobody has taken: a check of 0, a count at one half, nothing seen.
const EMPTY: u32 = 1 << (SEEN_BITS + 15);

pub struct Model {
    slots: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
    /// The check of the context being coded, already in its place at the top of a word.
    check: u32,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { slots: vec![EMPTY; 1 << BITS], rates, slot: 0, check: 0 }
    }

    /// The context has the last eight bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let five = cx.last8 & 0xFF_FFFF_FFFF;
        let mut h = ((five.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 32) as u32).wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        self.slot = (h >> (32 - BITS)) as usize;
        h ^= h >> 13;
        self.check = h << (32 - CHECK_BITS);
        let word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            out[0] = 0;
            out[1] = 0;
            return;
        }
        let seen = (word & SEEN_MASK) as i32;
        let said = cx.scale.stretch((word >> (SEEN_BITS + 4)) & 0xFFF);
        out[0] = said;
        out[1] = said * seen / LIMIT as i32;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let mut word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            word = EMPTY;
        }
        let seen = word & SEEN_MASK;
        let p = ((word >> SEEN_BITS) & 0xFFFF) as i32;
        let target = (bit << 16) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = self.check | ((moved as u32) << SEEN_BITS) | seen;
    }
}
