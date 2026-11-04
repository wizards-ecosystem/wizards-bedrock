//! Which numbering the bytes arriving are in, read from the bytes themselves.

/// What the bytes so far have proved about their numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed encoding; a new variant is a protocol bug, not a compatible extension"
)]
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
const AMBIGUOUS: [u8; 2] = [0xAA, 0xAB];

/// Reads the numbering off the stream without consuming it.
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
