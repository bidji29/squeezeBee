//! The transform: rewrites the data into a text that is easier to model, before anything is coded, and rewrites it
//! back, exactly, after everything is decoded.
//!
//! Three rewritings, each undone exactly:
//! - a capital letter is written as a marker and the small letter: CAP before a word whose first letter alone is
//!   a capital (and before each capital of a word of mixed case), UP before a word of two letters or more that is
//!   all capitals. A word is a longest run of ASCII letters;
//! - the curly quotes and the long dash, three bytes each in UTF-8, are written as one byte each;
//! - the most frequent words are written as one byte each, from 128 up, after their marker if they have one.
//! A byte of the data that would be taken for one of these (1 to 8, or 128 and more) is written after ESC.
//!
//! Before those, the line breaks of wrapped prose are written as spaces (see `walk`), so that the models see
//! the words of a paragraph run on as they do inside a line.

use alloc::borrow::Cow;
use alloc::vec::Vec;

const ESC: u8 = 1;
const CAP: u8 = 2;
const UP: u8 = 3;

/// The third byte of the sequences E2 80 xx that are written as the bytes 4, 5, 6, 7 and 8.
const MARKS: [u8; 5] = [0x99, 0x98, 0x9C, 0x9D, 0x94];
const FIRST_MARK: u8 = 4;

/// The words written as one byte, the first as 128: the ones that save most on this kind of text.
const FIRST_WORD: u8 = 128;
const WORDS: &str = "\
    the and that with you to of have this his for not in your what was her but my which there will it \
    him would shall thou said he when she from is all little should me they had their as good more were \
    be come are then know before here upon well them never like thee enter lord than now where king so \
    think very could been sir our love nothing thy at these first on some one such another again \
    gentleman out time being richard himself about myself father great much make thought by must how \
    into if we no though man other house without say scene night heart better mr therefore who do take \
    down something hath speak made hand might micawber friend lady let";

/// One byte more at the end of `out` (written once, called from everywhere: the program is counted too).
#[inline(never)]
fn put(out: &mut Vec<u8>, b: u8) {
    out.push(b);
}

/// The width prose is wrapped at, in characters: a line break is put back where the next word would not fit.
const WIDTH: usize = 72;

/// Before a space that the rule would take for a line break, and before itself: the next byte is the data's own.
const KEEP: u8 = 0x1F;

/// Whether a byte starts a character (UTF-8: it is not a continuation byte).
fn starts_char(b: u8) -> bool {
    b & 0xC0 != 0x80
}

/// The entry of WORDS that is `key`, or else the `n`th: its number and its letters (none past the last entry).
#[inline(never)]
fn lookup(key: &[u8], n: usize) -> (usize, &'static [u8]) {
    for (k, word) in WORDS.as_bytes().split(|&c| c == b' ').enumerate() {
        if word == key || k == n {
            return (k, word);
        }
    }
    (0, &[])
}

/// How many characters the word at `j` holds, up to the next space or line break. In the text `walk` wraps
/// (`keep`), KEEP before a space ends the word, and KEEP before another byte stands for that byte.
fn ahead(t: &[u8], mut j: usize, keep: bool) -> usize {
    let mut word = 0;
    // KEEP, or a value no byte has.
    let mark = if keep { KEEP as u32 } else { 256 };
    while j < t.len() && t[j] != b' ' && t[j] != b'\n' {
        if t[j] as u32 == mark {
            j += 1;
            if j >= t.len() || t[j] == b' ' {
                break;
            }
        }
        if starts_char(t[j]) {
            word += 1;
        }
        j += 1;
    }
    word
}

/// One walk for both ways. Unwrapping (`back` false): each line break that the rule gives is written as a space,
/// and each space that the rule would take for a line break is written after KEEP (and KEEP itself after KEEP).
/// Wrapping (`back` true) puts them back exactly. The rule: at a space or a line break, after a line of `line`
/// characters, the line breaks there if the next word does not fit.
fn walk(t: &[u8], back: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let mut line = 0;
    let mut i = 0;
    while let Some(&c) = t.get(i) {
        let mut b = c;
        i += 1;
        // The data's own byte, whichever way.
        let mut d = b;
        if b == b' ' || (b == b'\n' && !back) {
            let word = ahead(t, i, back);
            let rule = word >= 1 && line + 1 + word > WIDTH;
            if back {
                if rule {
                    d = b'\n';
                    b = d;
                }
            } else if b == b'\n' {
                if rule {
                    b = b' ';
                }
            } else if rule {
                put(&mut out, KEEP);
            }
        } else if b == KEEP {
            if back {
                if i < t.len() {
                    d = t[i];
                    b = d;
                    i += 1;
                }
            } else {
                put(&mut out, KEEP);
            }
        }
        put(&mut out, b);
        line = if d == b'\n' { 0 } else { line + starts_char(d) as usize };
    }
    out
}

/// The text the models are given for the data.
pub fn forward(data: &[u8]) -> Cow<'_, [u8]> {
    let data = walk(data, false);
    let mut out = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let b = data[i];
        let mut j = i;
        let mut capitals = 0;
        while j < data.len() && data[j].is_ascii_alphabetic() {
            capitals += data[j].is_ascii_uppercase() as usize;
            j += 1;
        }
        if j > i {
            // UP before a word all of capitals, else CAP before each capital; then the small letters, which
            // the word's byte replaces if the word is one of WORDS and the markers are none, CAP first, or UP.
            let up = capitals >= 2 && capitals == j - i;
            // (A letter is a capital if it is below 'a'; the first is one only if `capitals` counts it.)
            let whole = up || capitals == (b < b'a') as usize;
            if up {
                put(&mut out, UP);
            }
            let start = out.len() + (capitals == 1) as usize;
            // A letter is a capital if it is below 'a'; under UP, none takes CAP.
            let below = if up { 0 } else { b'a' };
            for &c in &data[i..j] {
                if c < below {
                    put(&mut out, CAP);
                }
                put(&mut out, c | 32);
            }
            if whole {
                let (k, word) = lookup(&out[start..], usize::MAX);
                if !word.is_empty() {
                    out.truncate(start);
                    put(&mut out, FIRST_WORD + k as u8);
                }
            }
            i = j;
            continue;
        }
        i += 1;
        if let (0xE2, Some(&[0x80, m])) = (b, data.get(i..i + 2)) {
            if let Some(k) = MARKS.iter().position(|&x| x == m) {
                put(&mut out, FIRST_MARK + k as u8);
                i += 2;
                continue;
            }
        }
        if (ESC..FIRST_MARK + MARKS.len() as u8).contains(&b) || b >= FIRST_WORD {
            put(&mut out, ESC);
        }
        put(&mut out, b);
    }
    Cow::Owned(out)
}

/// The data, from the text `forward` made of it.
pub fn inverse(text: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::new();
    // The marker that waits for its letters: 0, CAP (for one letter) or UP (for the word's).
    let mut marker = 0;
    let mut i = 0;
    while let Some(&b) = text.get(i) {
        i += 1;
        let one = [b];
        let word = if b >= FIRST_WORD { lookup(&[], (b - FIRST_WORD) as usize).1 } else { &one[..] };
        if b >= FIRST_WORD || b.is_ascii_lowercase() {
            for &c in word {
                put(&mut out, if marker != 0 { c ^ 32 } else { c });
                if marker == CAP {
                    marker = 0;
                }
            }
            continue;
        }
        marker = 0;
        if b == ESC {
            if let Some(&c) = text.get(i) {
                put(&mut out, c);
            }
            i += 1;
        } else if b == CAP || b == UP {
            marker = b;
        } else if (FIRST_MARK..FIRST_MARK + MARKS.len() as u8).contains(&b) {
            for c in [0xE2, 0x80, MARKS[(b - FIRST_MARK) as usize]] {
                put(&mut out, c);
            }
        } else {
            put(&mut out, b);
        }
    }
    walk(&out, true)
}
