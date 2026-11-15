// Translation bits, identity replies and `F0 00` set queries have each been seen to lie.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arriving {
    Undecided,
    Set1,
    Set2,
}

// Set 1 has no `70` key, so `F0` means set 2 wherever it appears.
const BREAK: u8 = 0xF0;

const SET_2_HIGHEST_MAKE: u8 = 0x83;

const SET_1_HIGHEST_BREAK: u8 = 0xDD;

// Set 2 also sends these, so they never decide: `AA` self-test, `AB` identity, `84` SysRq.
const AMBIGUOUS: [u8; 3] = [0xAA, 0xAB, 0x84];

#[derive(Clone, Copy, Default, Debug)]
pub struct SetProbe;

impl SetProbe {
    #[must_use]
    pub const fn observe(byte: u8) -> Arriving {
        if byte == BREAK {
            return Arriving::Set2;
        }
        if byte > SET_2_HIGHEST_MAKE
            && byte <= SET_1_HIGHEST_BREAK
            && byte != AMBIGUOUS[0]
            && byte != AMBIGUOUS[1]
            && byte != AMBIGUOUS[2]
        {
            return Arriving::Set1;
        }
        Arriving::Undecided
    }
}

#[cfg(test)]
mod tests;
