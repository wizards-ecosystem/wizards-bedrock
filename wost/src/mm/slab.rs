use core::fmt;
use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ops::Deref;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU32, Ordering};

pub use object_pool::{Issued, Refusal, Released};
use object_pool::{SLOTS_PER_PAGE, Slots};

use super::direct_pointer;
use super::pool::{ArenaFrame, FrameKind};

const FRAME_BYTES: usize = frame_space::FRAME_SIZE as usize;

// A whole object as a constant, which `insert_blank` copies into a slot with no stack temporary.
pub trait Blank: Sized {
    const BLANK: Self;
}

// Slot state, borrow counts and objects in one frame; only `&mut self` changes slot state, and a
// hold keeps its slot's object in place by its borrow count.
pub struct ObjectPage<T, const N: usize> {
    frame: ManuallyDrop<ArenaFrame>,
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

    pub fn open(frame: ArenaFrame, floor: u32) -> Self {
        let () = Self::FITS;
        assert!(
            frame.kind() == FrameKind::ArenaObject,
            "an object page opened over a {:?} frame",
            frame.kind()
        );
        let Some(slots) = Slots::<N>::new(floor) else {
            panic!("a page of {N} slots is outside the slot range");
        };
        let page = Self {
            frame: ManuallyDrop::new(frame),
            objects: PhantomData,
        };
        // SAFETY: the frame is this page's alone, offset 0 is page aligned and the slot state fits
        // before the borrow counts, and nothing reads it before this write.
        unsafe { page.slots().write(slots) };
        for borrow in page.borrows() {
            borrow.store(0, Ordering::Relaxed);
        }
        page
    }

    fn base(&self) -> *mut u8 {
        direct_pointer(self.frame.address(), FRAME_BYTES as u64)
            .unwrap_or_else(|| panic!("an object page lost its direct-map address"))
    }

    fn slots(&self) -> *mut Slots<N> {
        self.base().cast()
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
        let borrows = self.base().wrapping_add(Self::BORROWS);
        // SAFETY: the counts lie inside the frame at an aligned offset, every bit pattern is a valid
        // count, and they are only ever reached as atomics.
        unsafe { &*borrows.cast::<[AtomicU32; N]>() }
    }

    fn object(&self, slot: usize) -> *mut T {
        self.base()
            .wrapping_add(Self::OBJECTS + slot * size_of::<T>())
            .cast()
    }

    fn unborrowed(&self, slot: u16, generation: u32) -> Option<usize> {
        let at = usize::from(slot);
        let borrow = self.borrows().get(at)?;
        (self.slot_state().resolves(slot, generation) && borrow.load(Ordering::Acquire) == 0)
            .then_some(at)
    }

    // Ends the page in constant time: objects are forgotten and the frame waits for `dissolve`.
    pub fn surrender(self) {
        let mut page = ManuallyDrop::new(self);
        page.assert_unborrowed();
        // SAFETY: `page` is never dropped, so this take is the frame's only move out.
        let frame = unsafe { ManuallyDrop::take(&mut page.frame) };
        frame.surrender();
    }

    fn assert_unborrowed(&self) {
        let borrowed = self
            .borrows()
            .iter()
            .any(|borrow| borrow.load(Ordering::Acquire) != 0);
        assert!(
            !borrowed,
            "an object page was released while a hold borrows it"
        );
    }

    pub fn free(&self) -> u16 {
        self.slot_state().free()
    }

    pub fn live(&self) -> u16 {
        self.slot_state().live()
    }

    pub fn first_live(&self) -> Option<Issued> {
        self.slot_state().first_live()
    }

    // The least generation a later page at this page's index may start from; `None` once retired.
    pub fn floor(&self) -> Option<u32> {
        self.slot_state().floor_after(0)
    }

    // `make` runs before any slot turns live, and its value is written straight into the slot.
    pub fn insert_with(&mut self, make: impl FnOnce() -> T) -> Option<Issued> {
        if self.free() == 0 {
            return None;
        }
        let value = make();
        let Ok(issued) = self.slot_state_mut().insert() else {
            panic!("an object page with a free slot refused an insert");
        };
        // SAFETY: the slot just turned live from free, so no object or hold occupies it, and it
        // lies inside the frame at `T`'s alignment.
        unsafe { self.object(usize::from(issued.slot)).write(value) };
        Some(issued)
    }

    pub fn insert_blank(&mut self) -> Option<Issued>
    where
        T: Blank,
    {
        let issued = self.slot_state_mut().insert().ok()?;
        let slot = self
            .object(usize::from(issued.slot))
            .cast::<MaybeUninit<T>>();
        // SAFETY: the slot just turned live from free, so no object or hold occupies it, it lies
        // inside the frame at `T`'s alignment, and assigning a `MaybeUninit` drops nothing.
        unsafe { *slot = const { MaybeUninit::new(T::BLANK) } };
        Some(issued)
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
        self.assert_unborrowed();
        // SAFETY: drop runs once, and the field is never read after this take.
        let frame = unsafe { ManuallyDrop::take(&mut self.frame) };
        drop(frame);
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
