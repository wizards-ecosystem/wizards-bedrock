//! The kernel heap.
//!
//! Segregated free lists over slabs carved from single frames; anything too
//! large for a slot takes whole frames. The size-class arithmetic is
//! `slab_layout`, host-tested. What is here needs raw pointers: the free
//! lists live inside the free slots themselves.
//!
//! It allocates out of the direct map rather than mapping pages. That began
//! as a constraint — `activate` made mapping one-way (D-47) — and D-86 removed
//! the constraint without changing the choice: the direct map needs no table
//! walk and no invalidation, which is what a slab refill wants. Slab frames are
//! never returned; [`stats`] reports the numbers that would justify changing
//! that.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use slab_layout::{CLASSES, Placement, SizeClass, place};

use super::{Paddr, Segment, allocate_zeroed, direct_pointer};
use crate::arch::current::interrupt;
use crate::sync::SpinLock;

/// An empty free list. Zero is never a slot address: the direct map is in the
/// higher half, so any real slot is far above it.
const EMPTY: usize = 0;

/// Free lists and the counters that say how the heap is doing.
struct Heap {
    /// Head of each class's free list, or [`EMPTY`].
    free: [usize; CLASSES],
    /// Frames handed to slabs, never returned.
    slab_frames: usize,
    /// Frames currently held by large allocations.
    large_frames: usize,
    /// Live allocations, slots and runs together.
    live: usize,
}

impl Heap {
    const fn new() -> Self {
        Self {
            free: [EMPTY; CLASSES],
            slab_frames: 0,
            large_frames: 0,
            live: 0,
        }
    }

    /// Take a slot of `class`, or null if this class's free list is empty.
    ///
    /// Refilling is the caller's job, done with this lock *dropped* — which is
    /// what keeps `HEAP` a leaf and is why D-47's `HEAP` → `FRAMES` order no
    /// longer exists (D-85).
    fn take_slot(&mut self, class: SizeClass) -> *mut u8 {
        if self.free[class.index()] == EMPTY {
            return ptr::null_mut();
        }

        let slot = self.free[class.index()];
        // SAFETY: `slot` came off this class's free list, so it is a slot of
        // this class inside a slab frame this heap owns and nothing else
        // holds. Its first word is the link, written when it was pushed.
        self.free[class.index()] = unsafe { (slot as *mut usize).read() };
        self.live += 1;
        slot as *mut u8
    }

    /// Give a slot of `class` back.
    ///
    /// # Safety
    ///
    /// `slot` must be an address this heap handed out for `class` and not
    /// since returned.
    unsafe fn give_slot(&mut self, slot: *mut u8, class: SizeClass) {
        // SAFETY: the caller guarantees `slot` is a slot of `class` this heap
        // owns, so its first word is ours to overwrite with the link — the
        // slot's contents are dead the moment it is freed.
        unsafe { (slot as *mut usize).write(self.free[class.index()]) };
        self.free[class.index()] = slot as usize;
        self.live -= 1;
    }

    /// Carve a frame the caller already owns into slots of `class`.
    ///
    /// # Safety
    ///
    /// `base` must address a whole free frame reachable through the direct
    /// map, which nothing else holds and which the heap now owns.
    unsafe fn refill_with(&mut self, class: SizeClass, base: *mut u8) {
        self.slab_frames += 1;

        for slot in 0..class.slots_per_frame() {
            // SAFETY: the caller's contract is that `base` addresses a whole
            // free frame reachable through the direct map, and `offset_of` is
            // below `FRAME_SIZE` by construction, so the result is inside it.
            // That contract is the ground here — `direct_pointer` is
            // arithmetic, and this function never calls it (D-140).
            let address = unsafe { base.add(class.offset_of(slot)) };
            // SAFETY: the frame was just allocated, so no other code holds any
            // slot in it; writing the link is the first use of each.
            unsafe { (address as *mut usize).write(self.free[class.index()]) };
            self.free[class.index()] = address as usize;
        }
    }
}

static HEAP: SpinLock<Heap> = SpinLock::new(Heap::new());

/// What the heap has taken and is holding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    /// Frames given to slabs. Never returned.
    pub slab_frames: usize,
    /// Frames held by allocations too large for a slot.
    pub large_frames: usize,
    /// Live allocations.
    pub live: usize,
}

/// What the heap has taken and is holding, right now.
pub fn stats() -> Stats {
    let heap = HEAP.lock_leaf();
    Stats {
        slab_frames: heap.slab_frames,
        large_frames: heap.large_frames,
        live: heap.live,
    }
}

/// Reclaim the heap's lock on the terminal panic path.
///
/// The standing obligation `005-code-conventions.md` places on every `static`
/// lock. A panic can interrupt an allocation, and a handler that formats a
/// message would otherwise spin on the lock the code it interrupted holds.
pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    HEAP.force_unlock(panic);
}

/// The kernel's allocator.
struct KernelAllocator;

// SAFETY: `alloc` returns either null or the address of a slot or run that
// nothing else holds, aligned to at least the requested alignment — slots
// because a slab frame is frame-aligned and a class's slots sit at multiples
// of the class size, runs because they begin on a frame boundary. `dealloc`
// returns exactly what `alloc` handed out, under the same lock.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // A handler that allocates can be the one that interrupted the heap's
        // own refill, and `003`'s rule that handlers do not allocate was
        // enforced by nobody until there was a count to ask (D-55). Loud,
        // because a silent null here would be blamed on memory.
        assert!(!interrupt::in_trap(), "allocation in trap context");
        match place(layout.size(), layout.align()) {
            Ok(Placement::Slot(class)) => {
                let slot = HEAP.lock_leaf().take_slot(class);
                if !slot.is_null() {
                    return slot;
                }
                // Empty free list. Take a frame with the heap's lock *not*
                // held, then re-check: a preemption between the two may have
                // refilled this class already, which costs a slab frame and
                // nothing else.
                let Some(frame) = allocate_zeroed() else {
                    return ptr::null_mut();
                };
                let Some(base) = direct_pointer(frame.address(), super::FRAME_SIZE) else {
                    return ptr::null_mut();
                };
                // The slab owns the frame from here on; nothing returns one.
                core::mem::forget(frame);
                let mut heap = HEAP.lock_leaf();
                // SAFETY: the frame was just allocated, so nothing else holds
                // any slot in it, and the direct map covers every frame the
                // allocator manages — which is what makes the whole of it
                // reachable. `direct_pointer` returned a pointer; it checked
                // no coverage (D-140).
                unsafe { heap.refill_with(class, base) };
                heap.take_slot(class)
            }
            Ok(Placement::Frames { count, align }) => {
                let Some(run) = super::allocate_contiguous(count, align) else {
                    return ptr::null_mut();
                };
                let Some(address) = direct_pointer(run.address(), run.len()) else {
                    return ptr::null_mut();
                };
                // The run is owned by the caller's allocation now; `dealloc`
                // reconstructs it from the layout.
                core::mem::forget(run);

                let mut heap = HEAP.lock_leaf();
                heap.large_frames += count;
                heap.live += 1;
                address
            }
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // A handler that drops a `Box` touches the heap as much as one that
        // makes it (D-55).
        assert!(!interrupt::in_trap(), "deallocation in trap context");
        match place(layout.size(), layout.align()) {
            // SAFETY: `GlobalAlloc`'s contract is that `pointer` came from
            // `alloc` with this same layout, so it is a live slot of exactly
            // this class.
            Ok(Placement::Slot(class)) => unsafe { HEAP.lock_leaf().give_slot(pointer, class) },
            Ok(Placement::Frames { count, .. }) => {
                let Some(physical) = physical_of(pointer) else {
                    return;
                };
                let mut heap = HEAP.lock_leaf();
                heap.large_frames -= count;
                heap.live -= 1;
                drop(heap);

                // SAFETY: `GlobalAlloc`'s contract is that `pointer` came from
                // `alloc` with this layout, so `count` frames at `physical`
                // are exactly the run allocated for it and not since freed.
                drop(unsafe { Segment::from_raw(physical, count) });
            }
            Err(_) => {}
        }
    }
}

/// The physical address behind a direct-map pointer.
fn physical_of(pointer: *mut u8) -> Option<Paddr> {
    let offset = super::direct_map()?.offset();
    (pointer as u64).checked_sub(offset)
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;
