//! The kernel's own page tables (D-45).
//!
//! [`AddressSpace`] builds a four-level table out of frames from
//! [`crate::mm`]; [`activate`] switches the CPU to it. The encoding lives in
//! `x86_paging`, host-tested against the manual (D-30), so what is here is
//! what needs a raw pointer: walking a table through the direct map, and the
//! `mov cr3` itself.
//!
//! Three design points the signatures alone do not give away. [`activate`] is
//! safe because it walks the new tables for its own code and stack before
//! loading them, so a table that would not survive the switch is refused
//! rather than triple-faulting.
//!
//! Mapping into a *live* space is safe because `set` accepts only an identical
//! rewrite of a present entry: nothing the running kernel stands on can be
//! re-pointed, and a translation that was not present was never cached, so a
//! new mapping owes no invalidation (D-86). Unmapping is the operation that
//! owes one, and [`Unmapped`] is how it is paid (D-88).
//!
//! And a mapping carries its evidence (D-87). There is no entry taking a bare
//! physical address: [`AddressSpace::map_owned`] takes frames the caller owns,
//! [`AddressSpace::map_boot`] takes a [`BootRegion`] the memory map covers, and
//! [`AddressSpace::map_device`] refuses anything that is not a device hole.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use stack_layout::{Slot, StackLayout};
use x86_paging::{
    ENTRIES, Entry, EntryError, Flags, LARGE_PAGE_SIZE, Level, MappingRange, MappingRangeError,
    PAGE_SIZE, TablePrivilege,
};

use super::cpu;
use crate::mm::{self, Paddr, Segment};
use crate::mmio::MmioRegion;
use crate::sync::Once;

/// Why a mapping could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MapError {
    /// No frame was free for a table this mapping needed.
    OutOfMemory,
    /// A table this mapping needs is outside the direct map, so it cannot be
    /// reached to be written.
    Unreachable(Paddr),
    /// The address range is already mapped by a larger page.
    Conflict(u64),
    /// The virtual address is not canonical, so no walk can reach it.
    NonCanonical(u64),
    /// The range's start or length is not page-aligned.
    Misaligned(u64),
    /// A device's registers are not four-byte aligned, so an [`MmioRegion`]
    /// over them would make misaligned accesses.
    UnalignedDevice(Paddr),
    /// A physical address did not fit the entry encoding.
    Encoding(EntryError),
    /// The complete mapping range failed arithmetic or architecture checks.
    InvalidRange(MappingRangeError),
    /// A device range was empty or its page-rounded extent overflowed.
    InvalidDeviceRange(Paddr),
    /// The range overlaps RAM or firmware storage rather than a device hole.
    NotDeviceMemory(Paddr),
    /// Another mapping in this address space already owns physical bytes.
    DeviceOverlap(Paddr),
    /// The bounded device-ownership table is full.
    TooManyDevices,
    /// The boot handoff never described this physical range, so there is no
    /// evidence it is memory rather than a guess (D-87).
    Undescribed(Paddr),
    /// Nothing is mapped at this address, so there is nothing to take away.
    NotMapped(u64),
    /// The mapping here is not one this space owns the frame under, so
    /// unmapping it would hand the allocator memory that was never its (D-88).
    NotOwned(u64),
    /// A large page covers this address. Splitting one is not offered: the
    /// direct map is deliberately unsplittable (D-45).
    Unsplittable(u64),
    /// The range's frames are not one ascending run, so they were not mapped
    /// by a single `map_owned` and cannot be returned as one segment.
    Discontiguous(u64),
}

impl From<EntryError> for MapError {
    fn from(error: EntryError) -> Self {
        Self::Encoding(error)
    }
}

impl From<MappingRangeError> for MapError {
    fn from(error: MappingRangeError) -> Self {
        Self::InvalidRange(error)
    }
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::OutOfMemory => f.write_str("no free frame for a page table"),
            Self::Unreachable(address) => {
                write!(f, "page table at {address:#x} is outside the direct map")
            }
            Self::Conflict(address) => write!(f, "{address:#x} is already mapped by a larger page"),
            Self::NonCanonical(address) => write!(f, "{address:#x} is not a canonical address"),
            Self::Misaligned(address) => write!(f, "{address:#x} is not page-aligned"),
            Self::UnalignedDevice(address) => {
                write!(
                    f,
                    "device registers at {address:#x} are not four-byte aligned"
                )
            }
            Self::Encoding(error) => write!(f, "{error}"),
            Self::InvalidRange(error) => write!(f, "{error}"),
            Self::InvalidDeviceRange(address) => {
                write!(f, "device range at {address:#x} is empty or overflows")
            }
            Self::Undescribed(address) => {
                write!(f, "{address:#x} is not described by the memory map")
            }
            Self::NotMapped(address) => write!(f, "nothing is mapped at {address:#x}"),
            Self::NotOwned(address) => {
                write!(f, "the mapping at {address:#x} does not own its frame")
            }
            Self::Unsplittable(address) => {
                write!(f, "{address:#x} is inside a large page, which is not split")
            }
            Self::Discontiguous(address) => {
                write!(f, "the frames behind {address:#x} are not one run")
            }
            Self::NotDeviceMemory(address) => {
                write!(f, "device range at {address:#x} overlaps ordinary memory")
            }
            Self::DeviceOverlap(address) => {
                write!(f, "device range at {address:#x} is already owned")
            }
            Self::TooManyDevices => f.write_str("device ownership table is full"),
        }
    }
}

impl core::error::Error for MapError {}

/// Why a table was refused at the moment of the switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActivateError {
    /// A second caller tried to switch. The kernel's address space is
    /// installed once.
    AlreadyActive,
    /// The new tables do not map the code that would execute the switch.
    CodeUnmapped(u64),
    /// The new tables map the executing code without execute permission.
    CodeNotExecutable(u64),
    /// The new tables do not map the stack the switch is running on.
    StackUnmapped(u64),
    /// The new tables map the stack without write permission.
    StackReadOnly(u64),
    /// A live virtual address was redirected to different physical memory.
    MappingChanged(u64),
    /// A kernel-image page is missing or has the wrong protection class.
    ImageChanged(u64),
    /// The new tables do not give this address of the direct map the writable,
    /// never-executable mapping every frame access assumes.
    DirectMapGap(u64),
}

impl fmt::Display for ActivateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::AlreadyActive => f.write_str("an address space is already installed"),
            Self::CodeUnmapped(address) => {
                write!(f, "the switch's own code at {address:#x} is unmapped")
            }
            Self::CodeNotExecutable(address) => {
                write!(f, "the switch's own code at {address:#x} is not executable")
            }
            Self::StackUnmapped(address) => write!(f, "the stack at {address:#x} is unmapped"),
            Self::StackReadOnly(address) => write!(f, "the stack at {address:#x} is read-only"),
            Self::MappingChanged(address) => {
                write!(f, "the mapping at {address:#x} changes physical memory")
            }
            Self::ImageChanged(address) => {
                write!(f, "the kernel image mapping at {address:#x} is wrong")
            }
            Self::DirectMapGap(address) => {
                write!(f, "the direct map at {address:#x} is missing or wrong")
            }
        }
    }
}

impl core::error::Error for ActivateError {}

/// A four-level page table under construction.
///
/// Every table in it is a frame permanently owned by the structure: they are
/// deliberately leaked from the frame allocator rather than tracked. `unmap`
/// clears leaf entries and leaves the tables above them (D-88), because
/// reclaiming one means proving no sibling entry survives — and a table freed
/// underneath a live `CR3` is the one bug this design must not permit.
#[derive(Debug)]
pub struct AddressSpace {
    root: Paddr,
    /// Next free address in the device window.
    device_next: u64,
    /// Physical ranges already promised to device mappings in this space.
    devices: [Option<DeviceExtent>; MAX_DEVICE_MAPPINGS],
}

impl AddressSpace {
    /// An address space mapping nothing.
    ///
    /// # Errors
    ///
    /// [`MapError::OutOfMemory`] if no frame is free for the top-level table.
    pub fn new() -> Result<Self, MapError> {
        let root = take_table()?;
        Ok(Self {
            root,
            device_next: DEVICE_WINDOW_START,
            devices: [None; MAX_DEVICE_MAPPINGS],
        })
    }

    /// The physical address of the top-level table — what `CR3` would hold.
    pub const fn root(&self) -> Paddr {
        self.root
    }

    /// Map `len` bytes at `virtual_base` onto physical memory at
    /// `physical_base`, with `flags`.
    ///
    /// Large pages wherever the addresses and remaining length permit, and
    /// `PRESENT` is added; no other flag is.
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless both bases are page-aligned,
    /// [`MapError::NonCanonical`] if the virtual range leaves canonical space,
    /// [`MapError::Conflict`] if a larger page already covers part of the
    /// range, and [`MapError::OutOfMemory`] if a table cannot be allocated.
    fn map_range(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        flags: Flags,
        grain: Grain,
    ) -> Result<(), MapError> {
        let range = MappingRange::new(virtual_base, physical_base, len)?;
        let flags = flags.union(Flags::PRESENT);

        let mut offset = 0;
        while offset < range.len() {
            let (virtual_address, physical, level) = Self::step(&range, offset, grain);
            if let Err(error) = self.set(virtual_address, physical, flags, level) {
                // Everything written so far is taken back, so a refused
                // mapping leaves the space exactly as it was found (D-94).
                // Without this, a caller that owns the frames — which is every
                // `map_owned` caller — frees a run some entries still map, and
                // the allocator hands out memory this space can still write.
                self.undo(&range, offset, grain);
                return Err(error);
            }
            offset += level.span();
        }
        Ok(())
    }

    /// Where the mapping of `range` at `offset` goes, and at what size.
    ///
    /// Split out so [`undo`](Self::undo) retraces exactly the same steps: a
    /// rollback that disagreed with the walk about where a large page began
    /// would clear the wrong entry.
    fn step(range: &MappingRange, offset: u64, grain: Grain) -> (u64, Paddr, Level) {
        let virtual_address = range.virtual_base() + offset;
        let physical = range.physical_base() + offset;
        let large = grain == Grain::Largest
            && range.len() - offset >= LARGE_PAGE_SIZE
            && virtual_address.is_multiple_of(LARGE_PAGE_SIZE)
            && physical.is_multiple_of(LARGE_PAGE_SIZE);
        (
            virtual_address,
            physical,
            if large { Level::Pd } else { Level::Pt },
        )
    }

    /// Clear the first `written` bytes of `range`, undoing a partial mapping.
    ///
    /// One narrow case is *not* undone cleanly: `set` permits an idempotent
    /// rewrite of an entry that already held exactly this mapping, so a range
    /// overlapping an identical pre-existing one walks through it, and clearing
    /// that entry here removes something the call did not create. Only the
    /// direct map's outward rounding can produce identical overlaps, and a
    /// failure there aborts the whole space — but the day something recovers
    /// from a `map_range` error, this is the case to fix (D-95).
    ///
    /// Tables allocated on the way are **not** freed. They are zeroed frames
    /// with no entries pointing anywhere, so the next mapping through them is
    /// correct; reclaiming them would mean proving no sibling entry survives,
    /// which is work for the day something unmaps enough to care.
    fn undo(&mut self, range: &MappingRange, written: u64, grain: Grain) {
        let mut offset = 0;
        while offset < written {
            let (virtual_address, _, level) = Self::step(range, offset, grain);
            if let Ok(slot) = self.slot_at(virtual_address, level) {
                // SAFETY: `slot_at` returns a pointer into a table frame this
                // space owns, reachable through the direct map, at the level
                // the forward walk wrote.
                unsafe { slot.write_volatile(Entry::ABSENT) };
                // And the CPU is told, because these entries were live: this
                // runs against the *active* space, and `map_owned` gives the
                // frames straight back to the allocator afterwards. Leaving a
                // cached translation to them is the hazard `Unmapped` exists
                // to prevent, arriving by the other door (D-95).
                if is_active() && active_root() == self.root {
                    cpu::invalidate_page(virtual_address);
                }
            }
            offset += level.span();
        }
    }

    /// The existing entry slot for `virtual_address` at `target`, allocating
    /// nothing.
    ///
    /// [`MapError::NotMapped`] where the walk runs out of tables, which during
    /// a rollback means the forward walk never got that far.
    fn slot_at(&self, virtual_address: u64, target: Level) -> Result<*mut Entry, MapError> {
        let mut table = self.root;
        for level in Level::WALK {
            let slot = entry_pointer(table, level.index(virtual_address))?;
            if level == target {
                return Ok(slot);
            }
            // SAFETY: `entry_pointer` confirmed the slot is inside a table
            // frame this space owns and reachable through the direct map.
            let entry = unsafe { slot.read_volatile() };
            if !entry.is_present() || entry.is_huge() {
                return Err(MapError::NotMapped(virtual_address));
            }
            table = entry.address();
        }
        Err(MapError::NotMapped(virtual_address))
    }

    /// Map memory the boot handoff described, at `virtual_base`.
    ///
    /// The evidence is the [`BootRegion`], which exists only for a range the
    /// bootloader's memory map covers. This is the entry the direct map and
    /// the kernel image use, and the reason there is no longer a public `map`
    /// taking a bare physical address (D-87): a bound stated in prose is not a
    /// bound, and this one is the shape D-64 gave physical reads.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned).
    pub fn map_boot(
        &mut self,
        virtual_base: u64,
        region: BootRegion,
        flags: Flags,
    ) -> Result<(), MapError> {
        self.map_range(virtual_base, region.base, region.len, flags, Grain::Largest)
    }

    /// Map a run of frames this caller owns, at `virtual_base`.
    ///
    /// Ownership moves into the address space: the frames are not freed while
    /// the mapping stands, which is D-44's type discipline applied to the
    /// mapping API and the discharge of D-76's first authority debt.
    ///
    /// On failure nothing was mapped — the whole range is preflighted before
    /// any entry is written (D-77) — so `run` is dropped here and its frames
    /// go back to the allocator, which is what the caller wants.
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless `virtual_base` is page-aligned,
    /// [`MapError::NonCanonical`] if the range leaves canonical space,
    /// [`MapError::Conflict`] if any page of it is already mapped to something
    /// else, and [`MapError::OutOfMemory`] if a table cannot be allocated.
    pub fn map_owned(
        &mut self,
        virtual_base: u64,
        run: Segment,
        flags: Flags,
    ) -> Result<(), MapError> {
        // The owned bit is what `unmap` reads to know these frames are the
        // allocator's to have back (D-88). It is written in the entry rather
        // than tracked beside it because the entry is the one thing every path
        // to an unmap can see.
        // Four-kilobyte pages only, whatever the run's size and alignment.
        // A large page carrying `OWNED` would be a mapping `unmap` must refuse
        // for ever — it does not split (D-45) — so the frames could never come
        // back. Better to spend a few more entries than to make a leak
        // unreachable (D-94).
        self.map_range(
            virtual_base,
            run.address(),
            run.len(),
            flags.union(Flags::OWNED),
            Grain::Pages,
        )?;
        // The run belongs to the mapping now; `unmap` is what gives it back.
        core::mem::forget(run);
        Ok(())
    }

    /// Map one guarded stack, and hand back the evidence that it is one.
    ///
    /// Allocates the frames, zeroes them, maps them writable and never
    /// executable at `slot`'s stack range, and then **verifies the guard**: the
    /// pages below the stack must be absent in these tables. A stack whose
    /// guard is mapped is not a guarded stack, and the caller has no way to
    /// tell by looking at the return value, so it is checked here.
    ///
    /// The returned [`MappedStack`] is what makes
    /// [`task::continue_on_stack`](super::task::continue_on_stack) safe to
    /// expose: it is the proof that the region is mapped, writable, aligned
    /// and guarded, and it cannot be made any other way.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned), plus [`MapError::Conflict`] naming
    /// the first guard page that turned out to be mapped.
    pub fn map_stack(&mut self, slot: &Slot) -> Result<MappedStack, MapError> {
        let pages = slot.stack_len().div_ceil(PAGE_SIZE) as usize;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        if !run.zero() {
            return Err(MapError::Unreachable(run.address()));
        }
        self.map_owned(
            slot.stack_base(),
            run,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )?;

        let mut guard = slot.guard_base();
        while guard < slot.stack_base() {
            if self.translate_here(guard).is_some() {
                return Err(MapError::Conflict(guard));
            }
            guard = guard
                .checked_add(PAGE_SIZE)
                .ok_or(MapError::Conflict(guard))?;
        }

        Ok(MappedStack {
            top: slot.stack_top(),
            root: self.root,
        })
    }

    /// Translate through this space's tables, active or not.
    fn translate_here(&self, virtual_address: u64) -> Option<(Paddr, Flags)> {
        translate_in(self.root, virtual_address)
    }

    /// Take a mapping away, and hand back what it owned.
    ///
    /// The range must be exactly what one [`map_owned`](Self::map_owned) put
    /// there: every page present, four-kilobyte, owned, and backed by one
    /// ascending run of frames.
    ///
    /// The frames are **not** free when this returns. They belong to the
    /// [`Unmapped`] until its invalidation has run, because a CPU may still
    /// hold a cached translation to them (D-70 rule 9, D-88).
    ///
    /// # Errors
    ///
    /// [`MapError::Misaligned`] unless base and length are page-aligned,
    /// [`MapError::NotMapped`] for a hole, [`MapError::Unsplittable`] for a
    /// large page, [`MapError::NotOwned`] for a mapping whose frame is not the
    /// allocator's, and [`MapError::Discontiguous`] if the frames are not one
    /// run.
    pub fn unmap(&mut self, virtual_base: u64, len: u64) -> Result<Unmapped, MapError> {
        if !virtual_base.is_multiple_of(PAGE_SIZE) || len == 0 || !len.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virtual_base));
        }
        // Both ends, because a walk masks bits 48 and above: without this, the
        // low-half alias of a higher-half address reaches the same tables, and
        // the `invlpg` that follows would name the non-canonical form — which
        // faults on nothing and invalidates nothing, leaving a live TLB entry
        // for frames this call is about to hand back (D-94).
        let end = virtual_base
            .checked_add(len)
            .ok_or(MapError::NonCanonical(virtual_base))?;
        if !x86_paging::is_canonical(virtual_base) || !x86_paging::is_canonical(end - 1) {
            return Err(MapError::NonCanonical(virtual_base));
        }
        let pages = len / PAGE_SIZE;

        // Preflight the whole range before clearing anything, so a refusal
        // leaves the space exactly as it was — the rule D-77 gave `map`.
        let mut first = None;
        for page in 0..pages {
            let address = virtual_base
                .checked_add(page * PAGE_SIZE)
                .ok_or(MapError::NonCanonical(virtual_base))?;
            let slot = self.leaf(address)?;
            // SAFETY: `leaf` returns a pointer into a table frame this space
            // owns, reachable through the direct map.
            let entry = unsafe { slot.read_volatile() };
            if !entry.flags().contains(Flags::OWNED) {
                return Err(MapError::NotOwned(address));
            }
            match first {
                None => first = Some(entry.address()),
                Some(base) if entry.address() == base + page * PAGE_SIZE => {}
                Some(_) => return Err(MapError::Discontiguous(address)),
            }
        }
        let base = first.ok_or(MapError::NotMapped(virtual_base))?;

        for page in 0..pages {
            let address = virtual_base + page * PAGE_SIZE;
            let slot = self.leaf(address)?;
            // SAFETY: as above, and the preflight confirmed this slot holds a
            // present, owned, four-kilobyte mapping.
            unsafe { slot.write_volatile(Entry::ABSENT) };
        }

        Ok(Unmapped {
            base: virtual_base,
            pages,
            // SAFETY: the preflight proved these `pages` frames from `base`
            // are one ascending run that a `map_owned` moved into this space
            // and that nothing else holds, and the entries naming them have
            // just been cleared.
            frames: Some(unsafe { Segment::from_raw(base, pages as usize) }),
        })
    }

    /// The four-kilobyte leaf entry for `address`, or why there is not one.
    fn leaf(&self, address: u64) -> Result<*mut Entry, MapError> {
        let mut table = self.root;
        for level in Level::WALK {
            let slot = entry_pointer(table, level.index(address))?;
            // SAFETY: `entry_pointer` confirmed the slot is inside a table
            // frame this space owns and reachable through the direct map.
            let entry = unsafe { slot.read_volatile() };
            if level == Level::Pt {
                if !entry.is_present() {
                    return Err(MapError::NotMapped(address));
                }
                return Ok(slot);
            }
            if !entry.is_present() {
                return Err(MapError::NotMapped(address));
            }
            if entry.is_huge() {
                return Err(MapError::Unsplittable(address));
            }
            table = entry.address();
        }
        Err(MapError::NotMapped(address))
    }

    /// Map a device's registers, and hand back the claim on them.
    ///
    /// Uncacheable and never executable, at an address in the device window
    /// rather than the physical one. `len` is rounded up to whole pages.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned), plus [`MapError::OutOfMemory`] if the device
    /// window is exhausted.
    pub fn map_device(&mut self, physical: Paddr, len: u64) -> Result<DeviceMapping, MapError> {
        self.map_device_with(physical, len, Flags::NO_CACHE)
    }

    /// Map a device window with `memory_type` chosen by the caller.
    fn map_device_with(
        &mut self,
        physical: Paddr,
        len: u64,
        memory_type: Flags,
    ) -> Result<DeviceMapping, MapError> {
        // The window keeps the device's offset from a page boundary, so the
        // region's base inherits its low bits — and an `MmioRegion` makes
        // thirty-two-bit accesses. Refused here, where it can be reported.
        if !physical.is_multiple_of(size_of::<u32>() as u64) {
            return Err(MapError::UnalignedDevice(physical));
        }
        let len_usize = usize::try_from(len).map_err(|_| MapError::InvalidDeviceRange(physical))?;
        let physical_end = physical
            .checked_add(len)
            .filter(|_| len != 0)
            .ok_or(MapError::InvalidDeviceRange(physical))?;
        if !mm::device_range_is_eligible(physical, physical_end) {
            return Err(MapError::NotDeviceMemory(physical));
        }
        if self
            .devices
            .iter()
            .flatten()
            .any(|extent| physical < extent.end && extent.start < physical_end)
        {
            return Err(MapError::DeviceOverlap(physical));
        }
        let device_slot = self
            .devices
            .iter()
            .position(Option::is_none)
            .ok_or(MapError::TooManyDevices)?;
        let offset = physical % PAGE_SIZE;
        let base = physical - offset;
        let covered = len
            .checked_add(offset)
            .ok_or(MapError::InvalidDeviceRange(physical))?;
        let page_count = covered.div_ceil(PAGE_SIZE);
        let pages = page_count
            .checked_mul(PAGE_SIZE)
            .ok_or(MapError::InvalidDeviceRange(physical))?;

        let virtual_base = self
            .device_next
            .checked_add(pages)
            .filter(|end| *end <= DEVICE_WINDOW_END)
            .map(|_| self.device_next)
            .ok_or(MapError::OutOfMemory)?;

        self.map_range(
            virtual_base,
            base,
            pages,
            Flags::WRITABLE.union(Flags::NO_EXECUTE).union(memory_type),
            Grain::Pages,
        )?;
        self.device_next = virtual_base
            .checked_add(pages)
            .ok_or(MapError::InvalidDeviceRange(physical))?;
        self.devices[device_slot] = Some(DeviceExtent {
            start: physical,
            end: physical_end,
        });

        Ok(DeviceMapping {
            address: virtual_base + offset,
            physical,
            len: len_usize,
            root: self.root,
        })
    }

    /// Map every stack of `layout`, leaving every guard gap unmapped.
    ///
    /// Writable and never executable, one physically contiguous run per slot,
    /// zeroed before it is mapped. The guards are guards precisely because
    /// this function never touches them: an overrun lands on an absent
    /// mapping, which faults, rather than on a neighbour's stack, which
    /// would not.
    ///
    /// # Errors
    ///
    /// As [`map_owned`](Self::map_owned), plus [`MapError::OutOfMemory`] if any slot's
    /// frames cannot be allocated. Frames already committed stay committed —
    /// the space that failed is abandoned whole, exactly as a failed
    /// [`activate`] abandons it.
    pub fn map_stacks(&mut self, layout: &StackLayout) -> Result<StackArena, MapError> {
        for index in 0..layout.slots() {
            let slot = layout.slot(index).ok_or(MapError::OutOfMemory)?;
            let frames = slot.stack_len().div_ceil(PAGE_SIZE) as usize;
            let mut run = mm::allocate_contiguous(frames, 1).ok_or(MapError::OutOfMemory)?;
            if !run.zero() {
                return Err(MapError::Unreachable(run.address()));
            }
            self.map_owned(
                slot.stack_base(),
                run,
                Flags::WRITABLE.union(Flags::NO_EXECUTE),
            )?;
        }
        Ok(StackArena {
            layout: *layout,
            root: self.root,
        })
    }

    /// Install one entry at `target`, allocating the tables above it.
    fn set(
        &mut self,
        virtual_address: u64,
        physical: Paddr,
        flags: Flags,
        target: Level,
    ) -> Result<(), MapError> {
        let mut table = self.root;

        for level in Level::WALK {
            let slot = entry_pointer(table, level.index(virtual_address))?;

            if level == target {
                let entry = Entry::page(target, physical, flags)?;
                // SAFETY: `entry_pointer` confirmed the slot is inside a table
                // frame this space owns and reachable through the direct map.
                let current = unsafe { slot.read_volatile() };
                // Only an identical rewrite is allowed: anything else would
                // re-point a mapping some handle is evidence of.
                if current.is_present() && current != entry {
                    return Err(MapError::Conflict(virtual_address));
                }
                // SAFETY: as above. Nothing else writes these tables: `map`
                // takes `&mut self`, and the check above means a live mapping
                // is never re-pointed, only an absent one filled in.
                unsafe { slot.write_volatile(entry) };
                return Ok(());
            }

            // SAFETY: as above — the slot is inside a table frame this space
            // owns and reachable through the direct map.
            let current = unsafe { slot.read_volatile() };
            table = if current.is_present() {
                if current.is_huge() {
                    return Err(MapError::Conflict(virtual_address));
                }
                if flags.contains(Flags::USER) && !current.flags().contains(Flags::USER) {
                    let entry = Entry::table(current.address(), TablePrivilege::User)?;
                    // SAFETY: this space is still under exclusive construction.
                    unsafe { slot.write_volatile(entry) };
                }
                current.address()
            } else {
                let frame = take_table()?;
                // SAFETY: as above.
                let privilege = if flags.contains(Flags::USER) {
                    TablePrivilege::User
                } else {
                    TablePrivilege::Supervisor
                };
                // SAFETY: as above; this slot is in the exclusively built table.
                unsafe { slot.write_volatile(Entry::table(frame, privilege)?) };
                frame
            };
        }

        // Unreachable: `target` is always `Pt` or `Pd`, both of which are in
        // `Level::WALK`, so the loop returns before it can end.
        Err(MapError::Conflict(virtual_address))
    }
}

/// A zeroed frame, leaked, to serve as a page table.
///
/// Leaked deliberately: the frame belongs to the page tables for the life of
/// the kernel, and the allocator's records say so. Nothing unmaps, so nothing
/// ever gives one back.
fn take_table() -> Result<Paddr, MapError> {
    let frame = mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?;
    let address = frame.address();
    core::mem::forget(frame);
    Ok(address)
}

/// A pointer to one entry of the table at `table`.
fn entry_pointer(table: Paddr, index: usize) -> Result<*mut Entry, MapError> {
    debug_assert!(index < ENTRIES, "a level's index is always below 512");
    let base = mm::direct_pointer(table, PAGE_SIZE).ok_or(MapError::Unreachable(table))?;
    // SAFETY: `direct_pointer` confirmed the direct map covers all `PAGE_SIZE`
    // bytes of the table, and `index` is below the 512 entries one holds, so
    // the result is inside that mapping.
    Ok(unsafe { base.cast::<Entry>().add(index) })
}

static ACTIVE: Once = Once::new();

/// The root of the installed space, or zero before [`activate`] succeeds.
static ACTIVE_ROOT: AtomicU64 = AtomicU64::new(0);

/// Switch this CPU to `space`, keeping it for the life of the kernel.
///
/// Safe because it walks the new tables first, for this function's own code
/// and its stack. A table that would not survive the switch is refused with
/// the old one still loaded, so the failure is a `Result` and not a reset.
///
/// # Errors
///
/// [`ActivateError`] naming which address the new tables would have lost, or
/// [`ActivateError::AlreadyActive`] on a second call.
pub fn activate(space: &mut AddressSpace) -> Result<(), ActivateError> {
    /// Headroom below the stack pointer that must also be mapped, so the
    /// first push after the switch has somewhere to go.
    const STACK_HEADROOM: u64 = PAGE_SIZE;

    // Asked before the walks, so a second caller is told what is actually
    // wrong with its request rather than which mapping its empty table lacks.
    // The claim below is still what makes the decision, since only one caller
    // can win that swap.
    if ACTIVE.is_claimed() {
        return Err(ActivateError::AlreadyActive);
    }

    let current_root = cpu::page_table_base();
    let code = activate as *const () as u64;
    match translate_in(space.root, code) {
        None => return Err(ActivateError::CodeUnmapped(code)),
        Some((_, flags)) if flags.contains(Flags::NO_EXECUTE) => {
            return Err(ActivateError::CodeNotExecutable(code));
        }
        Some((physical, _)) => {
            if translate_in(current_root, code).map(|current| current.0) != Some(physical) {
                return Err(ActivateError::MappingChanged(code));
            }
        }
    }

    let stack = cpu::stack_pointer();
    for probe in [stack, stack.saturating_sub(STACK_HEADROOM)] {
        match translate_in(space.root, probe) {
            None => return Err(ActivateError::StackUnmapped(probe)),
            Some((_, flags)) if !flags.contains(Flags::WRITABLE) => {
                return Err(ActivateError::StackReadOnly(probe));
            }
            Some((physical, _)) => {
                if translate_in(current_root, probe).map(|current| current.0) != Some(physical) {
                    return Err(ActivateError::MappingChanged(probe));
                }
            }
        }
    }

    if let Some(gap) = kernel_image_gap(space.root, current_root) {
        return Err(ActivateError::ImageChanged(gap));
    }

    if let Some(gap) = direct_map_gap(space.root) {
        return Err(ActivateError::DirectMapGap(gap));
    }

    if !ACTIVE.claim() {
        return Err(ActivateError::AlreadyActive);
    }

    // SAFETY: the walks above confirmed that these tables map this function's
    // own code as executable and its stack as writable, at the addresses both
    // already occupy — which is the whole of `set_page_table_base`'s contract
    // for the code that survives the switch. The space is moved into this call
    // and never dropped, so the tables outlive the load.
    unsafe { cpu::set_page_table_base(space.root) };
    ACTIVE_ROOT.store(space.root, Ordering::Release);

    // The caller keeps the space, and may map into it from here on. What
    // makes that sound is not this signature but `set`: a present entry
    // accepts only an identical rewrite, so no mapping the running kernel
    // stands on can be re-pointed, and a mapping that was not present cannot
    // be one the CPU has cached (D-86).
    Ok(())
}

/// First kernel-image page redirected or assigned the wrong protection class.
fn kernel_image_gap(candidate: Paddr, current: Paddr) -> Option<u64> {
    let sections = crate::boot::sections();
    for (section, writable, executable) in [
        (sections.text, false, true),
        (sections.rodata, false, false),
        (sections.data, true, false),
    ] {
        let Some(end) = section.start.checked_add(section.mapped_len()) else {
            return Some(section.start);
        };
        let mut address = section.start;
        while address < end {
            let Some((current_physical, _)) = translate_in(current, address) else {
                return Some(address);
            };
            let Some((candidate_physical, flags)) = translate_in(candidate, address) else {
                return Some(address);
            };
            if candidate_physical != current_physical
                || flags.is_write_execute()
                || flags.contains(Flags::WRITABLE) != writable
                || !flags.contains(Flags::NO_EXECUTE) != executable
            {
                return Some(address);
            }
            let Some(next) = address.checked_add(PAGE_SIZE) else {
                return Some(address);
            };
            address = next;
        }
    }
    None
}

/// The first direct-map address `root` gets wrong, if any: every frame access
/// after the switch is raw arithmetic against this mapping, so a gap must be
/// refused while the refusal can still be a `Result`.
fn direct_map_gap(root: Paddr) -> Option<u64> {
    let offset = mm::direct_map()?.offset();
    mm::with_memory_map(|map| {
        for region in map
            .regions()
            .filter(|region| region.kind.is_direct_mapped())
        {
            let base = region.base & !(PAGE_SIZE - 1);
            let end = region
                .base
                .saturating_add(region.len)
                .next_multiple_of(PAGE_SIZE);
            let mut physical = base;
            while physical < end {
                let Some(virtual_address) = offset.checked_add(physical) else {
                    return Some(physical);
                };
                match walk(root, virtual_address) {
                    Some((mapped, flags, span))
                        if mapped == physical
                            && if region.kind == crate::boot::MemoryKind::KernelAndModules {
                                !flags.is_write_execute()
                            } else {
                                flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE)
                            } =>
                    {
                        physical += span - (virtual_address & (span - 1));
                    }
                    _ => return Some(virtual_address),
                }
            }
        }
        None
    })?
}

/// Whether the kernel is running on tables it built itself.
pub fn is_active() -> bool {
    ACTIVE.is_claimed()
}

/// The root of the space this kernel installed, or zero if it installed none.
///
/// Compared against [`cpu::page_table_base`] by the boot-time check that the
/// loaded table really is ours — the two agreeing is the only evidence that
/// the switch took effect rather than merely being attempted.
pub fn active_root() -> Paddr {
    ACTIVE_ROOT.load(Ordering::Acquire)
}

/// What the table the CPU is walking maps `virtual_address` to.
///
/// Reads `CR3` rather than any remembered root, so it reports what the machine
/// is actually using — which is the whole point of a read-back check.
pub fn translate(virtual_address: u64) -> Option<(Paddr, Flags)> {
    translate_in(cpu::page_table_base(), virtual_address)
}

/// Walk `root` for `virtual_address`, returning the frame and the leaf's flags.
fn translate_in(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags)> {
    walk(root, virtual_address).map(|(physical, flags, _)| (physical, flags))
}

/// As [`translate_in`], but also naming the leaf's span, so a sweep can step
/// by whole entries.
fn walk(root: Paddr, virtual_address: u64) -> Option<(Paddr, Flags, u64)> {
    if !x86_paging::is_canonical(virtual_address) {
        return None;
    }

    // The permission bits a walk arrives with, narrowed at every level. The
    // architecture computes the effective permission of a page from the whole
    // path: `USER` and `WRITABLE` are the *conjunction* down the walk, and
    // `NO_EXECUTE` is the *disjunction*. Returning the leaf's flags alone
    // states a stronger permission than the CPU will grant, and four callers
    // read this as if it were the effective one (D-103).
    /// The permissions a walk narrows rather than reads off the leaf.
    const NARROWED: Flags = Flags::USER.union(Flags::WRITABLE);

    let mut granted = NARROWED;
    let mut forbidden = false;

    let mut table = root;
    for level in Level::WALK {
        let slot = entry_pointer(table, level.index(virtual_address)).ok()?;
        // SAFETY: `entry_pointer` confirmed the slot is inside a table
        // reachable through the direct map. An entry is eight bytes on an
        // eight-byte boundary, so a volatile read of one yields some value
        // that was written there and never a tear — which is what this needs,
        // because live tables *are* written now (D-86, D-88) and the older
        // justification, that nothing writes them after activation, is gone.
        let entry = unsafe { slot.read_volatile() };
        if !entry.is_present() {
            return None;
        }

        // The huge bit means something else entirely in a leaf table, so the
        // level is checked first.
        granted = granted.intersection(entry.flags());
        forbidden |= entry.flags().contains(Flags::NO_EXECUTE);

        if level == Level::Pt || entry.is_huge() {
            let offset = virtual_address & (level.span() - 1);
            // The leaf's own flags, with the narrowed ones put back as the
            // walk computed them.
            let mut effective = entry.flags().difference(NARROWED).union(granted);
            if forbidden {
                effective = effective.union(Flags::NO_EXECUTE);
            }
            return Some((entry.address() + offset, effective, level.span()));
        }
        table = entry.address();
    }
    None
}

/// First address of the window device registers are mapped into.
///
/// Well clear of the direct map below it and the kernel image above it, so a
/// device address is never mistaken for either.
const DEVICE_WINDOW_START: u64 = 0xffff_ff00_0000_0000;

/// One page past the last address device registers may occupy.
const DEVICE_WINDOW_END: u64 = 0xffff_ff01_0000_0000;

/// Device ranges one address space can own before activation.
const MAX_DEVICE_MAPPINGS: usize = 32;

#[derive(Debug, Clone, Copy)]
struct DeviceExtent {
    start: Paddr,
    end: Paddr,
}

/// The stacks [`AddressSpace::map_stacks`] committed, ready to run threads on.
///
/// Held rather than used until the space is active, like [`DeviceMapping`],
/// and not `Copy` or `Clone` for the same reason: it is the evidence that
/// these ranges were mapped by exactly one call, which is what lets
/// [`task::init`](super::task::init) fabricate contexts on them from safe
/// code.
#[derive(Debug)]
pub struct StackArena {
    layout: StackLayout,
    root: Paddr,
}

impl StackArena {
    /// The layout the stacks were mapped to.
    pub(crate) const fn layout(&self) -> &StackLayout {
        &self.layout
    }

    /// First guard or stack page that disagrees with this arena's proof.
    pub(crate) fn invalid_page(&self) -> Option<u64> {
        if !is_active() || active_root() != self.root {
            return Some(self.layout.base());
        }
        for index in 0..self.layout.slots() {
            let Some(slot) = self.layout.slot(index) else {
                return Some(self.layout.base());
            };
            let mut guard = slot.guard_base();
            while guard < slot.stack_base() {
                if translate(guard).is_some() {
                    return Some(guard);
                }
                let Some(next) = guard.checked_add(PAGE_SIZE) else {
                    return Some(guard);
                };
                guard = next;
            }
            let mut stack = slot.stack_base();
            while stack < slot.stack_top() {
                match translate(stack) {
                    Some((_, flags))
                        if flags.contains(Flags::WRITABLE) && flags.contains(Flags::NO_EXECUTE) => {
                    }
                    _ => return Some(stack),
                }
                let Some(next) = stack.checked_add(PAGE_SIZE) else {
                    return Some(stack);
                };
                stack = next;
            }
        }
        None
    }
}

/// A stack this address space mapped, with its guard verified absent.
///
/// Not `Copy` or `Clone`, for the reason [`DeviceMapping`] is not: it is the
/// evidence that exactly one caller owns this region. The root ties it to the
/// space that made it, because a stack address means nothing without one.
#[derive(Debug)]
pub struct MappedStack {
    top: u64,
    root: Paddr,
}

impl MappedStack {
    /// The address the stack grows down from, sixteen-byte aligned.
    pub const fn top(&self) -> u64 {
        self.top
    }

    /// Whether this stack belongs to the space the CPU is walking now.
    ///
    /// A stack mapped into a space that was never installed is not a stack
    /// this CPU can stand on, and the addresses collide across spaces, so the
    /// root is the only thing that can tell them apart.
    pub fn is_installed(&self) -> bool {
        is_active() && active_root() == self.root
    }
}

/// Frames taken out of an address space, and the invalidation they owe.
///
/// D-70 rule 9: unmap returns a token rather than flushing, so a caller taking
/// several mappings away pays for one invalidation rather than one each — and
/// so that the SMP shootdown, when it exists, has something to batch.
///
/// **The frames are not free until the invalidation has run**, and that is a
/// soundness property rather than a convenience. A CPU may still hold a cached
/// translation to them; handing them back before it is dropped would let the
/// allocator give away memory a stale entry still reaches. So the frames live
/// *inside* this token and leave only through [`flush`](Self::flush) or the
/// `Drop` that does the same thing.
///
/// Leaking one with `mem::forget` is therefore safe, which matters: leaking is
/// safe in Rust, so no soundness argument may rest on `Drop` running. The cost
/// of a leak is the frames, not correctness.
#[derive(Debug)]
#[must_use = "the frames are not returned to the allocator until this is flushed or dropped"]
pub struct Unmapped {
    base: u64,
    pages: u64,
    frames: Option<Segment>,
}

impl Unmapped {
    /// Invalidate the range, and hand back the frames it owned.
    ///
    /// The order is the whole point: the cached translations go first, and
    /// only then does anything else become able to allocate these frames.
    pub fn flush(mut self) -> Option<Segment> {
        self.invalidate();
        self.frames.take()
    }

    /// How many pages this took away.
    pub const fn pages(&self) -> u64 {
        self.pages
    }

    /// Drop this CPU's cached translations for the range.
    fn invalidate(&self) {
        for page in 0..self.pages {
            cpu::invalidate_page(self.base + page * PAGE_SIZE);
        }
    }
}

impl Drop for Unmapped {
    /// Invalidate, then let the frames go back. A token that is simply dropped
    /// is as correct as one that is flushed; it just returns the frames to the
    /// allocator instead of to the caller.
    fn drop(&mut self) {
        self.invalidate();
    }
}

/// How large the pages of a mapping may be.
///
/// Not a `bool`, because the two cases are not more-and-less of one thing
/// (`005`, C-CUSTOM-TYPE): one is "use the biggest page that fits", which the
/// direct map wants, and the other is "four kilobytes, always", which anything
/// that must be unmappable wants — because a large page is one `unmap` refuses
/// to split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grain {
    /// The largest page each aligned stretch allows.
    Largest,
    /// Four kilobytes throughout.
    Pages,
}

/// Evidence that a physical range is one the boot handoff described.
///
/// The only way to reach [`AddressSpace::map_boot`], and constructible only
/// from a range the bootloader's memory map covers. Not `Copy`: it is a
/// statement about one range, made once, and a copy would let a caller reuse
/// the evidence for a range nobody checked.
#[derive(Debug)]
pub struct BootRegion {
    base: Paddr,
    len: u64,
}

impl BootRegion {
    /// Describe `len` bytes at `base`, or `None` if the memory map does not
    /// cover all of it.
    ///
    /// `None` also before the map is up, which is the honest answer: until
    /// then nothing has described anything.
    pub fn describe(base: Paddr, len: u64) -> Option<Self> {
        let end = base.checked_add(len)?;
        if len == 0 || !base.is_multiple_of(PAGE_SIZE) {
            return None;
        }
        crate::mm::with_memory_map(|map| map.covers(base, end))
            .unwrap_or(false)
            .then_some(Self { base, len })
    }
}

/// A claim on a device's registers, made while the tables were being built.
///
/// Held rather than used until the space is active — the mapping does not
/// exist for the CPU until then. Not `Copy` or `Clone` on purpose: it is the
/// evidence that exactly one caller owns this device's window, which is what
/// makes [`claim`](Self::claim) safe to expose without an obligation. The
/// root ties the claim to the space that minted it: window addresses collide
/// across spaces, so a stale claim must not cover the installed space's.
#[derive(Debug)]
pub struct DeviceMapping {
    address: u64,
    physical: Paddr,
    len: usize,
    root: Paddr,
}

impl DeviceMapping {
    /// Where the registers are readable, once the space is active.
    pub const fn address(&self) -> u64 {
        self.address
    }

    /// Take the region these registers are reached through.
    ///
    /// Safe because `MmioRegion::new`'s contract holds by construction: the
    /// mapping is permanent, and this value cannot be copied, so no second
    /// region can cover the same bytes. `None` until the space is active, and
    /// `None` forever for a claim minted by a space that never became it.
    pub fn claim(self) -> Option<MmioRegion> {
        if !is_active() || active_root() != self.root {
            return None;
        }
        let mut offset = 0usize;
        while offset < self.len {
            let virtual_address = self.address.checked_add(offset as u64)?;
            let expected = self.physical.checked_add(offset as u64)?;
            let (mapped, _, span) = walk(self.root, virtual_address)?;
            if mapped != expected {
                return None;
            }
            let inside_leaf = virtual_address & (span - 1);
            let available = usize::try_from(span - inside_leaf).ok()?;
            offset = offset.checked_add((self.len - offset).min(available))?;
        }
        // SAFETY: the walk confirmed the mapping is live in the tables that
        // minted this claim; exclusive because `self` cannot be copied and
        // only the minting space's claims pass the root comparison; permanent
        // because `unmap` refuses any entry without `OWNED` and a device
        // mapping never carries it (D-88); four-byte aligned by
        // `map_device_with`.
        Some(unsafe { MmioRegion::new(self.address as *mut u8, self.len) })
    }
}
