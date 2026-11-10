//! Which numbering the bytes arriving are in, read from the bytes themselves.
//!
//! Every other way of asking has been observed to lie. A controller that
//! translates while reporting the bit clear is one real machine (D-178); a
//! controller that reports a translated identity *and* a translated set query
//! while delivering set 2 is another (D-239). The two disagree about which
//! probe is honest, so no probe is.
//!
//! The stream cannot lie, because the encodings are distinguishable:
//!
//! - Set 2 prefixes every release with `F0`. Set 1 has no `70` key, so `F0`
//!   is not a release it can produce either — `F0` means set 2 wherever it
//!   appears, including inside Pause and after `E0`.
//! - Set 1 marks a release by setting the high bit, and its makes stop at
//!   `5D`, so its releases run to `DD`. Set 2's makes stop at `83`. A byte
//!   above `83` that is not a reply either set can send is therefore set 1.
//!
//! Two bytes are excluded for being both: `AA`, set 2's self-test reply and
//! set 1's left-shift release, and `AB`, the first byte of every identity
//! reply and set 1's backslash release. Letting a reply answer this would
//! reinstate the probe the stream is here to replace.
//!
//! Both bounds are read off the tables in `set1` and `table` rather than
//! chosen, and `the_two_tables_still_bound_this_probe` fails if either grows
//! past them.

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

/// Set 2's break prefix, and the whole of why it is decisive.
const BREAK: u8 = 0xF0;

/// The highest make code set 2 has. Above it, key data is not set 2's.
const SET_2_HIGHEST_MAKE: u8 = 0x83;

/// The highest release set 1 can produce: its highest make, high bit set.
const SET_1_HIGHEST_BREAK: u8 = 0xDD;

/// Bytes in set 1's release range that set 2 can also send, so they decide
/// nothing.
///
/// `AA` is set 2's self-test reply and set 1's left-shift release;
/// [`Decoder`](crate::Decoder) documents the same collision from the other
/// side. `AB` is the first byte of every identity reply — the very probe whose
/// dishonesty this exists to route around — and set 1's backslash release. A
/// reply that reaches the stream must not be allowed to answer the question
/// the stream is being asked.
const AMBIGUOUS: [u8; 2] = [0xAA, 0xAB];

/// Reads the numbering off the stream without consuming it.
///
/// Stateless on purpose: every rule above holds for a byte in any position,
/// so there is no sequence to track and no state that could disagree with the
/// decoder's.
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
