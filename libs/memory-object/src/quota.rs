use core::fmt;

// Pages used in bits 0 to 31, the limit in bits 32 to 63, in one `AtomicU64`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quota {
    limit: u32,
    used: u32,
}

impl Quota {
    pub fn new(limit: u32) -> Self {
        Self { limit, used: 0 }
    }

    pub fn from_bits(bits: u64) -> Option<Self> {
        let quota = Self {
            limit: (bits >> u32::BITS) as u32,
            used: bits as u32,
        };
        if quota.used > quota.limit {
            return None;
        }
        Some(quota)
    }

    pub fn bits(self) -> u64 {
        (u64::from(self.limit) << u32::BITS) | u64::from(self.used)
    }

    pub fn limit(self) -> u32 {
        self.limit
    }

    pub fn used(self) -> u32 {
        self.used
    }

    pub fn debit(self, pages: u32) -> Result<Self, QuotaRefusal> {
        if pages > self.limit - self.used {
            return Err(QuotaRefusal::NoQuota);
        }
        Ok(Self {
            used: self.used + pages,
            ..self
        })
    }

    pub fn credit(self, pages: u32) -> Result<Self, QuotaRefusal> {
        if pages > self.used {
            return Err(QuotaRefusal::Overcredited);
        }
        Ok(Self {
            used: self.used - pages,
            ..self
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuotaRefusal {
    NoQuota,
    Overcredited,
}

impl fmt::Display for QuotaRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoQuota => "the quota cannot cover the pages",
            Self::Overcredited => "the credit exceeds the pages the quota has used",
        })
    }
}

impl core::error::Error for QuotaRefusal {}
