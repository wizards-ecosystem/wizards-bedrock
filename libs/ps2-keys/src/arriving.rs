//! Which numbering arriving bytes are in, read from the bytes since controllers' translation bits and identity replies have been seen to lie.

/// What the bytes so far have proved about their numbering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arriving {
    /// Nothing seen yet distinguishes the two.
    Undecided,
    /// A translating controller's set 1.
    Set1,
    /// The set the keyboard sends when nothing rewrites it.
    Set2,
}

/// Set 2's break prefix; set 1 has no `70` key, so `F0` means set 2 wherever it appears.
const BREAK: u8 = 0xF0;

/// The highest make code set 2 has. Above it, key data is not set 2's.
const SET_2_HIGHEST_MAKE: u8 = 0x83;

/// The highest release set 1 can produce: its highest make, high bit set.
const SET_1_HIGHEST_BREAK: u8 = 0xDD;

/// Set 1 releases set 2 also sends as replies (`AA` self-test, `AB` identity), so they must never decide.
const AMBIGUOUS: [u8; 2] = [0xAA, 0xAB];

/// Reads the numbering off the stream without consuming it, statelessly since every rule holds at any position.
#[derive(Clone, Copy, Default, Debug)]
pub struct SetProbe;

impl SetProbe {
    /// What one byte proves, if anything.
    ///
    /// # Examples
    ///
    /// ```
    /// use ps2_keys::{Arriving, SetProbe};
    ///
    /// // `F0` is set 2's break prefix and set 1 cannot produce it.
    /// assert_eq!(SetProbe::observe(0xF0), Arriving::Set2);
    /// // `9E` is A released in set 1, and above every set 2 make.
    /// assert_eq!(SetProbe::observe(0x9E), Arriving::Set1);
    /// // A bare make is the same shape in both.
    /// assert_eq!(SetProbe::observe(0x1C), Arriving::Undecided);
    /// ```
    #[must_use]
    pub const fn observe(byte: u8) -> Arriving {
        if byte == BREAK {
            return Arriving::Set2;
        }
        if byte > SET_2_HIGHEST_MAKE
            && byte <= SET_1_HIGHEST_BREAK
            && byte != AMBIGUOUS[0]
            && byte != AMBIGUOUS[1]
        {
            return Arriving::Set1;
        }
        Arriving::Undecided
    }
}

#[cfg(test)]
mod tests;
