//! Order 6: the six bytes before, and the bits of this byte known so far, choose one count.
//! Together they are 56 bits: a hash sends them to one of 2^BITS slots, and each slot keeps, beside its count,
//! CHECK_BITS more bits of the hash. A context that finds another's check in its slot says nothing (0, no
//! opinion either way) and takes the slot over when the bit is known: a collision is not taken for evidence.
//!
//! The slot is a word as in order 4: the check on top, a count of 16 bits, and a counter of the bits seen at
//! the bottom; the count moves 1 / (seen + 1.5) of the way until the counter reaches LIMIT.
//!
//! The six bytes are hashed once a byte, when the byte starts; each bit then only mixes in the bits known so
//! far, so the work per bit is one multiplication and one look in the table.
//!
//! Each slot keeps a second count beside it, which moves fast: 1 / (seen + 1.5) of the way while the counter is
//! below FAST_LIMIT, then 1 / (FAST_LIMIT + 1.5) for good. The slow count says what the context does on the whole,
//! the fast one what it did lately; the mixer weighs the two.
//!
//! Three predictions: the slow count's, stretched, the same scaled by how much the count has seen, and the fast
//! count's, stretched.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 3;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 30;

/// The fast count's rate stops falling here: from then on it moves 1 / (FAST_LIMIT + 1.5) of the way.
const FAST_LIMIT: u32 = 6;

/// The table holds 2^BITS slots: few, so that a context comes back to its slot only while it is recent (measured:
/// smaller tables did better here, down to this size).
const BITS: u32 = 15;

const CHECK_BITS: u32 = 8;
const SEEN_BITS: u32 = 8;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

/// A slot nobody has taken: a check of 0, a count at one half, nothing seen.
const EMPTY: u32 = 1 << (SEEN_BITS + 15);

pub struct Model {
    slots: Vec<u32>,
    /// The fast count of each slot, 16 bits, kept with the slot's check (taken over with it).
    fast: Vec<u16>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    /// The hash of the six bytes before the byte being coded.
    base: u32,
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
        Model { slots: vec![EMPTY; 1 << BITS], fast: vec![1 << 15; 1 << BITS], rates, base: 0, slot: 0, check: 0 }
    }

    /// The context has the last eight bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let six = cx.last8 & 0xFFFF_FFFF_FFFF;
            let h = six.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            self.base = (h >> 32) as u32;
        }
        let mut h = self.base.wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        self.slot = (h >> (32 - BITS)) as usize;
        h ^= h >> 13;
        self.check = h << (32 - CHECK_BITS);
        let word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            out[0] = 0;
            out[1] = 0;
            out[2] = 0;
            return;
        }
        let seen = (word & SEEN_MASK) as i32;
        let said = cx.scale.stretch((word >> (SEEN_BITS + 4)) & 0xFFF);
        out[0] = said;
        out[1] = said * seen / LIMIT as i32;
        out[2] = cx.scale.stretch((self.fast[self.slot] >> 4) as u32);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let mut word = self.slots[self.slot];
        let mut fast = self.fast[self.slot] as i32;
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            word = EMPTY;
            fast = 1 << 15;
        }
        let seen = word & SEEN_MASK;
        let p = ((word >> SEEN_BITS) & 0xFFFF) as i32;
        let target = (bit << 16) as i32 - bit as i32;
        let moved = p + (((target - p) as i64 * self.rates[seen as usize] as i64) >> 16) as i32;
        let quick = self.rates[seen.min(FAST_LIMIT) as usize] as i64;
        self.fast[self.slot] = (fast + (((target - fast) as i64 * quick) >> 16) as i32) as u16;
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = self.check | ((moved as u32) << SEEN_BITS) | seen;
    }
}
