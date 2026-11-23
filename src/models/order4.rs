//! Order 4: the four bytes before, and the bits of this byte known so far, choose one slot.
//! Together they are 40 bits: a hash sends them to one of 2^BITS slots, and each slot keeps, beside its counts,
//! CHECK_BITS more bits of the hash. A context that finds another's check in its slot says nothing (0, no
//! opinion either way) and takes the slot over when the bit is known: a collision is not taken for evidence.
//!
//! A slot is one 64-bit word that holds two counts of 16 bits, which adapt at two speeds, and a counter:
//! - the check on top;
//! - a steady count, which always moves 1 / 2^STEADY of the way towards each bit it sees, new or old;
//! - a count that slows as evidence grows: as in order 2, its first bit takes it two thirds of the way and
//!   the next ones 1 / (seen + 1.5) of the way, until the counter reaches LIMIT;
//! - at the bottom, the counter of the bits seen.
//!
//! The model gives both counts as predictions, stretched, and the mixer weighs them.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The counter stops here: from then on the slowing count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u64 = 30;

/// The steady count moves 1 / 2^STEADY of the way.
const STEADY: u32 = 6;

/// The table holds 2^BITS slots.
const BITS: u32 = 24;

const CHECK_BITS: u32 = 8;
const SEEN_BITS: u32 = 8;
const SEEN_MASK: u64 = (1 << SEEN_BITS) - 1;
const STEADY_SHIFT: u32 = SEEN_BITS + 16;

/// A slot nobody has taken: a check of 0, both counts at one half, nothing seen.
const EMPTY: u64 = (1 << (SEEN_BITS + 15)) | (1 << (STEADY_SHIFT + 15));

pub struct Model {
    slots: Vec<u64>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    slot: usize,
    /// The check of the context being coded, already in its place at the top of a word.
    check: u64,
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT as u32 {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { slots: vec![EMPTY; 1 << BITS], rates, slot: 0, check: 0 }
    }

    /// The context has the last four bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let mut h = cx.last4.wrapping_mul(0x9E37_79B1).wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        self.slot = (h >> (32 - BITS)) as usize;
        h ^= h >> 13;
        self.check = ((h << (32 - CHECK_BITS)) as u64) << 32;
        let word = self.slots[self.slot];
        if (word ^ self.check) >> (64 - CHECK_BITS) != 0 {
            out[0] = 0;
            out[1] = 0;
            return;
        }
        out[0] = cx.scale.stretch(((word >> (SEEN_BITS + 4)) & 0xFFF) as u32);
        out[1] = cx.scale.stretch(((word >> (STEADY_SHIFT + 4)) & 0xFFF) as u32);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let mut word = self.slots[self.slot];
        if (word ^ self.check) >> (64 - CHECK_BITS) != 0 {
            word = EMPTY;
        }
        let seen = word & SEEN_MASK;
        let p = ((word >> SEEN_BITS) & 0xFFFF) as i32;
        let steady = ((word >> STEADY_SHIFT) & 0xFFFF) as i32;
        let target = (bit << 16) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let steady = steady + ((target - steady) >> STEADY);
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = self.check | ((steady as u64) << STEADY_SHIFT) | ((moved as u64) << SEEN_BITS) | seen;
    }
}
