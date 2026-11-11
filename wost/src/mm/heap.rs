//! The kernel heap: free lists in single-frame slabs from the direct map, whole frames if large.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

use slab_layout::{CLASSES, Placement, SizeClass, place};

use super::{PhysicalAddress, Segment, allocate_zeroed, direct_pointer};
use crate::arch::current::interrupt;
use crate::sync::SpinLock;

/// An empty free list; zero is never a slot address, since the direct map is in the higher half.
const EMPTY: usize = 0;

/// Free lists and the accounting that protects heap state transitions.
struct Heap {
    /// Head of each class's free list, or [`EMPTY`].
    free: [usize; CLASSES],
    /// Frames handed to slabs, never returned.
    #[cfg(feature = "selftest")]
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
            #[cfg(feature = "selftest")]
            slab_frames: 0,
            large_frames: 0,
            live: 0,
        }
    }

    /// Take a slot of `class`, or null if this class's free list is empty.
    ///
    /// The caller refills with this lock dropped, which is what keeps `HEAP` a leaf.
    fn take_slot(&mut self, class: SizeClass) -> *mut u8 {
        if self.free[class.index()] == EMPTY {
            return ptr::null_mut();
        }
        let Some(live) = self.live.checked_add(1) else {
            return ptr::null_mut();
        };

        let slot = self.free[class.index()];
        // SAFETY: `slot` came off this class's free list, so it is an unheld slot of this class in
        // a slab frame this heap owns, and its first word is the link written when it was pushed.
        self.free[class.index()] = unsafe { core::ptr::read(slot as *const usize) };
        self.live = live;
        slot as *mut u8
    }

    /// Give a slot of `class` back.
    ///
    /// # Safety
    ///
    /// `slot` must be an address this heap handed out for `class` and not since returned.
    #[must_use = "a refused slot return changed no heap state"]
    unsafe fn give_slot(&mut self, slot: *mut u8, class: SizeClass) -> bool {
        let Some(live) = self.live.checked_sub(1) else {
            return false;
        };
        let link = slot.cast::<usize>();
        // SAFETY: the caller guarantees a slot of `class` this heap owns, dead once it is freed, so
        // its first word is ours to overwrite with the link.
        unsafe { core::ptr::write(link, self.free[class.index()]) };
        self.free[class.index()] = slot as usize;
        self.live = live;
        true
    }

    /// Carve a frame the caller already owns into slots of `class`.
    ///
    /// # Safety
    ///
    /// `base` must address a whole free direct-mapped frame nothing else holds, now the heap's.
    #[must_use = "a refused slab refill changed no heap state"]
    unsafe fn refill_with(&mut self, class: SizeClass, base: *mut u8) -> bool {
        #[cfg(feature = "selftest")]
        if !self.record_slab_frame() {
            return false;
        }

        for slot in 0..class.slots_per_frame() {
            let address = base.wrapping_add(class.offset_of(slot));
            let link = address.cast::<usize>();
            // SAFETY: the caller's uniquely owned direct-mapped frame holds `offset_of`'s slot, and
            // writing the link is that slot's first use; forming the pointer proved no coverage.
            unsafe { core::ptr::write(link, self.free[class.index()]) };
            self.free[class.index()] = address as usize;
        }
        true
    }

    #[cfg(feature = "selftest")]
    fn record_slab_frame(&mut self) -> bool {
        let Some(next) = self.slab_frames.checked_add(1) else {
            return false;
        };
        self.slab_frames = next;
        true
    }
}

static HEAP: SpinLock<Heap> = SpinLock::new(Heap::new());

/// What the heap has taken and is holding.
#[cfg(feature = "selftest")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Stats {
    /// Frames given to slabs. Never returned.
    pub slab_frames: usize,
    /// Frames held by allocations too large for a slot.
    pub large_frames: usize,
    /// Live allocations.
    pub live: usize,
}

/// What the heap has taken and is holding, right now.
#[cfg(feature = "selftest")]
pub fn stats() -> Stats {
    let heap = HEAP.lock_leaf();
    Stats {
        slab_frames: heap.slab_frames,
        large_frames: heap.large_frames,
        live: heap.live,
    }
}

/// Reclaim the heap's lock on the panic path, where formatting would otherwise spin on it.
pub(crate) fn force_unlock(panic: &crate::panic::PanicToken) {
    HEAP.force_unlock(panic);
}

struct KernelAllocator;

// SAFETY: `alloc` gives null or an unheld slot or run aligned at least as asked: slots sit at
// class-size multiples in a frame-aligned slab, runs on a frame; `dealloc` returns that, same lock.
unsafe impl GlobalAlloc for KernelAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // A handler allocation can interrupt a refill, so it fails loudly, never as exhaustion.
        assert!(!interrupt::in_trap(), "allocation in trap context");
        match place(layout.size(), layout.align()) {
            Ok(Placement::Slot(class)) => {
                let slot = HEAP.lock_leaf().take_slot(class);
                if !slot.is_null() {
                    return slot;
                }
                // Refill unlocked, then re-check: a preemption may already have refilled it.
                let Some(frame) = allocate_zeroed() else {
                    return ptr::null_mut();
                };
                let Some(base) = direct_pointer(frame.address(), super::FRAME_SIZE) else {
                    return ptr::null_mut();
                };
                let mut heap = HEAP.lock_leaf();
                // SAFETY: the fresh frame has no held slot and the direct map covers every managed
                // frame, so all of it is reachable, a coverage `direct_pointer` never checked.
                let refilled = unsafe { heap.refill_with(class, base) };
                if !refilled {
                    drop(heap);
                    return ptr::null_mut();
                }
                let slot = heap.take_slot(class);
                drop(heap);
                // The free list owns the frame from here on.
                core::mem::forget(frame);
                slot
            }
            Ok(Placement::Frames { count, align }) => {
                let Some(run) = super::allocate_contiguous(count, align) else {
                    return ptr::null_mut();
                };
                let Some(address) = direct_pointer(run.address(), run.len()) else {
                    return ptr::null_mut();
                };
                let mut heap = HEAP.lock_leaf();
                let Some(large_frames) = heap.large_frames.checked_add(count) else {
                    drop(heap);
                    return ptr::null_mut();
                };
                let Some(live) = heap.live.checked_add(1) else {
                    drop(heap);
                    return ptr::null_mut();
                };
                heap.large_frames = large_frames;
                heap.live = live;
                drop(heap);
                // The caller's allocation owns the run now; `dealloc` rebuilds it from the layout.
                core::mem::forget(run);
                address
            }
            Err(_) => core::ptr::null_mut(),
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // A handler that drops a `Box` touches the heap as much as one that makes it.
        assert!(!interrupt::in_trap(), "deallocation in trap context");
        match place(layout.size(), layout.align()) {
            Ok(Placement::Slot(class)) => {
                // SAFETY: `GlobalAlloc` promises `pointer` came from `alloc` with this layout, so
                // it is a live slot of this exact class.
                let returned = unsafe { HEAP.lock_leaf().give_slot(pointer, class) };
                assert!(returned, "heap refused a live slot return");
            }
            Ok(Placement::Frames { count, .. }) => {
                let Some(physical) = physical_of(pointer) else {
                    panic!("heap could not recover a live allocation's physical address");
                };
                let returned = {
                    let mut heap = HEAP.lock_leaf();
                    match (
                        heap.large_frames.checked_sub(count),
                        heap.live.checked_sub(1),
                    ) {
                        (Some(large_frames), Some(live)) => {
                            heap.large_frames = large_frames;
                            heap.live = live;
                            true
                        }
                        _ => false,
                    }
                };
                assert!(returned, "heap refused a live frame return");

                // SAFETY: `GlobalAlloc` promises `pointer` came from `alloc` with this layout, so
                // `count` frames at `physical` are exactly its allocated run, not since freed.
                drop(unsafe { Segment::from_raw(physical, count) });
            }
            Err(reason) => panic!("heap refused a live allocation's layout: {reason}"),
        }
    }
}

/// The physical address behind a direct-map pointer.
fn physical_of(pointer: *mut u8) -> Option<PhysicalAddress> {
    let offset = super::direct_map()?.offset();
    (pointer as u64).checked_sub(offset)
}

#[global_allocator]
static ALLOCATOR: KernelAllocator = KernelAllocator;
