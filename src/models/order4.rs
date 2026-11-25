//! Order 4: the four bytes before, and the bits of this byte known so far, choose one count.
//! Together they are 40 bits: a hash sends them to one of 2^BITS slots, and each slot keeps, beside its count,
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

/// 131072 / (2 seen + 3), for each value of a counter that stops at `limit`: every model's counts move
/// 1 / (seen + 1.5) of the way, and each takes its table of rates from here.
#[inline(never)]
pub fn rates(limit: u32) -> Vec<u32> {
    (0..=limit).map(|seen| 131072 / (2 * seen + 3)).collect()
}

/// A count of `top` bits above a counter of `seen_bits` bits, in one word, moved towards `bit` by the rate its
/// counter gives, the counter then raised until it reaches `limit`. Every model's slow counts move this way.
#[inline(never)]
pub fn moved(word: u32, bit: u32, rates: &[u32], top: u32, seen_bits: u32, limit: u32) -> u32 {
    let seen = word & ((1 << seen_bits) - 1);
    let p = ((word >> seen_bits) & ((1 << top) - 1)) as i32;
    // The count goes towards 0 or towards the top of its bits.
    let target = (bit << top) as i32 - bit as i32;
    let moved = p + (((target - p) as i64 * rates[seen as usize] as i64) >> 16) as i32;
    let seen = if seen < limit { seen + 1 } else { seen };
    ((moved as u32) << seen_bits) | seen
}

/// The table of slots with a check, which order 4, order 6, the pair and the triple models share: each gives the
/// hash of its whole bytes, `base`, and the number of its slots, and the rest of the work is this one.
pub struct Checked {
    slots: Vec<u32>,
    rates: Vec<u32>,
    /// 32 - log2 of the number of slots.
    shift: u32,
    slot: usize,
    /// The check of the context being coded, already in its place at the top of a word.
    check: u32,
}

impl Checked {
    pub fn new(bits: u32) -> Self {
        Checked { slots: vec![EMPTY; 1 << bits], rates: rates(LIMIT), shift: 32 - bits, slot: 0, check: 0 }
    }

    /// The bits of this byte known so far are mixed into `base`, which chooses the slot and the check.
    #[inline(never)]
    pub fn predict(&mut self, base: u32, cx: &Context, out: &mut [i32]) {
        let mut h = base.wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
        h ^= h >> 15;
        h = h.wrapping_mul(0x2F0B_4A8D);
        self.slot = (h >> self.shift) as usize;
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

    #[inline(never)]
    pub fn update(&mut self, bit: u32) {
        let mut word = self.slots[self.slot];
        if (word ^ self.check) >> (32 - CHECK_BITS) != 0 {
            word = EMPTY;
        }
        self.slots[self.slot] = self.check | moved(word, bit, &self.rates, 16, SEEN_BITS, LIMIT);
    }
}

pub struct Model {
    table: Checked,
}

impl Model {
    pub fn new() -> Self {
        Model { table: Checked::new(BITS) }
    }

    /// The context has the last four bytes: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        self.table.predict(cx.last4.wrapping_mul(0x9E37_79B1), cx, out);
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        self.table.update(bit);
    }
}
