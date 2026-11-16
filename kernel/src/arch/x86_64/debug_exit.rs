use wost::arch::current::port::{self, Port};

// QEMU exits with `(value << 1) | 1`: 33 when every check passed, 35 when any failed.
const PASSED: u32 = 0x10;

const FAILED: u32 = 0x11;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Verdict {
    Passed,
    Failed,
}

impl Verdict {
    pub(crate) const fn from_passed(passed: bool) -> Self {
        if passed { Self::Passed } else { Self::Failed }
    }
}

pub(crate) fn report(verdict: Verdict) {
    if let Some(mut device) = port::debug_exit() {
        let code = match verdict {
            Verdict::Passed => PASSED,
            Verdict::Failed => FAILED,
        };
        Port::write(&mut device, code);
    }
}
