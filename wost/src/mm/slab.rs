use core::fmt;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::Deref;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, Ordering};

pub use object_pool::{Issued, Refusal, Released};
use object_pool::{SLOTS_PER_PAGE, Slots};

use super::direct_pointer;
use super::pool::{ArenaFrame, ArenaId, Earmark, FrameKind, PageCount, count_page};

const FRAME_BYTES: usize = frame_space::FRAME_SIZE as usize;

// One arena object frame, counted against its arena so dissolve cannot reclaim it while open.
pub struct PageFrame {
    frame: ManuallyDrop<ArenaFrame>,
}

impl PageFrame {
    pub fn take(earmark: &mut Earmark) -> Option<Self> {
        let frame = earmark.take(FrameKind::ArenaObject)?;
        count_page(frame.arena(), PageCount::Opened);
        Some(Self {
            frame: ManuallyDrop::new(frame),
        })
    }

    pub fn arena(&self) -> ArenaId {
        self.frame.arena()
    }

    fn base(&self) -> *mut u8 {
        direct_pointer(self.frame.address(), FRAME_BYTES as u64)
            .unwrap_or_else(|| panic!("an object page lost its direct-map address"))
    }
}

impl fmt::Debug for PageFrame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PageFrame")
            .field("frame", &*self.frame)
            .finish()
    }
}

impl Drop for PageFrame {
    fn drop(&mut self) {
        let arena = self.frame.arena();
        // SAFETY: drop runs once, and the field is never read after this take.
        let frame = unsafe { ManuallyDrop::take(&mut self.frame) };
        drop(frame);
        count_page(arena, PageCount::Closed);
    }
}

// Slot state, borrow counts and objects in one frame; only `&mut self` changes slot state, and a
// hold keeps its slot's object in place by its borrow count.
pub struct ObjectPage<T, const N: usize> {
    frame: PageFrame,
    objects: PhantomData<T>,
}

impl<T, const N: usize> ObjectPage<T, N> {
    const BORROWS: usize = size_of::<Slots<N>>().next_multiple_of(align_of::<AtomicU32>());
    const OBJECTS: usize =
        (Self::BORROWS + N * size_of::<AtomicU32>()).next_multiple_of(align_of::<T>());
    const FITS: () = assert!(
        N >= 1
            && N <= SLOTS_PER_PAGE as usize
            && align_of::<T>() <= FRAME_BYTES
            && Self::OBJECTS + N * size_of::<T>() <= FRAME_BYTES,
        "the slots and objects of an object page fit one frame"
    );

    pub fn open(frame: PageFrame, floor: u32) -> Self {
        let () = Self::FITS;
        let Some(slots) = Slots::<N>::new(floor) else {
            panic!("a page of {N} slots is outside the slot range");
        };
        let page = Self {
            frame,
            objects: PhantomData,
        };
        // SAFETY: the frame is this page's alone and zeroed, offset 0 is page aligned and the slot
        // state fits before the borrow counts, and nothing reads it before this write.
        unsafe { page.slots().write(slots) };
        page
    }

    fn slots(&self) -> *mut Slots<N> {
        self.frame.base().cast()
    }

    fn slot_state(&self) -> &Slots<N> {
        // SAFETY: `open` wrote the slot state, and it is written only through `&mut self`.
        unsafe { &*self.slots() }
    }

    fn slot_state_mut(&mut self) -> &mut Slots<N> {
        // SAFETY: `open` wrote the slot state, `&mut self` excludes every other reference to it,
        // and holds never reach it.
        unsafe { &mut *self.slots() }
    }

    fn borrows(&self) -> &[AtomicU32; N] {
        let borrows = self.frame.base().wrapping_add(Self::BORROWS);
        // SAFETY: the counts lie inside the frame at an aligned offset, the zeroed frame is a
        // valid array of zero counts, and they are only ever reached as atomics.
        unsafe { &*borrows.cast::<[AtomicU32; N]>() }
    }

    fn object(&self, slot: usize) -> *mut T {
        self.frame
            .base()
            .wrapping_add(Self::OBJECTS + slot * size_of::<T>())
            .cast()
    }

    fn unborrowed(&self, slot: u16, generation: u32) -> Option<usize> {
        let at = usize::from(slot);
        let borrow = self.borrows().get(at)?;
        (self.slot_state().resolves(slot, generation) && borrow.load(Ordering::Acquire) == 0)
            .then_some(at)
    }

    pub fn free(&self) -> u16 {
        self.slot_state().free()
    }

    pub fn live(&self) -> u16 {
        self.slot_state().live()
    }

    // The least generation a later page at this page's index may start from; `None` once retired.
    pub fn floor(&self) -> Option<u32> {
        self.slot_state().floor_after(0)
    }

    pub fn insert(&mut self, value: T) -> Result<Issued, T> {
        let Ok(issued) = self.slot_state_mut().insert() else {
            return Err(value);
        };
        let object = self.object(usize::from(issued.slot));
        // SAFETY: the slot just turned live from free, so no object or hold occupies it, and it
        // lies inside the frame at `T`'s alignment.
        unsafe { object.write(value) };
        Ok(issued)
    }

    pub fn hold(&self, slot: u16, generation: u32) -> Option<Hold<T>>
    where
        T: Sync,
    {
        if !self.slot_state().resolves(slot, generation) {
            return None;
        }
        let at = usize::from(slot);
        let borrow = self.borrows().get(at)?;
        borrow
            .fetch_update(Ordering::Acquire, Ordering::Relaxed, |count| {
                count.checked_add(1)
            })
            .ok()?;
        Some(Hold {
            object: NonNull::new(self.object(at))?,
            borrow: NonNull::from(borrow),
        })
    }

    pub fn get_mut(&mut self, slot: u16, generation: u32) -> Option<&mut T> {
        let at = self.unborrowed(slot, generation)?;
        // SAFETY: the slot is live, `&mut self` stops a new hold, and no hold borrows it.
        Some(unsafe { &mut *self.object(at) })
    }

    pub fn acquire(&mut self, slot: u16, generation: u32) -> Result<(), Refusal> {
        self.slot_state_mut().acquire(slot, generation)
    }

    pub fn release(&mut self, slot: u16, generation: u32) -> Result<Released, Refusal> {
        self.slot_state_mut().release(slot, generation)
    }

    pub fn remove(&mut self, slot: u16, generation: u32) -> Result<T, Removal> {
        let Some(at) = self.unborrowed(slot, generation) else {
            return Err(if self.slot_state().resolves(slot, generation) {
                Removal::Borrowed
            } else {
                Removal::Refused(Refusal::Stale)
            });
        };
        self.slot_state_mut()
            .remove(slot, generation)
            .map_err(Removal::Refused)?;
        // SAFETY: the slot held a live object, its removal ended that life in the slot state, no
        // hold borrows it and `&mut self` stops a new one, so this read moves it out once.
        Ok(unsafe { self.object(at).read() })
    }
}

impl<T, const N: usize> fmt::Debug for ObjectPage<T, N> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectPage")
            .field("frame", &self.frame)
            .field("live", &self.live())
            .finish()
    }
}

// Live objects are forgotten, never dropped: releasing a page zeroes and credits its frame only.
impl<T, const N: usize> Drop for ObjectPage<T, N> {
    fn drop(&mut self) {
        let borrowed = self
            .borrows()
            .iter()
            .any(|borrow| borrow.load(Ordering::Acquire) != 0);
        assert!(
            !borrowed,
            "an object page was released while a hold borrows it"
        );
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Removal {
    Refused(Refusal),
    Borrowed,
}

impl fmt::Display for Removal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused(refusal) => fmt::Display::fmt(refusal, formatter),
            Self::Borrowed => formatter.write_str("a kernel hold still borrows the object"),
        }
    }
}

impl core::error::Error for Removal {}

// A counted borrow of one object; its page refuses to remove the object or close while it lives.
pub struct Hold<T> {
    object: NonNull<T>,
    borrow: NonNull<AtomicU32>,
}

impl<T> Deref for Hold<T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: the borrow count keeps the object initialized and in place, and `hold` admits
        // only `T: Sync`, so shared references on several processors are sound.
        unsafe { self.object.as_ref() }
    }
}

impl<T: fmt::Debug> fmt::Debug for Hold<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&**self, formatter)
    }
}

impl<T> Drop for Hold<T> {
    fn drop(&mut self) {
        // SAFETY: the count lives in the page header, which outlives every hold it counts.
        let borrow = unsafe { self.borrow.as_ref() };
        borrow.fetch_sub(1, Ordering::Release);
    }
}
