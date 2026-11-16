use crate::sync::ThreadContext;

// Processors that may hold a stale translation; one CPU invalidates first, so tokens start done.
#[derive(PartialEq, Eq, Debug)]
#[must_use = "a translation may survive until the flush completes"]
pub struct Flush {
    cpus: u64,
    generation: u64,
}

impl Flush {
    pub const fn none() -> Self {
        Self {
            cpus: 0,
            generation: 0,
        }
    }

    pub const fn merge(&mut self, other: Self) {
        self.cpus |= other.cpus;
        if other.generation > self.generation {
            self.generation = other.generation;
        }
    }

    pub const fn is_complete(&self) -> bool {
        self.cpus == 0
    }

    pub fn wait(self, context: &mut ThreadContext) {
        let _ = context;
        assert!(
            self.is_complete(),
            "a flush named a processor that cannot acknowledge it"
        );
    }
}
