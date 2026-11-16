use core::ptr::null_mut;
use core::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

use frame_space::{FRAME_SIZE, Owner, Record};

use super::{Space, direct_pointer, frame::frame_address};

static BASE: AtomicPtr<AtomicU64> = AtomicPtr::new(null_mut());

static LEN: AtomicUsize = AtomicUsize::new(0);

// One record per frame below the highest managed one, carved from managed memory once at init.
pub(super) fn carve(space: &mut Space) -> Option<usize> {
    let records = space.limit();
    let frames = (records * size_of::<AtomicU64>()).div_ceil(FRAME_SIZE as usize);
    let first = space.allocate_contiguous(frames, 1)?;
    let bytes = frames as u64 * FRAME_SIZE;
    let base = direct_pointer(frame_address(first), bytes)?;
    // SAFETY: the run was just taken from the allocator, so nothing else names it, and the
    // bootloader's direct map covers every managed frame.
    unsafe { core::ptr::write_bytes(base, 0, bytes as usize) };
    BASE.store(base.cast(), Ordering::Release);
    LEN.store(records, Ordering::Release);
    for frame in first..first + frames {
        claim(frame, Owner::Kernel);
    }
    Some(frames)
}

fn slot(frame: usize) -> &'static AtomicU64 {
    assert!(
        frame < LEN.load(Ordering::Acquire),
        "frame {frame} has no record"
    );
    let word = BASE.load(Ordering::Acquire).wrapping_add(frame);
    // SAFETY: `carve` zeroed `LEN` aligned words at the base, which are never freed and are reached
    // only atomically, so a shared reference to one of them is sound for the rest of the boot.
    unsafe { &*word }
}

pub(super) fn read(frame: usize) -> Record {
    let word = slot(frame).load(Ordering::Acquire);
    Record::decode(word).unwrap_or_else(|| panic!("frame {frame} holds a corrupt record {word:#x}"))
}

fn write(frame: usize, record: Record) {
    slot(frame).store(record.encode(), Ordering::Release);
}

pub(super) fn claim(frame: usize, owner: Owner) {
    let Some(record) = read(frame).allocated(owner) else {
        panic!("frame {frame} was handed out while its record names an owner");
    };
    write(frame, record);
}

pub(super) fn seal(frame: usize) {
    let Some(record) = read(frame).sealed() else {
        panic!("frame {frame} cannot be sealed from its current record");
    };
    write(frame, record);
}

pub(super) fn release(frame: usize) -> Record {
    let current = read(frame);
    let Some(record) = current.released() else {
        panic!("frame {frame} was released while its record is already free");
    };
    write(frame, record);
    current
}
