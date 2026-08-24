//! The kernel's own page tables (D-45).
//!
//! [`AddressSpace`] builds a four-level table out of frames from
//! [`crate::mm`]; [`activate`] switches the CPU to it. The encoding lives in
//! `x86_paging`, host-tested against the manual (D-30), so what is here is
//! what needs a raw pointer: walking a table through the direct map, and the
//! `mov cr3` itself.
//!
//! Two design points that the signatures alone do not give away. [`activate`]
//! is safe because it walks the new tables for its own code and stack before
//! loading them, so a table that would not survive the switch is refused
//! rather than triple-faulting. And it takes the space *by value*, which is
//! what keeps [`AddressSpace::map`] safe — nothing can map into a table the
//! CPU is already walking.

use core::fmt;
use core::sync::atomic::{AtomicU64, Ordering};

use stack_layout::StackLayout;
use x86_paging::{ENTRIES, Entry, EntryError, Flags, LARGE_PAGE_SIZE, Level, PAGE_SIZE};

use super::cpu;
use crate::mm::{self, Paddr};
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
}

impl From<EntryError> for MapError {
    fn from(error: EntryError) -> Self {
        Self::Encoding(error)
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
/// deliberately leaked from the frame allocator rather than tracked, because
/// nothing unmaps and a table freed underneath a live `CR3` is the one bug
/// this design must not permit.
#[derive(Debug)]
pub struct AddressSpace {
    root: Paddr,
    /// Next free address in the device window.
    device_next: u64,
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
    pub fn map(
        &mut self,
        virtual_base: u64,
        physical_base: Paddr,
        len: u64,
        flags: Flags,
    ) -> Result<(), MapError> {
        if !virtual_base.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(virtual_base));
        }
        if !physical_base.is_multiple_of(PAGE_SIZE) {
            return Err(MapError::Misaligned(physical_base));
        }
        let flags = flags.union(Flags::PRESENT);

        let mut offset = 0;
        while offset < len {
            let virtual_address = virtual_base
                .checked_add(offset)
                .ok_or(MapError::NonCanonical(virtual_base))?;
            let physical = physical_base
                .checked_add(offset)
                .ok_or(MapError::Encoding(EntryError::OutOfRange(physical_base)))?;
            if !x86_paging::is_canonical(virtual_address) {
                return Err(MapError::NonCanonical(virtual_address));
            }

            let large = len - offset >= LARGE_PAGE_SIZE
                && virtual_address.is_multiple_of(LARGE_PAGE_SIZE)
                && physical.is_multiple_of(LARGE_PAGE_SIZE);
            let level = if large { Level::Pd } else { Level::Pt };

            self.set(virtual_address, physical, flags, level)?;
            offset += level.span();
        }
        Ok(())
    }

    /// Map a device's registers, and hand back the claim on them.
    ///
    /// Uncacheable and never executable, at an address in the device window
    /// rather than the physical one. `len` is rounded up to whole pages.
    ///
    /// # Errors
    ///
    /// As [`map`](Self::map), plus [`MapError::OutOfMemory`] if the device
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
        let offset = physical % PAGE_SIZE;
        let base = physical - offset;
        let pages = (len + offset).div_ceil(PAGE_SIZE) * PAGE_SIZE;

        let virtual_base = self
            .device_next
            .checked_add(pages)
            .filter(|end| *end <= DEVICE_WINDOW_END)
            .map(|_| self.device_next)
            .ok_or(MapError::OutOfMemory)?;

        self.map(
            virtual_base,
            base,
            pages,
            Flags::WRITABLE.union(Flags::NO_EXECUTE).union(memory_type),
        )?;
        self.device_next = virtual_base + pages;

        Ok(DeviceMapping {
            address: virtual_base + offset,
            len,
            root: self.root,
        })
    }

    /// Map a device whose writes should coalesce rather than cross the bus one
    /// at a time.
    ///
    /// Selects the entry [`super::cpu::enable_write_combining`] sets. A caller
    /// that has not enabled it gets write-through: slow, never wrong.
    ///
    /// # Errors
    ///
    /// As [`map_device`](Self::map_device).
    pub fn map_device_write_combining(
        &mut self,
        physical: Paddr,
        len: u64,
    ) -> Result<DeviceMapping, MapError> {
        self.map_device_with(physical, len, Flags::WRITE_THROUGH)
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
    /// As [`map`](Self::map), plus [`MapError::OutOfMemory`] if any slot's
    /// frames cannot be allocated. Frames already committed stay committed —
    /// the space that failed is abandoned whole, exactly as a failed
    /// [`activate`] abandons it.
    pub fn map_stacks(&mut self, layout: &StackLayout) -> Result<StackArena, MapError> {
        for index in 0..layout.slots() {
            let slot = layout.slot(index).ok_or(MapError::OutOfMemory)?;
            let frames = slot.stack_len.div_ceil(PAGE_SIZE) as usize;
            let mut run = mm::allocate_contiguous(frames, 1).ok_or(MapError::OutOfMemory)?;
            if !run.zero() {
                return Err(MapError::Unreachable(run.address()));
            }
            self.map(
                slot.stack_base,
                run.address(),
                slot.stack_len,
                Flags::WRITABLE.union(Flags::NO_EXECUTE),
            )?;
            // The frames belong to the mapping for the life of the kernel,
            // like the tables themselves: nothing unmaps.
            core::mem::forget(run);
        }
        Ok(StackArena { layout: *layout })
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
                let flags = if target == Level::Pt {
                    flags
                } else {
                    flags.union(Flags::HUGE)
                };
                let entry = Entry::page(physical, flags)?;
                // SAFETY: `entry_pointer` confirmed the slot is inside a table
                // frame this space owns and reachable through the direct map.
                let current = unsafe { slot.read_volatile() };
                // Only an identical rewrite is allowed: anything else would
                // re-point a mapping some handle is evidence of.
                if current.is_present() && current != entry {
                    return Err(MapError::Conflict(virtual_address));
                }
                // SAFETY: as above. Nothing else writes these tables: `map`
                // takes `&mut self`, and `activate` consumes the space so no
                // mapping can follow the switch.
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
                current.address()
            } else {
                let frame = take_table()?;
                // SAFETY: as above.
                unsafe { slot.write_volatile(Entry::table(frame)?) };
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
pub fn activate(space: AddressSpace) -> Result<(), ActivateError> {
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

    let code = activate as *const () as u64;
    match translate_in(space.root, code) {
        None => return Err(ActivateError::CodeUnmapped(code)),
        Some((_, flags)) if flags.contains(Flags::NO_EXECUTE) => {
            return Err(ActivateError::CodeNotExecutable(code));
        }
        Some(_) => {}
    }

    let stack = cpu::stack_pointer();
    for probe in [stack, stack.saturating_sub(STACK_HEADROOM)] {
        match translate_in(space.root, probe) {
            None => return Err(ActivateError::StackUnmapped(probe)),
            Some((_, flags)) if !flags.contains(Flags::WRITABLE) => {
                return Err(ActivateError::StackReadOnly(probe));
            }
            Some(_) => {}
        }
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
    // The map in force covers strictly less than the bootloader's did.
    mm::narrow_direct_map();

    // `space` is dropped here, and that is correct: an `AddressSpace` owns no
    // frame it would give back. Its tables were leaked when they were taken,
    // because nothing unmaps and a table freed under a live `CR3` is the one
    // bug this design must not permit. Taking the space by value is about the
    // *caller* — it no longer has anything to call `map` on.
    Ok(())
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
                            && flags.contains(Flags::WRITABLE)
                            && flags.contains(Flags::NO_EXECUTE) =>
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

    let mut table = root;
    for level in Level::WALK {
        let slot = entry_pointer(table, level.index(virtual_address)).ok()?;
        // SAFETY: `entry_pointer` confirmed the slot is inside a table
        // reachable through the direct map. Reading an entry cannot alias
        // anything: the tables are only written before activation.
        let entry = unsafe { slot.read_volatile() };
        if !entry.is_present() {
            return None;
        }

        // The huge bit means something else entirely in a leaf table, so the
        // level is checked first.
        if level == Level::Pt || entry.is_huge() {
            let offset = virtual_address & (level.span() - 1);
            return Some((entry.address() + offset, entry.flags(), level.span()));
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
}

impl StackArena {
    /// The layout the stacks were mapped to.
    pub(crate) const fn layout(&self) -> &StackLayout {
        &self.layout
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
    len: u64,
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
        if !is_active() || active_root() != self.root || translate(self.address).is_none() {
            return None;
        }
        // SAFETY: the walk confirmed the mapping is live in the tables that
        // minted this claim; exclusive because `self` cannot be copied and
        // only the minting space's claims pass the root comparison; permanent
        // because nothing unmaps; four-byte aligned by `map_device_with`.
        Some(unsafe { MmioRegion::new(self.address as *mut u8, self.len as usize) })
    }
}
