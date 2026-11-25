//! The match model: where the last bytes were last seen, and the byte that followed them there.
//!
//! An index sends a hash of the last SEED bytes to the places just after their last two occurrences. Two
//! matches are held. While one of them is missing, each new byte looks its places up; a place found, that the
//! other match is not already at, is checked by walking back from it, which gives the true length of the match
//! (up to REACH). While a match is held, each new byte either agrees with the byte the match foretold, and the
//! match grows by one, or does not, and the match is dropped: the other one, if it foretold that byte, goes on.
//!
//! The byte foretold is the longer match's. During a byte, as long as the bits coded agree with it, the model
//! says the bit that byte has next. How sure it is, is learnt: a count of how often the foretold bit came true,
//! chosen by the length of the match (in a few classes), by what the other match says (there is none, it
//! foretells the same byte, or another, and then whether it is a short one) and by how many bits of this byte
//! have already agreed. When there is no match, or the byte has already parted from the foretold one, it says
//! nothing (0).
//!
//! It gives two predictions: the learnt one, and one from a count chosen by the foretold byte itself and the
//! byte before, so that the mixer can tell a repeat that usually holds here from one that usually breaks.
//!
//! The history is the one the frame gives to `predict` (the whole bytes coded so far): the model keeps the
//! index and no second copy of the bytes.

use alloc::boxed::Box;
use alloc::vec;

use crate::context::Context;

pub const OUTPUTS: usize = 2;

/// How many of the last bytes are hashed to find an earlier place.
const SEED: usize = 7;

/// The index holds 2^BITS pairs of places.
const BITS: u32 = 22;

/// A match is not measured beyond this length.
const REACH: usize = 400;

/// The classes of length.
const CLASSES: usize = 32;

/// Each bit moves a count 1 / 2^RATE of the way.
const RATE: u32 = 6;

/// The second count's table: 2^SIDE_BITS counts.
const SIDE_BITS: u32 = 16;

/// What the other match says of the byte foretold: there is none, it foretells the same byte, or another one
/// (a match shorter than WEIGHTY, or not).
const VIEWS: usize = 4;
const WEIGHTY: usize = 12;

/// Where a count starts, out of 65536. A table holds each count XOR START, so that it starts at 0.
const START: u16 = 49152;

pub struct Model {
    /// For each hash, the last two places: the most recent first.
    index: Box<[u32; 2 << BITS]>,
    /// How often the foretold bit came true, out of 65536: for each view of the other match, for each class of
    /// length, for each bit of a byte.
    sure: Box<[u16; VIEWS * CLASSES * 8]>,
    /// The same, chosen by the foretold byte, the byte before and the bit.
    side: Box<[u16; 1 << SIDE_BITS]>,
    /// The two matches held: the place in the history of the byte each foretells, and its length (0: none).
    place: [usize; 2],
    length: [usize; 2],
    /// The length of the longer one, and what the other one says of its byte (a view).
    longer: usize,
    view: usize,
    /// The byte the longer one foretells, with a leading 1 (0 when there is no match).
    foretold: u32,
    /// For the bit being coded: the bit foretold (2: none), and the two counts it was read from.
    expected: u32,
    slot: usize,
    side_slot: usize,
}

fn class(length: usize) -> usize {
    if length < 16 {
        length
    } else if length < 32 {
        16 + (length - 16) / 4
    } else if length < 96 {
        20 + (length - 32) / 16
    } else {
        (24 + (length - 96) / 40).min(CLASSES - 1)
    }
}

/// A table of N counts, each `value`, on the heap.
fn table<T: Clone, const N: usize>(value: T) -> Box<[T; N]> {
    match vec![value; N].into_boxed_slice().try_into() {
        Ok(table) => table,
        Err(_) => loop {},
    }
}

impl Model {
    pub fn new() -> Self {
        Model {
            index: table(0),
            sure: table(0),
            side: table(0),
            place: [0; 2],
            length: [0; 2],
            longer: 0,
            view: 0,
            foretold: 0,
            expected: 2,
            slot: 0,
            side_slot: 0,
        }
    }

    /// At the start of a byte: moves the matches on, or looks for one, and notes the place of the last bytes.
    fn step(&mut self, cx: &Context, history: &[u8]) {
        let n = history.len();
        for m in 0..2 {
            if self.length[m] > 0 {
                if history.get(self.place[m]) == Some(&(cx.last4 as u8)) {
                    self.place[m] += 1;
                    if self.length[m] < REACH {
                        self.length[m] += 1;
                    }
                } else {
                    self.length[m] = 0;
                }
            }
        }
        if n >= SEED {
            let seed = cx.last8 & ((1u64 << (8 * SEED)) - 1);
            let slot = ((seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> (64 - BITS)) as usize) << 1;
            if self.length[0] == 0 || self.length[1] == 0 {
                for found in [self.index[slot] as usize, self.index[slot + 1] as usize] {
                    let free = if self.length[0] == 0 { 0 } else { 1 };
                    if found == 0 || self.length[free] > 0 || (self.length[1 - free] > 0 && self.place[1 - free] == found) {
                        continue;
                    }
                    let mut length = 0;
                    while length < REACH && length < found && history.get(found - 1 - length) == history.get(n - 1 - length) {
                        length += 1;
                    }
                    if length >= SEED {
                        self.length[free] = length;
                        self.place[free] = found;
                    }
                }
            }
            self.index[slot + 1] = self.index[slot];
            self.index[slot] = n as u32;
        }
        // The longer of the two; of two as long, the one at the more recent place.
        let long = ((self.length[1], self.place[1]) > (self.length[0], self.place[0])) as usize;
        let byte = |m: usize| match history.get(self.place[m]) {
            Some(&b) if self.length[m] > 0 => b as u32 | 256,
            _ => 0,
        };
        self.longer = self.length[long];
        self.foretold = byte(long);
        let other = byte(1 - long);
        self.view = if other == 0 {
            0
        } else if other == self.foretold {
            1
        } else if self.length[1 - long] < WEIGHTY {
            2
        } else {
            3
        };
    }

    pub fn predict(&mut self, cx: &Context, history: &[u8], out: &mut [i32]) {
        if cx.bit == 0 {
            self.step(cx, history);
        }
        let bit = cx.bit;
        out[1] = 0;
        out[0] = 0;
        self.expected = 2;
        if self.foretold >> (8 - bit) != cx.partial {
            return;
        }
        self.expected = (self.foretold >> (7 - bit)) & 1;
        self.slot = ((self.view * CLASSES + class(self.longer)) * 8 + bit as usize) % (VIEWS * CLASSES * 8);
        let h = ((self.foretold & 0xFF) << 8 | (cx.last4 & 0xFF)).wrapping_mul(0x9E37_79B1) >> (32 - SIDE_BITS + 3);
        self.side_slot = ((h << 3) | bit) as usize % (1 << SIDE_BITS);
        let first = cx.scale.stretch((self.sure[self.slot] ^ START) as u32 >> 4);
        let second = cx.scale.stretch((self.side[self.side_slot] ^ START) as u32 >> 4);
        let m = self.expected as i32 - 1;
        out[1] = (second ^ m) - m;
        out[0] = (first ^ m) - m;
    }

    pub fn update(&mut self, _cx: &Context, bit: u32) {
        if self.expected == 2 {
            return;
        }
        let came = bit == self.expected;
        for count in [&mut self.sure[self.slot % (VIEWS * CLASSES * 8)], &mut self.side[self.side_slot % (1 << SIDE_BITS)]] {
            let p = (*count ^ START) as u32;
            *count = if came { p + ((65535 - p) >> RATE) } else { p - (p >> RATE) } as u16 ^ START;
        }
    }
}
