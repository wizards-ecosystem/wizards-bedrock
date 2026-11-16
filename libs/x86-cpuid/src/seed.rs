use core::fmt;

const RDRAND_BIT: u32 = 1 << 30;

const RDSEED_BIT: u32 = 1 << 18;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeedSource {
    Rdseed,
    Rdrand,
}

impl SeedSource {
    // `RDRAND` returns a conditioned generator's output, so it is credited only without `RDSEED`.
    pub const fn read(leaf_1_ecx: u32, leaf_7_ebx: u32) -> Option<Self> {
        if leaf_7_ebx & RDSEED_BIT != 0 {
            Some(Self::Rdseed)
        } else if leaf_1_ecx & RDRAND_BIT != 0 {
            Some(Self::Rdrand)
        } else {
            None
        }
    }
}

impl fmt::Display for SeedSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Rdseed => "rdseed",
            Self::Rdrand => "rdrand",
        })
    }
}
