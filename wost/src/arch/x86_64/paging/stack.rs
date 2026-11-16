use alloc::vec::Vec;

use stack_layout::{Slot, StackLayout, ranges_overlap};
use x86_paging::{Flags, PAGE_SIZE, page_identity_matches};

use super::{
    AddressSpace, AddressSpaceRoots, MapError, PERMANENT_STACKS, TASK_STACK_PIN, cpu,
    current_mapping_root, is_active, percpu, translate_in, walk,
};
use crate::mm::{self, PhysicalAddress};

impl AddressSpace {
    // The guard is checked first, so a refusal never has a mapping to remove and flush.
    pub fn map_stack(&mut self, slot: &Slot) -> Result<MappedStack, MapError> {
        self.verify_guard(slot)?;
        let pages = (slot.stack_len() / PAGE_SIZE) as usize;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        if !run.zero() {
            return Err(MapError::Unreachable(run.address()));
        }
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

    fn remove_stack(&mut self, slot: &Slot) {
        match self.unmap(slot.stack_base(), slot.stack_len()) {
            Ok(unmapped) => drop(unmapped),
            Err(reason) => panic!("a stack this space mapped could not be unmapped: {reason}"),
        }
    }

    fn remove_stacks(&mut self, layout: &StackLayout, count: usize) {
        for index in (0..count).rev() {
            let Some(slot) = layout.slot(index) else {
                panic!("stack removal lost a checked layout slot");
            };
            self.remove_stack(&slot);
        }
    }

    pub fn map_stacks(&mut self, layout: &StackLayout) -> Result<StackArena, MapError> {
        let mut stacks = Vec::new();
        stacks
            .try_reserve_exact(layout.slots())
            .map_err(|_| MapError::OutOfMemory)?;
        for index in 0..layout.slots() {
            let mapped = layout
                .slot(index)
                .ok_or(MapError::OutOfMemory)
                .and_then(|slot| self.map_stack(&slot));
            match mapped {
                Ok(stack) => stacks.push(stack.identity),
                Err(reason) => {
                    self.remove_stacks(layout, index);
                    return Err(reason);
                }
            }
        }
        Ok(StackArena {
            layout: *layout,
            roots: self.roots(),
            stacks,
        })
    }

    pub fn unmap_stacks(&mut self, arena: StackArena) {
        assert!(
            self.roots == Some(arena.roots),
            "a stack arena is unmapped only from the space that mapped it"
        );
        self.remove_stacks(&arena.layout, arena.layout.slots());
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

fn task_stack_overlap(roots: AddressSpaceRoots, base: u64, len: u64) -> Option<u64> {
    let pinned = *TASK_STACK_PIN.lock_leaf();
    let (pinned_roots, layout) = pinned?;
    if pinned_roots != roots {
        return None;
    }
    for index in 0..layout.slots() {
        let slot = layout.slot(index)?;
        let protected_len = slot.stack_top().checked_sub(slot.guard_base())?;
        if ranges_overlap(slot.guard_base(), protected_len, base, len) {
            return Some(slot.stack_base());
        }
    }
    None
}

pub(super) fn protected_stack_overlap(
    roots: AddressSpaceRoots,
    base: u64,
    len: u64,
) -> Option<u64> {
    permanent_stack_overlap(roots, base, len)
        .or_else(|| task_stack_overlap(roots, base, len))
        .or_else(|| entry_window_overlap(base, len))
}

// Only processor bring-up writes the entry window, and nothing unmaps it.
fn entry_window_overlap(base: u64, len: u64) -> Option<u64> {
    ranges_overlap(percpu::WINDOW, percpu::WINDOW_BYTES, base, len).then_some(base)
}

#[derive(Debug)]
pub struct StackArena {
    layout: StackLayout,
    roots: AddressSpaceRoots,
    stacks: Vec<StackIdentity>,
}

impl StackArena {
    pub(crate) const fn layout(&self) -> &StackLayout {
        &self.layout
    }

    pub(crate) fn pin(self) -> TaskStackPin {
        let mut pinned = TASK_STACK_PIN.lock_leaf();
        assert!(pinned.is_none(), "task stack arena pinned twice");
        *pinned = Some((self.roots, self.layout));
        TaskStackPin {
            stacks: self.stacks,
        }
    }

    pub(crate) fn invalid_page(&self) -> Option<u64> {
        let Some(current) = current_mapping_root(self.roots) else {
            return Some(self.layout.base());
        };
        self.mapping_mismatch_in(current)
    }

    fn mapping_mismatch_in(&self, root: PhysicalAddress) -> Option<u64> {
        if self.stacks.len() != self.layout.slots() {
            return Some(self.layout.base());
        }
        for (index, identity) in self.stacks.iter().enumerate() {
            let Some(slot) = self.layout.slot(index) else {
                return Some(self.layout.base());
            };
            if identity.roots != self.roots || identity.slot != slot {
                return Some(slot.guard_base());
            }
            if let Some(page) = identity.invalid_page_in(root) {
                return Some(page);
            }
        }
        None
    }
}

#[derive(Debug)]
pub(crate) struct TaskStackPin {
    pub(super) stacks: Vec<StackIdentity>,
}

#[derive(Debug)]
pub(crate) struct KernelStackTop<'pin>(&'pin StackIdentity);

impl KernelStackTop<'_> {
    pub(crate) const fn address(self) -> u64 {
        self.0.slot.stack_top()
    }
}

impl TaskStackPin {
    pub(crate) fn stack_top(&self, index: usize) -> Option<KernelStackTop<'_>> {
        let identity = self.stacks.get(index)?;
        if !is_active() {
            return None;
        }
        let current = cpu::page_table_base();
        identity
            .invalid_page_in(current)
            .is_none()
            .then_some(KernelStackTop(identity))
    }
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
