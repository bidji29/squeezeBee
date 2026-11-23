//! The verse model: where the line stands. The corpus is set in lines, verse and prose cut at a width, and
//! the bytes just before do not say how far along its line a byte is: the context keeps that (the column as
//! the data shows it, and the width of the whole line before), and this model reads it.
//!
//! Two counts are chosen at each bit, both also by the bits of this byte known so far:
//! - by the column and the last byte. After a word, near the width where the lines of this text are cut, a
//!   line feed comes where a space would have; at the start of a line, a margin or a capital;
//! - by the width of the line before, the column, and the kind of the last byte (its three high bits: a
//!   letter, a mark or a space, a word of the transform). Lines of verse are about as long as the one before,
//!   and a line of prose that was cut short is followed by a word too long to have fitted there.
//!
//! The byte at this column of the line before (the context says where it is in the history) was tried as a
//! third count, and in place of the second: it gave a twentieth of what the column gives, so it is not
//! read here. What the context keeps for it is left to a model that finds a better use for it.
//!
//! Each slot is one 32-bit word that holds two counts and a counter of the bits seen: a slow count of 16 bits,
//! of order 2's kind (a new count moving fast and an old one slowly), and a fast count of 10 bits that moves
//! 1 / 2^FAST of the way at each bit; both are given as predictions, and the mixer weighs them. Each context is spread over 2^BITS counts of its own by a multiplication, and
//! two contexts that land on the same count share it. The slow count's stretched prediction is scaled by the counter as
//! in the word model, from 96/64 for a new count down to 48/64 at LIMIT.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 4;

/// How many contexts the model reads: each gives two predictions, its slow count and its fast one.
const CONTEXTS: usize = 2;

/// The fast count moves 1 / 2^FAST of the way.
const FAST: u32 = 5;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 60;

/// The low bits of a word hold the counter, the 16 above them the slow count, the 10 on top the fast count.
const SEEN_BITS: u32 = 6;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;
const SLOW_BITS: u32 = 16;
const SLOW_MASK: u32 = (1 << SLOW_BITS) - 1;
const FAST_SHIFT: u32 = SEEN_BITS + SLOW_BITS;
const FAST_BITS: u32 = 32 - FAST_SHIFT;

/// A word that has seen nothing: both counts at one half.
const EMPTY: u32 = (1 << 31) | (1 << (FAST_SHIFT - 1));

/// Each of the two tables holds 2^BITS counts.
const BITS: u32 = 22;

/// A column, or a width, beyond this one is taken for this one.
const WIDEST: u32 = 127;

/// An odd multiplier that sends each 32-bit context to its own 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

pub struct Model {
    counts: Vec<u32>,
    /// 131072 / (2 seen + 3), for each value of the counter.
    rates: Vec<u32>,
    /// What the whole bytes say of each context, worked out at a byte's start.
    bases: [u32; CONTEXTS],
    slots: [usize; CONTEXTS],
}

impl Model {
    pub fn new() -> Self {
        let mut rates = Vec::with_capacity(LIMIT as usize + 1);
        for seen in 0..=LIMIT {
            rates.push(131072 / (2 * seen + 3));
        }
        Model { counts: vec![EMPTY; CONTEXTS << BITS], rates, bases: [0; CONTEXTS], slots: [0; CONTEXTS] }
    }

    /// The context has the column and the widths: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let last = cx.last4 & 0xFF;
            let shown = cx.shown.min(WIDEST);
            let before = cx.above_shown.min(WIDEST);
            self.bases = [(shown << 8) | last, (1 << 24) | (before << 16) | (shown << 8) | (last >> 5)];
        }
        for which in 0..CONTEXTS {
            let context = self.bases[which].wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
            let slot = (which << BITS) + (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
            self.slots[which] = slot;
            let word = self.counts[slot];
            let seen = (word & SEEN_MASK) as i32;
            let slow = (word >> (FAST_SHIFT - 12)) & 0xFFF;
            out[2 * which] = cx.scale.stretch(slow) * (96 - 48 * seen / LIMIT as i32) / 64;
            out[2 * which + 1] = cx.scale.stretch((word >> FAST_SHIFT) << (12 - FAST_BITS));
        }
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        for &slot in self.slots.iter() {
            let word = self.counts[slot];
            let seen = word & SEEN_MASK;
            let fast = word >> FAST_SHIFT;
            let slow = ((word >> SEEN_BITS) & SLOW_MASK) as i32;
            let fast = if bit == 1 { fast + (((1 << FAST_BITS) - fast) >> FAST) } else { fast - (fast >> FAST) };
            // The slow count goes towards 0 or towards the top of its 16 bits.
            let target = (bit << SLOW_BITS) as i32 - bit as i32;
            let slow = slow + (((target - slow) * self.rates[seen as usize] as i32) >> 16);
            let seen = if seen < LIMIT { seen + 1 } else { seen };
            self.counts[slot] = (fast << FAST_SHIFT) | ((slow as u32) << SEEN_BITS) | seen;
        }
    }
}
