//! Order 3: the three bytes before, and the bits of this byte known so far, choose one slot.
//! The four values together are 32 bits, too many for a table of their own: a multiplication spreads them
//! over 2^BITS slots, and two contexts that land on the same slot share it (nothing tells them apart).
//!
//! A slot is one 32-bit word that holds two counts, which adapt at two speeds, and a counter:
//! - on top, a fast count of 11 bits, which moves a quarter of the way towards each bit it sees: it follows
//!   what the context did lately;
//! - below it, a slow count of 15 bits, which moves fast while it is new and slower as evidence grows, as
//!   order 2's does: 1 / (seen + 1.5) of the way, until the counter reaches LIMIT and the rate stays there;
//! - at the bottom, the counter of the bits seen, 6 bits.
//!
//! The model gives both counts as predictions, stretched, and the mixer weighs them.
//!
//! Beside each slot, one byte sums up the bits that slot saw lately: how many zeros (0 to 7), how many ones
//! (0 to 7), and the last bit. When a bit comes, its own number grows by one, and the other number, if above
//! two, is cut to half and one: what the context did lately weighs more than what it did long ago. A table
//! shared by all slots, indexed by that byte and by the place of the bit in its byte, holds a probability that learns what came after each such
//! history: how far a context with that history is to be trusted. It is the third prediction.

use alloc::vec;
use alloc::vec::Vec;

use crate::context::Context;

pub const OUTPUTS: usize = 3;

/// The fast count moves 1 / 2^FAST of the way.
const FAST: u32 = 2;

/// The counter stops here: from then on the slow count moves 1 / (LIMIT + 1.5) of the way.
const LIMIT: u32 = 63;

const SEEN_BITS: u32 = 6;
const SEEN_MASK: u32 = (1 << SEEN_BITS) - 1;
const SLOW_BITS: u32 = 15;
const SLOW_MASK: u32 = (1 << SLOW_BITS) - 1;
const FAST_SHIFT: u32 = SEEN_BITS + SLOW_BITS;
const FAST_BITS: u32 = 32 - FAST_SHIFT;

/// The table holds 2^BITS slots.
const BITS: u32 = 22;

/// An odd multiplier near 2^32 divided by the golden ratio: it sends the 2^32 contexts each to its own
/// 32-bit value, whose high bits are the slot.
const SPREAD: u32 = 0x9E37_79B1;

/// A slot that has seen nothing: both counts at one half.
const EMPTY: u32 = (1 << 31) | (1 << (SEEN_BITS + SLOW_BITS - 1));

/// A history: the zeros above, in bits 4 to 6; the ones in bits 1 to 3; the last bit in bit 0.
const HISTORIES: usize = 128;

/// The shared table's rate stops at 1 / (TABLE_LIMIT + 1.5).
const TABLE_LIMIT: u32 = 255;

pub struct Model {
    slots: Vec<u32>,
    /// The history of each slot.
    histories: Vec<u8>,
    /// For each history and each bit it may meet, the history that follows.
    next: [[u8; 2]; HISTORIES],
    /// For each history, a probability of a one out of 65536, and how many bits it saw (up to TABLE_LIMIT).
    /// One row of HISTORIES for each bit of the byte: the same history means more or less at each bit.
    table: Vec<u32>,
    table_seen: Vec<u32>,
    /// 65536 / (seen + 1.5), for each value of the table's counter.
    table_rates: Vec<u32>,
    /// The slot's history, and where it stands in the table with the bit's place.
    state: u8,
    history: usize,
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
        let mut next = [[0u8; 2]; HISTORIES];
        let mut prior = [0u32; HISTORIES];
        for (state, follow) in next.iter_mut().enumerate() {
            let zeros = (state >> 4) as u32;
            let ones = ((state >> 1) & 7) as u32;
            for bit in 0..2u32 {
                let (mut own, mut other) = if bit == 1 { (ones, zeros) } else { (zeros, ones) };
                if own < 7 {
                    own += 1;
                }
                if other > 2 {
                    other = other / 2 + 1;
                }
                let (z, o) = if bit == 1 { (other, own) } else { (own, other) };
                follow[bit as usize] = ((z << 4) | (o << 1) | bit) as u8;
            }
            // What the history says before anything is learned: (ones + 1/2) / (all + 1).
            prior[state] = ((2 * ones + 1) * 65536) / (2 * (zeros + ones) + 2);
        }
        let mut table_rates = Vec::with_capacity(TABLE_LIMIT as usize + 1);
        for seen in 0..=TABLE_LIMIT {
            table_rates.push(131072 / (2 * seen + 3));
        }
        let mut table = Vec::with_capacity(8 * HISTORIES);
        for _ in 0..8 {
            table.extend_from_slice(&prior);
        }
        Model {
            slots: vec![EMPTY; 1 << BITS],
            histories: vec![0; 1 << BITS],
            next,
            table,
            table_seen: vec![0; 8 * HISTORIES],
            table_rates,
            state: 0,
            history: 0,
            rates,
            slot: 0,
        }
    }

    /// `history` holds the whole bytes coded so far (this model looks at the last three, which the context has).
    pub fn predict(&mut self, cx: &Context, _history: &[u8], out: &mut [i32]) {
        let context = (cx.last4 << 8) | cx.partial;
        self.slot = (context.wrapping_mul(SPREAD) >> (32 - BITS)) as usize;
        let word = self.slots[self.slot];
        // Both counts as probabilities out of 4096.
        out[0] = cx.scale.stretch((word >> FAST_SHIFT) << (12 - FAST_BITS));
        out[1] = cx.scale.stretch((word >> (SEEN_BITS + SLOW_BITS - 12)) & 0xFFF);
        self.state = self.histories[self.slot];
        self.history = self.state as usize | ((cx.bit as usize) << 7);
        out[2] = cx.scale.stretch((self.table[self.history] >> 4).clamp(1, 4095));
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        let word = self.slots[self.slot];
        let fast = word >> FAST_SHIFT;
        let slow = ((word >> SEEN_BITS) & SLOW_MASK) as i32;
        let seen = word & SEEN_MASK;
        let fast = if bit == 1 { fast + (((1 << FAST_BITS) - fast) >> FAST) } else { fast - (fast >> FAST) };
        // The slow count goes towards 0 or towards the top of its 15 bits.
        let target = (bit << SLOW_BITS) as i32 - bit as i32;
        let slow = slow + (((target - slow) * self.rates[seen as usize] as i32) >> 16);
        let seen = if seen < LIMIT { seen + 1 } else { seen };
        self.slots[self.slot] = (fast << FAST_SHIFT) | ((slow as u32) << SEEN_BITS) | seen;
        // The shared table learns what came after this history, and the slot's history takes the bit.
        let h = self.history;
        let p = self.table[h] as i32;
        let target = (bit << 16) as i32 - bit as i32;
        let n = self.table_seen[h];
        self.table[h] = (p + (((target - p) as i64 * self.table_rates[n as usize] as i64) >> 16) as i32) as u32;
        if n < TABLE_LIMIT {
            self.table_seen[h] = n + 1;
        }
        self.histories[self.slot] = self.next[self.state as usize][bit as usize];
    }
}
