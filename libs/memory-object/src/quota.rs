#[cfg(verus_only)]
use vstd::prelude::*;
use vstd::prelude::{proof, verus_spec, verus_verify};

// Pages used in bits 0 to 31, the limit in bits 32 to 63, in one `AtomicU64`.
#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Quota {
    pub(crate) limit: u32,
    pub(crate) used: u32,
}

#[verus_verify]
impl Quota {
    #[verus_spec(r =>
        ensures
            r.spec_limit() == limit,
            r.spec_used() == 0,
    )]
    pub fn new(limit: u32) -> Self {
        Self { limit, used: 0 }
    }

    #[verus_spec(r =>
        ensures
            match r {
                Some(quota) => quota.spec_bits() == bits,
                None => (bits & 0xffff_ffff) > (bits >> 32u32),
            },
    )]
    pub fn from_bits(bits: u64) -> Option<Self> {
        proof! {
            crate::proof::lemma_quota_unpacks(bits);
        }
        let limit = (bits >> u32::BITS) as u32;
        let used = (bits & 0xffff_ffff) as u32;
        if used > limit {
            return None;
        }
        Some(Self { limit, used })
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_bits(),
            forall|other: Quota| other.spec_bits() == r ==> other == self,
    )]
    pub fn bits(self) -> u64 {
        proof! {
            crate::proof::lemma_quota_packs(self.limit as u64, self.used as u64);
            assert forall|other: Quota| other.spec_bits() == self.spec_bits() implies other
                == self by {
                crate::proof::lemma_quota_packs(other.limit as u64, other.used as u64);
            }
        }
        (u64::from(self.limit) << u32::BITS) | u64::from(self.used)
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_limit(),
    )]
    pub fn limit(self) -> u32 {
        self.limit
    }

    #[verus_spec(r =>
        ensures
            r == self.spec_used(),
            r <= self.spec_limit(),
    )]
    pub fn used(self) -> u32 {
        proof! {
            use_type_invariant(&self);
        }
        self.used
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => crate::proof::quota_covers(self, pages)
                    && next.spec_limit() == self.spec_limit()
                    && next.spec_used() == self.spec_used() + pages,
                Err(QuotaRefusal::NoQuota) => !crate::proof::quota_covers(self, pages),
                Err(_) => false,
            },
    )]
    pub fn debit(self, pages: u32) -> Result<Self, QuotaRefusal> {
        proof! {
            use_type_invariant(&self);
        }
        if pages > self.limit - self.used {
            return Err(QuotaRefusal::NoQuota);
        }
        Ok(Self {
            used: self.used + pages,
            ..self
        })
    }

    #[verus_spec(r =>
        ensures
            match r {
                Ok(next) => pages <= self.spec_used()
                    && next.spec_limit() == self.spec_limit()
                    && next.spec_used() == self.spec_used() - pages,
                Err(QuotaRefusal::Overcredited) => pages > self.spec_used(),
                Err(_) => false,
            },
    )]
    pub fn credit(self, pages: u32) -> Result<Self, QuotaRefusal> {
        proof! {
            use_type_invariant(&self);
        }
        if pages > self.used {
            return Err(QuotaRefusal::Overcredited);
        }
        Ok(Self {
            used: self.used - pages,
            ..self
        })
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QuotaRefusal {
    NoQuota,
    Overcredited,
}
