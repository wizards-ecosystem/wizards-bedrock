use alloc::vec::Vec;

use stack_layout::{Slot, StackLayout, ranges_overlap};
use x86_paging::{Flags, LARGE_PAGE_SIZE, Level, PAGE_SIZE, TablePrivilege, page_identity_matches};

use super::super::task::{Entry, STACK_FRAMES, Task};
use super::{
    AddressSpace, AddressSpaceRoots, Flush, MapError, PERMANENT_STACKS, TASK_WINDOW, TableRoot,
    clear_mapping_slot, current_mapping_root, entry_slot, is_user_address, leaf_in, percpu,
    translate_in, walk,
};
use crate::mm::pool::ArenaFrame;
use crate::mm::{self, PhysicalAddress};
use crate::sync::ThreadContext;

const TASK_STACK_FLAGS: Flags = Flags::PRESENT
    .union(Flags::WRITABLE)
    .union(Flags::NO_EXECUTE);

// Every task stack's slot; its tables exist from boot, so mapping one takes no frame.
#[derive(Debug)]
pub(super) struct Window {
    layout: StackLayout,
    roots: AddressSpaceRoots,
    // One bit per slot, set while the slot is free.
    free: Vec<u64>,
}

impl Window {
    fn take(&mut self) -> Option<usize> {
        let (word, bits) = self
            .free
            .iter_mut()
            .enumerate()
            .find(|(_, bits)| **bits != 0)?;
        let bit = bits.trailing_zeros() as usize;
        *bits &= !(1 << bit);
        Some(word * 64 + bit)
    }

    fn give(&mut self, index: usize) {
        let bits = &mut self.free[index / 64];
        let bit = 1 << (index % 64);
        assert!(*bits & bit == 0, "task stack slot {index} was freed twice");
        *bits |= bit;
    }
}

fn task_window_overlap(base: u64, len: u64) -> Option<u64> {
    let window = TASK_WINDOW.lock_leaf();
    let layout = window.as_ref()?.layout;
    let window_len = layout.end() - layout.base();
    ranges_overlap(layout.base(), window_len, base, len).then_some(base)
}

// A task stack's slot and the frames mapped there, owned by its task.
#[derive(Debug)]
pub(crate) struct TaskStack {
    index: usize,
    top: u64,
    frames: [ArenaFrame; STACK_FRAMES],
}

impl TaskStack {
    pub(crate) const fn top(&self) -> u64 {
        self.top
    }

    pub(crate) const fn top_frame(&mut self) -> &mut ArenaFrame {
        &mut self.frames[STACK_FRAMES - 1]
    }
}

// Unmapped, with its frames held until every processor that could cache them has flushed.
#[derive(Debug)]
#[must_use = "a retired task's frames return only through its release"]
pub struct RetiredTask {
    index: usize,
    frames: Option<[ArenaFrame; STACK_FRAMES]>,
    flush: Flush,
}

impl RetiredTask {
    pub fn release(mut self, context: &mut ThreadContext) {
        core::mem::replace(&mut self.flush, Flush::none()).wait(context);
    }
}

impl Drop for RetiredTask {
    fn drop(&mut self) {
        assert!(
            self.flush.is_complete(),
            "a task stack's frames returned before its flush completed"
        );
        let frames = self.frames.take();
        if let Some(window) = TASK_WINDOW.lock_leaf().as_mut() {
            window.give(self.index);
        }
        drop(frames);
    }
}

impl AddressSpace {
    // Before activation, while the kernel half's top level may still grow.
    pub fn prepare_task_window(&mut self, base: u64) -> Result<usize, MapError> {
        if is_user_address(base) || base >= percpu::WINDOW {
            return Err(MapError::NotYours(base));
        }
        if !base.is_multiple_of(LARGE_PAGE_SIZE) {
            return Err(MapError::Misaligned(base));
        }
        if TASK_WINDOW.lock_leaf().is_some() {
            return Err(MapError::Conflict(base));
        }
        let pages = STACK_FRAMES as u64 + 1;
        let affordable = mm::managed_frames() / (STACK_FRAMES + 1);
        let room = ((percpu::WINDOW - base) / (pages * PAGE_SIZE)) as usize;
        let slots = affordable.min(room);
        let layout = StackLayout::new(base, STACK_FRAMES as u64 * PAGE_SIZE, PAGE_SIZE, slots)
            .ok_or(MapError::Conflict(base))?;
        let mut free = Vec::new();
        free.try_reserve_exact(slots.div_ceil(64))
            .map_err(|_| MapError::OutOfMemory)?;
        for word in 0..slots.div_ceil(64) {
            let left = slots - word * 64;
            free.push(if left >= 64 {
                u64::MAX
            } else {
                (1 << left) - 1
            });
        }
        let mut address = base;
        while address < layout.end() {
            self.prepare_table(address)?;
            address += LARGE_PAGE_SIZE;
        }
        let mut window = TASK_WINDOW.lock_leaf();
        if window.is_some() {
            return Err(MapError::Conflict(base));
        }
        *window = Some(Window {
            layout,
            roots: self.roots(),
            free,
        });
        Ok(slots)
    }

    fn prepare_table(&mut self, virtual_address: u64) -> Result<(), MapError> {
        let mut table = self.root();
        for level in Level::WALK {
            if level == Level::Pt {
                return Ok(());
            }
            let slot = entry_slot(table, level.index(virtual_address))?;
            let current = slot.read();
            table = if current.is_present() {
                if current.is_huge() {
                    return Err(MapError::Conflict(virtual_address));
                }
                PhysicalAddress::from_entry(current)
            } else {
                let (frame, link) = self.take_owned_table(
                    TableRoot::Full,
                    virtual_address,
                    level,
                    TablePrivilege::Supervisor,
                )?;
                slot.write(link);
                frame
            };
        }
        Ok(())
    }

    // `interruptible` is the creator's flag, read before any lock cleared it.
    pub fn map_task(
        &mut self,
        record: ArenaFrame,
        stack: [ArenaFrame; STACK_FRAMES],
        entry: Entry,
        argument: usize,
        interruptible: bool,
    ) -> Result<Task, MapError> {
        let roots = self.roots();
        let (index, slot) = {
            let mut window = TASK_WINDOW.lock_leaf();
            let Some(window) = window.as_mut().filter(|window| window.roots == roots) else {
                return Err(MapError::NotYours(self.root().get()));
            };
            let index = window.take().ok_or(MapError::OutOfMemory)?;
            let Some(slot) = window.layout.slot(index) else {
                panic!("task stack slot {index} lies outside its window");
            };
            (index, slot)
        };
        for (page, frame) in stack.iter().enumerate() {
            let address = slot.stack_base() + page as u64 * PAGE_SIZE;
            let physical = frame.address().get();
            if let Err(reason) = self.set(
                TableRoot::Full,
                address,
                physical,
                TASK_STACK_FLAGS,
                Level::Pt,
            ) {
                panic!("free task stack slot {index} could not be mapped: {reason}");
            }
        }
        let stack = TaskStack {
            index,
            top: slot.stack_top(),
            frames: stack,
        };
        Ok(Task::fabricate(
            record,
            stack,
            entry,
            argument,
            interruptible,
        ))
    }

    // Refused while the task may run, is bound or is the boot task; its record goes at once.
    #[expect(
        clippy::result_large_err,
        reason = "a refused task goes back whole to the owner that must keep it"
    )]
    pub fn unmap_task(&mut self, mut task: Task) -> Result<RetiredTask, Task> {
        let roots = self.roots();
        let Some(layout) = TASK_WINDOW
            .lock_leaf()
            .as_ref()
            .filter(|window| window.roots == roots)
            .map(|window| window.layout)
        else {
            return Err(task);
        };
        if task.hold_for_release().is_err() {
            return Err(task);
        }
        let Some((record, stack)) = task.take_owned() else {
            panic!("a held task lost its frames");
        };
        drop(task);
        let Some(slot) = layout.slot(stack.index) else {
            panic!("task stack slot {} lies outside its window", stack.index);
        };
        let mut flush = Flush::none();
        for (page, frame) in stack.frames.iter().enumerate() {
            let address = slot.stack_base() + page as u64 * PAGE_SIZE;
            let entry = leaf_in(self.root(), address)
                .unwrap_or_else(|reason| panic!("a task stack page went unmapped: {reason}"));
            assert!(
                entry.read().address() == frame.address().get(),
                "a task stack page maps a frame its task does not own"
            );
            flush.merge(clear_mapping_slot(entry, address, Level::Pt, roots));
        }
        drop(record);
        Ok(RetiredTask {
            index: stack.index,
            frames: Some(stack.frames),
            flush,
        })
    }
}

impl AddressSpace {
    // The guard is checked first, so a refusal never has a mapping to remove and flush.
    pub fn map_stack(&mut self, slot: &Slot) -> Result<MappedStack, MapError> {
        self.verify_guard(slot)?;
        let pages = (slot.stack_len() / PAGE_SIZE) as usize;
        let run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        let physical_base = run.address();
        self.map_owned(
            slot.stack_base(),
            run,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )?;

        Ok(MappedStack {
            identity: StackIdentity {
                roots: self.roots(),
                slot: *slot,
                physical_base,
            },
        })
    }

    fn verify_guard(&self, slot: &Slot) -> Result<(), MapError> {
        let mut guard = slot.guard_base();
        while guard < slot.stack_base() {
            if self.lookup(guard).is_some() {
                return Err(MapError::Conflict(guard));
            }
            guard = guard
                .checked_add(PAGE_SIZE)
                .ok_or(MapError::Conflict(guard))?;
        }
        Ok(())
    }
}

fn retain_permanent_stack(identity: StackIdentity) -> Result<(), MapError> {
    let mut pinned = PERMANENT_STACKS.lock_leaf();
    let Some(room) = pinned.iter_mut().find(|room| room.is_none()) else {
        let existing = pinned
            .iter()
            .flatten()
            .next()
            .map_or(identity.slot.stack_base(), |stack| stack.slot.stack_base());
        return Err(MapError::Pinned(existing));
    };
    *room = Some(identity);
    Ok(())
}

fn permanent_stack_overlap(roots: AddressSpaceRoots, base: u64, len: u64) -> Option<u64> {
    let pinned = PERMANENT_STACKS.lock_leaf();
    pinned.iter().flatten().find_map(|identity| {
        (identity.roots == roots && identity.overlaps(base, len))
            .then_some(identity.slot.stack_base())
    })
}

pub(super) fn protected_stack_overlap(
    roots: AddressSpaceRoots,
    base: u64,
    len: u64,
) -> Option<u64> {
    permanent_stack_overlap(roots, base, len)
        .or_else(|| task_window_overlap(base, len))
        .or_else(|| entry_window_overlap(base, len))
}

// Only processor bring-up writes the entry window, and nothing unmaps it.
fn entry_window_overlap(base: u64, len: u64) -> Option<u64> {
    ranges_overlap(percpu::WINDOW, percpu::WINDOW_BYTES, base, len).then_some(base)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct StackIdentity {
    pub(super) roots: AddressSpaceRoots,
    pub(super) slot: Slot,
    pub(super) physical_base: PhysicalAddress,
}

impl StackIdentity {
    fn overlaps(&self, base: u64, len: u64) -> bool {
        let Some(protected_len) = self.slot.stack_top().checked_sub(self.slot.guard_base()) else {
            return true;
        };
        ranges_overlap(self.slot.guard_base(), protected_len, base, len)
    }

    pub(super) fn invalid_page_in(&self, root: PhysicalAddress) -> Option<u64> {
        let mut guard = self.slot.guard_base();
        while guard < self.slot.stack_base() {
            if translate_in(root, guard).is_some() {
                return Some(guard);
            }
            let Some(next) = guard.checked_add(PAGE_SIZE) else {
                return Some(guard);
            };
            guard = next;
        }

        let expected_flags = Flags::PRESENT
            .union(Flags::WRITABLE)
            .union(Flags::NO_EXECUTE)
            .union(Flags::OWNED);
        let mut stack = self.slot.stack_base();
        while stack < self.slot.stack_top() {
            let Some(offset) = stack.checked_sub(self.slot.stack_base()) else {
                return Some(stack);
            };
            let Some(physical) = self.physical_base.get().checked_add(offset) else {
                return Some(stack);
            };
            match walk(root, stack) {
                Some((found, flags, span))
                    if page_identity_matches(
                        found.get(),
                        flags,
                        span,
                        physical,
                        expected_flags,
                    ) => {}
                _ => return Some(stack),
            }
            let Some(next) = stack.checked_add(PAGE_SIZE) else {
                return Some(stack);
            };
            stack = next;
        }
        None
    }
}

#[derive(Debug)]
pub struct MappedStack {
    pub(super) identity: StackIdentity,
}

impl MappedStack {
    pub const fn top(&self) -> u64 {
        self.identity.slot.stack_top()
    }

    pub(crate) fn is_installed(&self) -> bool {
        current_mapping_root(self.identity.roots)
            .is_some_and(|current| self.identity.invalid_page_in(current).is_none())
    }

    pub(crate) fn retain(self) -> Result<PinnedStack, MapError> {
        if !self.is_installed() {
            return Err(MapError::NotMapped(self.identity.slot.stack_base()));
        }
        retain_permanent_stack(self.identity)?;
        Ok(PinnedStack { stack: self })
    }
}

#[derive(Debug)]
pub struct PinnedStack {
    stack: MappedStack,
}

impl PinnedStack {
    pub const fn top(&self) -> u64 {
        self.stack.top()
    }
}
