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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, Copy, Default)]
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
mod tests {
    use super::*;
    use crate::{set1, table};

    /// The probe's two bounds are the tables', not numbers chosen beside them.
    ///
    /// This is the check that makes the rule an argument rather than a claim:
    /// if either table grows a code past the bound the probe assumes, the
    /// discriminator stops being sound and this fails before a machine can
    /// mis-decode a keystroke.
    #[test]
    fn the_two_tables_still_bound_this_probe() {
        for code in 0..=u8::MAX {
            for (set_2, set_1) in [
                (table::key_for(code), set1::key_for(code)),
                (table::extended_key_for(code), set1::extended_key_for(code)),
            ] {
                assert!(
                    set_2.is_none() || code <= SET_2_HIGHEST_MAKE,
                    "set 2 make {code:#04x} is above the probe's bound"
                );
                assert!(
                    set_1.is_none() || code | 0x80 <= SET_1_HIGHEST_BREAK,
                    "set 1 make {code:#04x} releases above the probe's bound"
                );
            }
        }
        let unshifted = BREAK & !0x80;
        assert!(
            set1::key_for(unshifted).is_none() && set1::extended_key_for(unshifted).is_none(),
            "set 1 gained a {unshifted:#04x} key, so F0 is now one of its releases"
        );
    }

    /// An identity reply that reaches the stream answers nothing.
    ///
    /// `AB 41` and `AB 83` are the replies whose dishonesty this probe exists
    /// to route around, so letting the first byte of either decide would put
    /// that answer straight back in the stream's mouth. `AB` is also a real
    /// set 1 backslash release, which is why it has to be excluded rather than
    /// argued away.
    #[test]
    fn an_identity_reply_in_the_stream_decides_nothing() {
        for reply in [[0xABu8, 0x41], [0xAB, 0x83]] {
            for byte in reply {
                assert_eq!(
                    SetProbe::observe(byte),
                    Arriving::Undecided,
                    "{byte:#04x} of an identity reply decided the numbering"
                );
            }
        }
    }

    /// Every byte either proves one set or proves nothing — never both.
    #[test]
    fn no_byte_proves_both_sets() {
        for byte in 0..=u8::MAX {
            let _ = SetProbe::observe(byte);
        }
        assert_eq!(SetProbe::observe(BREAK), Arriving::Set2);
        for byte in AMBIGUOUS {
            assert_eq!(
                SetProbe::observe(byte),
                Arriving::Undecided,
                "{byte:#04x} is a reply as well as a set 1 release"
            );
        }
        assert_eq!(SetProbe::observe(SET_2_HIGHEST_MAKE), Arriving::Undecided);
        assert_eq!(SetProbe::observe(SET_1_HIGHEST_BREAK), Arriving::Set1);
        assert_eq!(SetProbe::observe(0xDE), Arriving::Undecided);
    }

    /// A real keystroke from each set decides, and decides correctly.
    ///
    /// The exact bytes the two machines delivered: VMware's `c` arrived as
    /// set 2 while its identity claimed set 1, and the same key under a
    /// controller that really translates arrives as set 1.
    #[test]
    fn one_keystroke_from_either_machine_decides_it() {
        let set_2 = [0x21u8, 0xF0, 0x21];
        let decided = set_2.iter().map(|byte| SetProbe::observe(*byte));
        assert_eq!(
            decided.clone().find(|seen| *seen != Arriving::Undecided),
            Some(Arriving::Set2)
        );
        assert_eq!(decided.count(), 3, "the probe consumes nothing");

        let set_1 = [0x2Eu8, 0xAE];
        assert_eq!(
            set_1
                .iter()
                .map(|byte| SetProbe::observe(*byte))
                .find(|seen| *seen != Arriving::Undecided),
            Some(Arriving::Set1)
        );
    }

    /// Set 2's Pause sequence decides on its `F0`, not on its high bytes.
    #[test]
    fn the_longest_set_2_sequence_still_decides_set_2() {
        let pause = [0xE1u8, 0x14, 0x77, 0xE1, 0xF0, 0x14, 0xF0, 0x77];
        assert_eq!(
            pause
                .iter()
                .map(|byte| SetProbe::observe(*byte))
                .find(|seen| *seen != Arriving::Undecided),
            Some(Arriving::Set2)
        );
    }

    /// Set 1's Pause sequence decides set 1, and never set 2.
    #[test]
    fn the_longest_set_1_sequence_never_says_set_2() {
        let pause = [0xE1u8, 0x1D, 0x45, 0xE1, 0x9D, 0xC5];
        let seen: Vec<Arriving> = pause.iter().map(|byte| SetProbe::observe(*byte)).collect();
        assert!(!seen.contains(&Arriving::Set2), "{seen:?}");
        assert!(seen.contains(&Arriving::Set1));
    }
}
