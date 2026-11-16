use anyhow::{Context, Result, anyhow, ensure};
use elf64::{Elf, SegmentKind};

use crate::cargo;
use crate::paths::Paths;

const PAGE_BYTES: u64 = 4096;

// The small code model's reach, and so where every loaded byte must end.
const LOAD_LIMIT: u64 = 1 << 31;

// The kernel loader's segment limit.
const MAX_LOADS: usize = 8;

pub(crate) fn build(paths: &Paths) -> Result<Vec<u8>> {
    let bytes = cargo::build_witness(paths)?;
    check(&bytes).context("the ring 3 witness breaks the image contract")?;
    Ok(bytes)
}

fn check(bytes: &[u8]) -> Result<()> {
    let elf =
        Elf::parse(bytes).map_err(|error| anyhow!("not a static x86_64 executable: {error}"))?;
    let segments: Vec<_> = elf.segments().collect();
    ensure!(
        segments
            .first()
            .is_some_and(|text| text.kind() == SegmentKind::Load && text.is_executable()),
        "program header 0 is not the text load"
    );
    let mut loads = 0;
    for segment in segments {
        let at = segment.address();
        match segment.kind() {
            SegmentKind::Dynamic | SegmentKind::Interpreter => {
                return Err(anyhow!("it has a dynamic section or an interpreter"));
            }
            SegmentKind::Load => {
                loads += 1;
                ensure!(
                    at.is_multiple_of(PAGE_BYTES),
                    "a load at {at:#x} is not page aligned"
                );
                ensure!(
                    !segment.is_write_execute(),
                    "the load at {at:#x} is writable and executable"
                );
                ensure!(segment.memory_size() != 0, "the load at {at:#x} is empty");
                ensure!(
                    at.checked_add(segment.memory_size())
                        .is_some_and(|end| end <= LOAD_LIMIT),
                    "the load at {at:#x} ends above 2 GiB"
                );
            }
            _ => {}
        }
    }
    ensure!(
        loads <= MAX_LOADS,
        "it has {loads} loads, more than {MAX_LOADS}"
    );
    Ok(())
}
