//! The arithmetic coder. It is told, for each bit, the probability that the bit is 1, and writes the sequence in
//! about as many bits as those probabilities say it is worth: a bit foreseen at 4000 in 4096 costs a thirtieth of
//! a bit, a bit foreseen wrongly at the same odds costs five. The probabilities are out of 65536: a bit that is
//! nearly sure can cost down to a forty-five-thousandth of a bit. The decoder is told the same probabilities in the
//! same order and gives the bits back.
//!
//! The state is an interval [low, high] of 32-bit numbers. Each bit keeps the part of it its probability gives;
//! a leading byte that low and high share is settled and goes to the output.

use alloc::vec::Vec;

pub struct Encoder {
    low: u32,
    high: u32,
    out: Vec<u8>,
    start: usize,
}

impl Encoder {
    /// An encoder that writes after what `out` already holds.
    pub fn new(out: Vec<u8>) -> Self {
        let start = out.len();
        Encoder { low: 0, high: 0xFFFF_FFFF, out, start }
    }

    /// Codes one bit; `p` is the probability that it is 1, out of 65536 (1 to 65535).
    pub fn code(&mut self, bit: u32, p: u32) {
        let middle = self.low + (((self.high - self.low) as u64 * p as u64) >> 16) as u32;
        if bit == 1 {
            self.high = middle;
        } else {
            self.low = middle + 1;
        }
        while (self.low ^ self.high) & 0xFF00_0000 == 0 {
            self.out.push((self.high >> 24) as u8);
            self.low <<= 8;
            self.high = (self.high << 8) | 0xFF;
        }
    }

    /// Ends the message. The decoder reads zeros past the end, so one byte is enough to name a number of the
    /// last interval (none if zero is in it), and the zeros the message would end with are left out.
    pub fn finish(mut self) -> Vec<u8> {
        if self.low != 0 {
            self.out.push(((self.low >> 24) + 1) as u8);
        }
        while self.out.len() > self.start && self.out[self.out.len() - 1] == 0 {
            self.out.pop();
        }
        self.out
    }
}

pub struct Decoder<'a> {
    low: u32,
    high: u32,
    code: u32,
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        let mut decoder = Decoder { low: 0, high: 0xFFFF_FFFF, code: 0, bytes, at: 0 };
        for _ in 0..4 {
            decoder.code = (decoder.code << 8) | decoder.next();
        }
        decoder
    }

    /// The next byte of the message; past its end, zeros.
    fn next(&mut self) -> u32 {
        let byte = if self.at < self.bytes.len() { self.bytes[self.at] as u32 } else { 0 };
        self.at += 1;
        byte
    }

    /// Gives back one bit, told the probability the encoder was told.
    pub fn decode(&mut self, p: u32) -> u32 {
        let middle = self.low + (((self.high - self.low) as u64 * p as u64) >> 16) as u32;
        let bit = if self.code <= middle { 1 } else { 0 };
        if bit == 1 {
            self.high = middle;
        } else {
            self.low = middle + 1;
        }
        while (self.low ^ self.high) & 0xFF00_0000 == 0 {
            self.low <<= 8;
            self.high = (self.high << 8) | 0xFF;
            self.code = (self.code << 8) | self.next();
        }
        bit
    }
}
