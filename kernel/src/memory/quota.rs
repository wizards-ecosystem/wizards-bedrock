use core::sync::atomic::{AtomicU64, Ordering};

use memory_object::{Quota, QuotaRefusal};

// The pages of live sealed executable objects charged to it, never above its limit.
#[derive(Debug)]
pub(crate) struct CodeQuota {
    word: AtomicU64,
}

impl CodeQuota {
    pub(crate) fn new(limit: u32) -> Self {
        Self {
            word: AtomicU64::new(Quota::new(limit).bits()),
        }
    }

    pub(crate) fn debit(&self, pages: u32) -> Result<(), QuotaRefusal> {
        self.step(|quota| quota.debit(pages))
    }

    pub(crate) fn credit(&self, pages: u32) {
        if let Err(refusal) = self.step(|quota| quota.credit(pages)) {
            panic!("a code quota was credited pages it never charged: {refusal}");
        }
    }

    fn step(
        &self,
        next: impl Fn(Quota) -> Result<Quota, QuotaRefusal>,
    ) -> Result<(), QuotaRefusal> {
        let mut seen = self.word.load(Ordering::Acquire);
        loop {
            let Some(quota) = Quota::from_bits(seen) else {
                panic!("a code quota word {seen:#x} is not a quota");
            };
            let wanted = next(quota)?.bits();
            match self
                .word
                .compare_exchange_weak(seen, wanted, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(()),
                Err(current) => seen = current,
            }
        }
    }
}
