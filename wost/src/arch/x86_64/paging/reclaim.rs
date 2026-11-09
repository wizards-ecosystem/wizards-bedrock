use super::*;

impl AddressSpace {
    /// Take and retain one table frame, or change nothing.
    pub(super) fn take_owned_table(
        &mut self,
        virtual_address: u64,
        level: Level,
        privilege: TablePrivilege,
    ) -> Result<(Paddr, Entry), MapError> {
        self.take_owned_table_in(TableRoot::Full, virtual_address, level, privilege)
    }

    pub(super) fn take_owned_table_in(
        &mut self,
        root: TableRoot,
        virtual_address: u64,
        level: Level,
        privilege: TablePrivilege,
    ) -> Result<(Paddr, Entry), MapError> {
        if level == Level::Pml4
            && !is_user_address(virtual_address)
            && is_active()
            && root == TableRoot::Full
            && self.root() == active_root()
        {
            return Err(MapError::KernelTopologyFrozen(virtual_address));
        }
        self.tables
            .try_reserve(1)
            .map_err(|_| MapError::OutOfMemory)?;
        let pair_authority = self
            .pair_authority
            .map(|authority| authority.acquire_table().ok_or(MapError::OutOfMemory))
            .transpose()?;
        let frame = mm::allocate_zeroed().ok_or(MapError::OutOfMemory)?;
        let address = frame.address();
        let link = Entry::table(address, privilege).map_err(MapError::Encoding)?;
        core::mem::forget(frame);
        self.tables.push(OwnedTable {
            frame: address,
            virtual_address,
            level,
            link,
            root,
        });
        self.pair_authority = pair_authority;
        Ok((address, link))
    }

    /// Give this space's own table frames back to the allocator.
    ///
    /// Mappings must be removed first; reclaim owns only table frames. It
    /// refuses an active or bound space so no current or future `CR3` can walk
    /// freed tables.
    ///
    /// A refusal leaves the space and one-shot release evidence reusable. A
    /// success empties the space and makes every later operation refuse
    /// (D-123).
    pub fn reclaim(&mut self, released: Option<Released>) -> Result<usize, ReclaimFailure> {
        // A bound space needs the task table's linear `Released` evidence, and
        // a repeat could mint an owner for frame zero (D-123, D-139).
        if self.is_reclaimed() {
            assert!(
                self.pair_authority.is_none(),
                "a reclaimed space retained paired frame authority"
            );
            #[cfg(feature = "selftest")]
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return Err(ReclaimFailure {
                reason: ReclaimError::Reclaimed,
                released,
            });
        }
        let roots = self.roots();
        let root = roots.full_root();
        if roots.is_paired() {
            let authority = self
                .pair_authority
                .unwrap_or_else(|| panic!("paired space lost its complete frame authority"));
            assert_eq!(
                authority.table_frames(),
                self.tables.len(),
                "paired space frame authority disagrees with its tables"
            );
            assert_eq!(
                authority.settle(0),
                Ok(PairCleanupState::Retained(authority)),
                "paired cleanup refusal would lose complete authority"
            );
        } else {
            assert!(
                self.pair_authority.is_none(),
                "a shared-root space retained paired frame authority"
            );
        }
        if self.owned_pages != 0 {
            return Err(ReclaimFailure {
                reason: ReclaimError::Mapped(self.owned_pages),
                released,
            });
        }
        if let Some(reason) = self.device_reclaim_error() {
            return Err(ReclaimFailure { reason, released });
        }
        if !self.bound
            && let Some(evidence) = released.as_ref()
        {
            return Err(ReclaimFailure {
                reason: ReclaimError::UnexpectedRelease(evidence.roots().full_root()),
                released,
            });
        }
        if self.bound {
            match released.as_ref() {
                None => {
                    return Err(ReclaimFailure {
                        reason: ReclaimError::StillBound(root),
                        released,
                    });
                }
                Some(evidence) if evidence.roots() != roots => {
                    return Err(ReclaimFailure {
                        reason: ReclaimError::WrongRelease {
                            expected: root,
                            found: evidence.roots().full_root(),
                        },
                        released,
                    });
                }
                Some(_) => {}
            }
        }
        if Self::roots_are_active(roots, cpu::page_table_base()) {
            return Err(ReclaimFailure {
                reason: ReclaimError::Active(cpu::page_table_base()),
                released,
            });
        }

        let returned = self.tables.len() + roots.frames();
        for table in self.tables.drain(..) {
            // SAFETY: each was allocated by `take_table` for this space and
            // forgotten there, so exactly one handle is being rebuilt for it;
            // the space is terminally emptied here. The refusal above
            // establishes that nothing is walking it.
            drop(unsafe { mm::Frame::from_raw(table.frame) });
        }
        if let Some(stripped) = roots.stripped_root() {
            // SAFETY: paired construction allocated and forgot this distinct
            // root exactly once, and the active-root refusal covers it too.
            drop(unsafe { mm::Frame::from_raw(stripped) });
        }
        // The full root, which construction took before there was anywhere to
        // record it. SAFETY: as above.
        drop(unsafe { mm::Frame::from_raw(root) });
        if let Some(authority) = self.pair_authority.take() {
            assert_eq!(
                authority.settle(returned),
                Ok(PairCleanupState::Released),
                "paired cleanup returned an incomplete frame set"
            );
        }
        // Kept as an empty value so later safe operations refuse.
        self.roots = None;
        self.shared.clear();

        Ok(returned)
    }

    /// Whether this space's tables have gone back to the allocator.
    ///
    /// A reclaimed space answers nothing and maps nothing. It is kept as a
    /// value rather than consumed so a refused reclaim can give it back.
    pub const fn is_reclaimed(&self) -> bool {
        self.roots.is_none()
    }

    pub(super) fn roots_are_active(roots: AddressSpaceRoots, current_root: Paddr) -> bool {
        roots.contains(current_root) || (is_active() && roots.contains(active_root()))
    }

    pub(super) fn device_reclaim_error(&self) -> Option<ReclaimError> {
        let mappings = self.devices.iter().flatten().count();
        (mappings != 0).then_some(ReclaimError::Devices(mappings))
    }
}
