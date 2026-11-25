//! The logistic scale. A probability p of a 1, out of 4096, is `stretch`ed to ln(p / (1 - p)) in steps of 1/256,
//! from -2048 to 2047; `squash` goes back. Predictions are added and weighed on the stretched scale, where a
//! confident model counts for more than a hesitant one, and squashed once, for the coder.
//!
//! Twelve bits are too few where the prediction is nearly sure (the last step before certainty is 1 in 4096,
//! and the steps before it are as coarse): `squash_fine` gives the same probability out of 65536, for the coder,
//! and `stretch` is read against it, so that each probability goes to its nearest step there too.
//!
//! Both are tables, filled when the program starts, with the four operations alone (the program has no
//! mathematical library): the same tables on every machine.

use alloc::vec::Vec;

pub struct Scale {
    squash: Vec<u16>,
    fine: Vec<u16>,
    stretch: Vec<i16>,
}

impl Scale {
    pub fn new() -> Self {
        let mut squash = Vec::with_capacity(4096);
        for d in -2048i32..2048 {
            let p = (4096.0 / (1.0 + exp(-(d as f64) / 256.0)) + 0.5) as u32;
            squash.push(p.clamp(1, 4095) as u16);
        }
        let mut fine = Vec::with_capacity(4096);
        for d in -2048i32..2048 {
            let p = (65536.0 / (1.0 + exp(-(d as f64) / 256.0)) + 0.5) as u32;
            fine.push(p.clamp(1, 65535) as u16);
        }
        // stretch(p): the step whose sixteen-bit squash is nearest p, read out of 65536. Near 0 and 1 the
        // twelve-bit squash is flat over many steps, and its first step that reaches p is off by up to half a
        // twelve-bit unit, the most where the prediction is nearly sure; the fine squash tells those steps apart.
        let mut stretch = Vec::with_capacity(4096);
        let mut d = 0usize;
        for p in 0..4096u32 {
            let want = p << 4;
            while d < 4095 && (fine[d] as u32) < want {
                d += 1;
            }
            let near = if d > 0 && want - (fine[d - 1] as u32) < (fine[d] as u32).saturating_sub(want) { d - 1 } else { d };
            stretch.push((near as i32 - 2048) as i16);
        }
        Scale { squash, fine, stretch }
    }

    /// The probability, out of 4096 (1 to 4095), of a stretched prediction.
    pub fn squash(&self, d: i32) -> u32 {
        self.squash[(d.clamp(-2048, 2047) + 2048) as usize] as u32
    }

    /// The same probability out of 65536 (1 to 65535): sixteen bits, for the coder.
    pub fn squash_fine(&self, d: i32) -> u32 {
        self.fine[(d.clamp(-2048, 2047) + 2048) as usize] as u32
    }

    /// The stretched prediction (-2048 to 2047) of a probability out of 4096.
    pub fn stretch(&self, p: u32) -> i32 {
        self.stretch[p.min(4095) as usize] as i32
    }
}

/// e to the x, for the few hundred values the tables need: x = k ln 2 + r, the series of e to the r, then k doublings.
fn exp(x: f64) -> f64 {
    const LN2: f64 = 0.693_147_180_559_945_3;
    let k = if x >= 0.0 { (x / LN2 + 0.5) as i32 } else { -((-x / LN2 + 0.5) as i32) };
    let r = x - (k as f64) * LN2;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut n = 1;
    while n <= 16 {
        term *= r / (n as f64);
        sum += term;
        n += 1;
    }
    let mut doubled = 1.0;
    let mut i = 0;
    while i < k.abs() {
        doubled *= 2.0;
        i += 1;
    }
    if k >= 0 {
        sum * doubled
    } else {
        sum / doubled
    }
}
