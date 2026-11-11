use alloc::boxed::Box;
use alloc::vec::Vec;

use wost::mm;

pub(super) fn frame_round_trips() -> bool {
    let before = mm::free_frames();
    let frame = mm::allocate();
    let taken = frame.is_some() && mm::free_frames() == before - 1;
    drop(frame);
    taken && mm::free_frames() == before
}

pub(super) fn uncertain_dma_memory_is_never_recycled() -> bool {
    let before = mm::free_frames();
    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let reserved = mm::free_frames() == before - 1 && dma.address().is_some();

    dma.retain();
    let inaccessible = dma.address().is_none() && !dma.zero() && !dma.read_at(0, &mut [0u8; 1]);
    drop(dma);

    reserved && inaccessible && mm::free_frames() == before - 1
}

pub(super) fn dma_byte_access_is_exact_and_bounded() -> bool {
    const FILL: u8 = 0x31;
    const TAIL: [u8; 2] = [0xa5, 0x5a];

    let Some(segment) = mm::allocate_contiguous(1, 1) else {
        return false;
    };
    let mut dma = mm::DmaSegment::from(segment);
    let Some(len) = dma.len().and_then(|len| usize::try_from(len).ok()) else {
        return false;
    };
    let Some(tail) = len.checked_sub(TAIL.len()) else {
        return false;
    };

    let exact = dma.fill(FILL) && dma.write_at(tail, &TAIL) && {
        let mut observed = [0; TAIL.len()];
        dma.read_at(tail, &mut observed) && observed == TAIL
    };
    let mut refused_read = [0xcc; 2];
    let refused = !dma.write_at(len - 1, &TAIL)
        && !dma.write_at(usize::MAX, &TAIL)
        && !dma.read_at(len - 1, &mut refused_read)
        && refused_read == [0xcc; 2];
    let unchanged = {
        let mut observed = [0; TAIL.len()];
        dma.read_at(tail, &mut observed) && observed == TAIL
    };
    exact && refused && unchanged
}

pub(super) fn frames_are_exclusive() -> bool {
    let (Some(first), Some(second)) = (mm::allocate(), mm::allocate()) else {
        return false;
    };
    first.address() != second.address()
}

pub(super) fn dirty_memory_can_be_zeroed() -> bool {
    let Some(mut frame) = mm::allocate() else {
        return false;
    };

    let dirtied = frame.fill(0xAA) && frame.is_zero() == Some(false);
    let cleaned = frame.zero() && frame.is_zero() == Some(true);
    dirtied && cleaned
}

pub(super) fn allocation_is_zeroed() -> bool {
    mm::allocate_zeroed().and_then(|frame| frame.is_zero()) == Some(true)
}

// An allocation overlapping the run would disprove exclusive contiguity.
pub(super) fn a_run_is_contiguous_and_aligned() -> bool {
    const FRAMES: usize = 8;
    /// Distinct per frame, so a page from somewhere else reads as the wrong one.
    const MARK: u8 = 0xC7;

    let before = mm::free_frames();
    let Some(mut run) = mm::allocate_contiguous(FRAMES, FRAMES) else {
        return false;
    };
    let aligned = run.address().is_multiple_of(FRAMES as u64 * 4096);
    let sized = run.frames() == FRAMES && mm::free_frames() == before - FRAMES;

    let mut spans = true;
    for frame in 0..FRAMES {
        let at = frame * 4096;
        spans &= run.write_at(at, &[MARK ^ frame as u8]);
    }

    let start = run.address();
    let end = start + (FRAMES as u64 * 4096);
    let elsewhere = mm::allocate_zeroed()
        .map(|frame| {
            let address = frame.address();
            !(start..end).contains(&address)
        })
        .unwrap_or(false);

    let mut intact = true;
    for frame in 0..FRAMES {
        let mut byte = [0u8; 1];
        intact &= run.read_at(frame * 4096, &mut byte) && byte[0] == MARK ^ frame as u8;
    }
    drop(run);

    aligned && sized && spans && elsewhere && intact && mm::free_frames() == before
}

// The class is warmed first, since any first allocation of a class must pay for a refill.
// Accounting cannot see a free-list leak, so LIFO reuse must return the same address;
// `black_box` orders both allocations.
pub(super) fn the_heap_reuses_a_freed_slot() -> bool {
    drop(core::hint::black_box(Box::new(0u32)));

    let before = mm::heap::stats();
    let boxed = core::hint::black_box(Box::new(0x5A5A_5A5Au32));
    let taken = mm::heap::stats().live == before.live + 1;
    drop(boxed);
    let after = mm::heap::stats();
    let balanced = taken && after.live == before.live && after.slab_frames == before.slab_frames;

    let first = core::hint::black_box(Box::new(0u32));
    let address = core::hint::black_box(core::ptr::from_ref(&*first) as usize);
    drop(first);
    let second = core::hint::black_box(Box::new(0u32));
    let reused = core::hint::black_box(core::ptr::from_ref(&*second) as usize) == address;
    drop(second);

    balanced && reused
}

// The vector's buffer outlives all three snapshots, so only the slots under test move between
// them; a slot handed out twice would overwrite what an earlier one was given.
pub(super) fn a_refill_costs_one_frame_and_balances() -> bool {
    /// Enough allocations of one class to span several slab frames.
    const TAKEN: usize = 512;

    let mut held: Vec<Box<[u8; 64]>> = Vec::with_capacity(TAKEN);
    let before = mm::heap::stats();
    for index in 0..TAKEN {
        held.push(Box::new([index as u8; 64]));
    }
    let after_taking = mm::heap::stats();

    let intact = held
        .iter()
        .enumerate()
        .all(|(index, slot)| slot.iter().all(|byte| *byte == index as u8));

    held.clear();
    let after = mm::heap::stats();

    let refilled = after_taking.slab_frames > before.slab_frames;
    let took_every_one = after_taking.live == before.live + TAKEN;
    let balanced = after.live == before.live && after.slab_frames == after_taking.slab_frames;

    refilled && took_every_one && intact && balanced
}

pub(super) fn heap_memory_holds_what_it_is_given() -> bool {
    let mut values: Vec<u32> = (0..1024).collect();
    values
        .iter_mut()
        .for_each(|value| *value = value.wrapping_mul(3));

    values.len() == 1024
        && values
            .iter()
            .enumerate()
            .all(|(i, v)| *v == (i as u32).wrapping_mul(3))
}

pub(super) fn a_large_allocation_takes_and_returns_frames() -> bool {
    const FRAMES: usize = 4;

    let frames_before = mm::free_frames();
    let heap_before = mm::heap::stats();
    let large: Vec<u8> = alloc::vec![0x42; FRAMES * 4096];
    let heap_taken = mm::heap::stats();
    let taken = frames_before.checked_sub(mm::free_frames()) == Some(FRAMES)
        && heap_taken.large_frames == heap_before.large_frames + FRAMES
        && heap_taken.live == heap_before.live + 1;
    let held = large.iter().all(|byte| *byte == 0x42);
    drop(large);

    taken && held && mm::free_frames() == frames_before && mm::heap::stats() == heap_before
}
