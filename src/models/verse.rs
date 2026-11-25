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
//! The counts are order 2's kind: 22 bits and a counter of the bits seen, in one 32-bit word, a new count moving
//! fast and an old one slowly. Each context is spread over 2^BITS counts of its own by a multiplication, and
//! two contexts that land on the same count share it. The stretched prediction is scaled by the counter as
//! in the word model, from 96/64 for a new count down to 48/64 at LIMIT.

use alloc::vec;
use alloc::vec::Vec;

use super::order4::{moved, rates};
use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// The counter stops here: from then on a count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 60;

/// The low bits of a word hold the counter, the 22 above them the count.
const SEEN_BITS: u32 = 10;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;

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
    bases: [u32; OUTPUTS],
    slots: [usize; OUTPUTS],
}

impl Model {
    pub fn new() -> Self {
        Model { counts: vec![1 << 31; OUTPUTS << BITS], rates: rates(LIMIT), bases: [0; OUTPUTS], slots: [0; OUTPUTS] }
    }

    /// The context has the column and the widths: `history` is not read.
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            let last = cx.last4 & 0xFF;
            let shown = cx.shown.min(WIDEST);
            let before = cx.above_shown.min(WIDEST);
            self.bases = [(shown << 8) | last, (1 << 24) | (before << 16) | (shown << 8) | (last >> 5)];
        }
        for which in 0..OUTPUTS {
            let context = self.bases[which].wrapping_add(cx.partial.wrapping_mul(0x632B_E5AB));
            let slot = (which << BITS) + (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
            self.slots[which] = slot;
            let word = self.counts[slot];
            let seen = (word & SEEN_MASK) as i32;
            out[which] = cx.scale.stretch(word >> 20) * (96 - 48 * seen / LIMIT as i32) / 64;
        }
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        for &slot in self.slots.iter() {
            self.counts[slot] = moved(self.counts[slot], bit, &self.rates, 22, SEEN_BITS, LIMIT);
        }
    }
}
