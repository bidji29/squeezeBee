//! Order 8: the eight bytes before, and the bits of this byte known so far, choose one count.
//! Together they are 72 bits: a hash sends them to a pair of slots side by side, out of 2^BITS, and each slot
//! keeps, beside its count, CHECK_BITS more bits of the hash. The context's count is the slot of the two whose
//! check is its own. A context that finds neither says nothing (0, no opinion either way) and, when the bit is
//! known, takes over the one of the two whose counter has seen less: a collision is not taken for evidence, and
//! a settled count is not lost to a context met once.
//!
//! The slot is a word as in order 4: the check on top, a count of 16 bits, and a counter of the bits seen at
//! the bottom; the count moves 1 / (seen + 1.5) of the way until the counter reaches LIMIT.
//!
//! The eight bytes are hashed once a byte, when the byte starts; each bit then only mixes in the bits known so
//! far, so the work per bit is one multiplication and one look at two neighbouring words.
//!
//! Two predictions, as order 4's, in the other order: the count scaled by how much it has seen first, then the
//! count's own, stretched (measured: a little better first, where the mixer's fourth set reads one input).

use alloc::vec;
use alloc::vec::Vec;

use super::order4::{moved, rates};
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The table holds 2^BITS slots, 256 MB: many, since an order-8 context is rare (measured: 2^22, 2^24 and 2^25
/// did less well, and 2^16 or 2^18 worse than no model at all).
const BITS: u32 = 26;

/// The counter stops here, as in order 4.
const LIMIT: u32 = 30;
const CHECK_BITS: u32 = 8;
const SEEN_BITS: u32 = 8;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

/// A slot nobody has taken: a check of 0, a count at one half, nothing seen.
const EMPTY: u32 = 1 << (SEEN_BITS + 15);

pub struct Model {
    slots: Vec<u32>,
    rates: Vec<u32>,
    /// What the whole bytes say of the context, worked out at a byte's start.
    base: u32,
    /// The slot the context holds, or the one it takes over when the bit is known.
    slot: usize,
    /// Whether the context found its check in `slot`.
    found: bool,
    /// The check of the context being coded, already in its place at the top of a word.
    check: u32,
}

impl Model {
    pub fn new() -> Self {
        Model { slots: vec![EMPTY; 1 << BITS], rates: rates(LIMIT), base: 0, slot: 0, found: false, check: 0 }
    }

    /// The context has the last eight bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let h = cx.last8.wrapping_mul(0xA24B_AED4_963E_E407);
            self.base = (h >> 32) as u32;
        }
        let mut h = self.base.wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        let first = (h >> (32 - BITS)) as usize;
        h ^= h >> 13;
        self.check = h << (32 - CHECK_BITS);
        // The pair: this slot and its neighbour, in the same cache line. Neither the context's: the one that
        // has seen less is given up.
        let second = first ^ 1;
        let (a, b) = (self.slots[first], self.slots[second]);
        self.found = true;
        self.slot = if (a ^ self.check) >> (32 - CHECK_BITS) == 0 {
            first
        } else if (b ^ self.check) >> (32 - CHECK_BITS) == 0 {
            second
        } else {
            self.found = false;
            if (b & SEEN_MASK) < (a & SEEN_MASK) { second } else { first }
        };
        let word = self.slots[self.slot];
        let seen = (word & SEEN_MASK) as i32;
        // A context that found neither says nothing.
        let said = cx.scale.stretch((word >> (SEEN_BITS + 4)) & 0xFFF) * self.found as i32;
        out[0] = said * seen / LIMIT as i32;
        out[1] = said;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = if self.found { self.slots[self.slot] } else { EMPTY };
        self.slots[self.slot] = self.check | moved(word, bit, &self.rates, 16, SEEN_BITS, LIMIT);
    }
}
