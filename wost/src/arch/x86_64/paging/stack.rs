use alloc::vec::Vec;

use stack_layout::{Slot, StackLayout, ranges_overlap};
use x86_paging::{Flags, PAGE_SIZE, page_identity_matches};

#[cfg(doc)]
use super::DeviceMapping;
use super::{
    AddressSpace, AddressSpaceRoots, MapError, PERMANENT_STACKS, StackCleanup, StackMapError,
    StackRollback, TASK_STACK_PIN, cpu, current_mapping_root, is_active, translate_in, walk,
};
#[cfg(feature = "selftest")]
use super::{STACK_MAPPINGS_BEFORE_REFUSAL, STACK_ROLLBACKS_BEFORE_REFUSAL, stack_refusal};
use crate::mm::{self, Paddr};

impl AddressSpace {
    /// Map one guarded stack, or refuse with `Conflict` at the first guard page found mapped.
    ///
    /// The [`MappedStack`] proves zeroed writable, never-executable pages and an absent guard
    /// before [`task::continue_on_stack`](super::task::continue_on_stack) uses it.
    pub fn map_stack(&mut self, slot: &Slot) -> Result<MappedStack, StackMapError> {
        #[cfg(feature = "selftest")]
        if stack_refusal(&STACK_MAPPINGS_BEFORE_REFUSAL) {
            return Err(StackMapError::Mapping(MapError::OutOfMemory));
        }
        let pages = slot.stack_len().div_ceil(PAGE_SIZE) as usize;
        let mut run = mm::allocate_contiguous(pages, 1).ok_or(MapError::OutOfMemory)?;
        if !run.zero() {
            return Err(MapError::Unreachable(run.address()).into());
        }
        let physical_base = run.address();
        self.map_owned(
            slot.stack_base(),
            run,
            Flags::WRITABLE.union(Flags::NO_EXECUTE),
        )?;

        // Guard verification follows mapping, so a refusal must return the just-owned frames.
        if let Err(cause) = self.verify_guard(slot) {
            return match self.rollback_stack(slot, 0) {
                Ok(_) => Err(StackMapError::Mapping(cause)),
                Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
            };
        }

        Ok(MappedStack {
            identity: StackIdentity {
                roots: self.roots(),
                slot: *slot,
                physical_base,
            },
        })
    }

    pub(super) fn rollback_stack(
        &mut self,
        slot: &Slot,
        returned_pages: usize,
    ) -> Result<usize, StackRollback> {
        #[cfg(feature = "selftest")]
        if stack_refusal(&STACK_ROLLBACKS_BEFORE_REFUSAL) {
            return Err(StackRollback {
                reason: MapError::Pinned(slot.stack_base()),
                returned_pages,
                roots: self.roots(),
                pending: StackCleanup::One(*slot),
            });
        }
        let returned = self
            .unmap(slot.stack_base(), slot.stack_len())
            .map_err(|reason| StackRollback {
                reason,
                returned_pages,
                roots: self.roots(),
                pending: StackCleanup::One(*slot),
            })?;
        let pages = returned.pages() as usize;
        drop(returned);
        Ok(pages)
    }

    pub(super) fn rollback_stack_prefix(
        &mut self,
        layout: &StackLayout,
        mut remaining: usize,
        mut returned_pages: usize,
    ) -> Result<usize, StackRollback> {
        assert!(
            remaining <= layout.slots(),
            "stack rollback progress exceeded its layout"
        );
        while remaining != 0 {
            let index = remaining - 1;
            let Some(slot) = layout.slot(index) else {
                panic!("stack rollback lost a checked layout slot");
            };
            #[cfg(feature = "selftest")]
            if stack_refusal(&STACK_ROLLBACKS_BEFORE_REFUSAL) {
                return Err(StackRollback {
                    reason: MapError::Pinned(slot.stack_base()),
                    returned_pages,
                    roots: self.roots(),
                    pending: StackCleanup::Prefix {
                        layout: *layout,
                        remaining,
                    },
                });
            }
            let returned = self
                .unmap(slot.stack_base(), slot.stack_len())
                .map_err(|reason| StackRollback {
                    reason,
                    returned_pages,
                    roots: self.roots(),
                    pending: StackCleanup::Prefix {
                        layout: *layout,
                        remaining,
                    },
                })?;
            returned_pages = returned_pages
                .checked_add(returned.pages() as usize)
                .unwrap_or_else(|| panic!("stack rollback page accounting overflowed"));
            drop(returned);
            remaining -= 1;
        }
        Ok(returned_pages)
    }

    /// Map one guarded stack whose evidence refuses every overlapping map or unmap while hardware
    /// may retain its address; `Pinned` if a pinned stack exists, `Inactive` off the CPU's space.
    pub fn map_pinned_stack(&mut self, slot: &Slot) -> Result<PinnedStack, StackMapError> {
        let roots = self.roots();
        if current_mapping_root(roots).is_none() {
            return Err(MapError::Inactive(roots.full_root()).into());
        }
        if let Some(existing) = first_permanent_stack() {
            return Err(MapError::Pinned(existing).into());
        }
        let stack = self.map_stack(slot)?;
        if let Err(reason) = retain_permanent_stack(stack.identity) {
            return match self.rollback_stack(slot, 0) {
                Ok(_) => Err(StackMapError::Mapping(reason)),
                Err(cleanup) => Err(StackMapError::Rollback {
                    cause: reason,
                    cleanup,
                }),
            };
        }
        Ok(PinnedStack { stack })
    }

    /// Map every stack of `layout` as one contiguous run each, zeroed before mapping, writable and
    /// never executable.
    ///
    /// Guard gaps stay unmapped so an overrun faults instead of reaching a neighbour's stack. A
    /// refusal rolls every earlier stack back; [`StackMapError::Rollback`] reports cleanup exactly.
    pub fn map_stacks(&mut self, layout: &StackLayout) -> Result<StackArena, StackMapError> {
        let mut stacks = Vec::new();
        stacks
            .try_reserve_exact(layout.slots())
            .map_err(|_| StackMapError::Mapping(MapError::OutOfMemory))?;
        for mapped_stacks in 0..layout.slots() {
            let Some(slot) = layout.slot(mapped_stacks) else {
                let cause = MapError::OutOfMemory;
                return match self.rollback_stack_prefix(layout, mapped_stacks, 0) {
                    Ok(_) => Err(StackMapError::Mapping(cause)),
                    Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
                };
            };
            match self.map_stack(&slot) {
                Ok(stack) => stacks.push(stack.identity),
                Err(reason) => {
                    let (cause, committed, returned_pages) = match reason {
                        StackMapError::Mapping(cause) => (cause, mapped_stacks, 0),
                        StackMapError::Rollback { cause, cleanup } => {
                            assert!(
                                cleanup.roots == self.roots()
                                    && cleanup.mapped_stacks() == 1
                                    && cleanup.returned_pages == 0
                                    && cleanup.remaining_stacks() == 1,
                                "single-stack rollback lost its cleanup authority"
                            );
                            (cause, mapped_stacks + 1, cleanup.returned_pages)
                        }
                    };
                    return match self.rollback_stack_prefix(layout, committed, returned_pages) {
                        Ok(_) => Err(StackMapError::Mapping(cause)),
                        Err(cleanup) => Err(StackMapError::Rollback { cause, cleanup }),
                    };
                }
            }
        }
        Ok(StackArena {
            layout: *layout,
            roots: self.roots(),
            stacks,
        })
    }

    /// Remove every stack represented by `arena`, returning its frames.
    pub fn unmap_stacks(&mut self, arena: StackArena) -> Result<usize, StackRollback> {
        if arena.roots != self.roots() {
            return Err(StackRollback {
                reason: MapError::NotYours(arena.layout.base()),
                returned_pages: 0,
                roots: arena.roots,
                pending: StackCleanup::Prefix {
                    layout: arena.layout,
                    remaining: arena.layout.slots(),
                },
            });
        }
        self.rollback_stack_prefix(&arena.layout, arena.layout.slots(), 0)
    }

    /// Nothing is mapped in the pages below `slot`'s stack.
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

fn first_permanent_stack() -> Option<u64> {
    core::iter::Iterator::next(&mut PERMANENT_STACKS.lock_leaf().iter().flatten())
        .map(|stack| stack.slot.stack_base())
}

fn retain_permanent_stack(identity: StackIdentity) -> Result<(), MapError> {
    let mut pinned = PERMANENT_STACKS.lock_leaf();
    let Some(room) = pinned.iter_mut().find(|room| room.is_none()) else {
        let existing = core::iter::Iterator::next(&mut pinned.iter().flatten())
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
    permanent_stack_overlap(roots, base, len).or_else(|| task_stack_overlap(roots, base, len))
}

/// The stacks [`AddressSpace::map_stacks`] committed, as linear evidence that lets
/// [`task::init`](super::task::init) fabricate contexts only for the ranges one mapping made.
#[derive(Debug)]
pub struct StackArena {
    layout: StackLayout,
    roots: AddressSpaceRoots,
    stacks: Vec<StackIdentity>,
}

impl StackArena {
    /// The layout the stacks were mapped to.
    pub(crate) const fn layout(&self) -> &StackLayout {
        &self.layout
    }

    /// Retain every stack mapping permanently and return its task-table owner.
    pub(crate) fn pin(self) -> TaskStackPin {
        let mut pinned = TASK_STACK_PIN.lock_leaf();
        assert!(pinned.is_none(), "task stack arena pinned twice");
        *pinned = Some((self.roots, self.layout));
        TaskStackPin {
            stacks: self.stacks,
        }
    }

    /// First guard or stack page that disagrees with this arena's proof.
    pub(crate) fn invalid_page(&self) -> Option<u64> {
        let Some(current) = current_mapping_root(self.roots) else {
            return Some(self.layout.base());
        };
        self.mapping_mismatch_in(current)
    }

    fn mapping_mismatch_in(&self, root: Paddr) -> Option<u64> {
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

/// The task table's access to every permanently retained arena stack.
#[derive(Debug)]
pub(crate) struct TaskStackPin {
    pub(super) stacks: Vec<StackIdentity>,
}

/// A task-stack top borrowed from a pinned arena identity.
#[derive(Debug)]
pub(crate) struct KernelStackTop<'pin>(&'pin StackIdentity);

impl KernelStackTop<'_> {
    /// The address hardware may retain in `RSP0`.
    pub(crate) const fn address(self) -> u64 {
        self.0.slot.stack_top()
    }
}

impl TaskStackPin {
    /// The pinned stack at `index`, as retained-hardware authority.
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
    pub(super) physical_base: Paddr,
}

impl StackIdentity {
    fn overlaps(&self, base: u64, len: u64) -> bool {
        let Some(protected_len) = self.slot.stack_top().checked_sub(self.slot.guard_base()) else {
            return true;
        };
        ranges_overlap(self.slot.guard_base(), protected_len, base, len)
    }

    pub(super) fn invalid_page_in(&self, root: Paddr) -> Option<u64> {
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
            let Some(physical) = self.physical_base.checked_add(offset) else {
                return Some(stack);
            };
            match walk(root, stack) {
                Some((found, flags, span))
                    if page_identity_matches(found, flags, span, physical, expected_flags) => {}
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

/// A stack this space mapped with its guard absent, linear for the reason [`DeviceMapping`] is.
///
/// The root ties it to the space that made it, because a stack address means nothing without one.
#[derive(Debug)]
pub struct MappedStack {
    pub(super) identity: StackIdentity,
}

impl MappedStack {
    /// The address the stack grows down from, sixteen-byte aligned.
    pub const fn top(&self) -> u64 {
        self.identity.slot.stack_top()
    }

    /// Whether the CPU walks this stack's own space, which only the root tells apart.
    ///
    /// A complete re-walk needs every guard absent and every page the original owned, supervisor,
    /// write-back, writable, non-executable mapping, so partial and substituted stacks fail.
    pub fn is_installed(&self) -> bool {
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

/// A guarded stack its space records as pinned, since hardware keeps addresses, not Rust values.
///
/// No safe caller can remove, replace or fill the stack or its guard once hardware retains it.
#[derive(Debug)]
pub struct PinnedStack {
    stack: MappedStack,
}

impl PinnedStack {
    /// The address the stack grows down from, sixteen-byte aligned.
    pub const fn top(&self) -> u64 {
        self.stack.top()
    }

    pub(crate) fn is_installed(&self) -> bool {
        self.stack.is_installed()
    }
}
