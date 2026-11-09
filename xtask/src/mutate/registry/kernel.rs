use crate::qemu;

use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 17,
        file: "kernel/src/nvme.rs",
        find: "    match disabled.access(|registers| write(registers, offset, value)) {",
        replace: "    match disabled.access(|registers| {\n        #[cfg(debug_assertions)]\n        let stack_pressure = [0u8; 3072];\n        #[cfg(not(debug_assertions))]\n        let stack_pressure = [0u8; 16 * 1024];\n        core::hint::black_box(&stack_pressure);\n        let result = write(registers, offset, value);\n        core::hint::black_box(&stack_pressure);\n        result\n    }) {",
        breaks: "NVMe preparation consumes the two-page boot-stack safety margin",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 55,
        file: "kernel/src/vfs.rs",
        find: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            for entry in listing.entries() {",
        replace: "            let listing = crate::tmpfs::list(index).ok_or(VfsError::NotFound)?;\n            static REGRESSION: wost::sync::SpinLock<()> = wost::sync::SpinLock::new(());\n            let _backend = REGRESSION.lock_leaf();\n            for entry in listing.entries() {",
        breaks: "a directory callback runs while its backend still owns a leaf lock",
        reddens: Caught::Check("enumeration callbacks run without backend ownership"),
    },
    Mutation {
        id: 56,
        file: "kernel/src/nvme.rs",
        find: "        match device.admit_bus_master(authority, domain, interrupts) {",
        replace: "        match device\n            .admit_bus_master(authority, domain, interrupts)\n            .or(Ok::<(), pci_config::EnableError>(()))\n        {",
        breaks: "a refused bus-master transition mints start authority",
        reddens: Caught::Check("a failed bus master admission stops before nvme start"),
    },
    Mutation {
        id: 57,
        file: "kernel/src/input.rs",
        find: "            KeyState::Released => {\n                if self.down & bit != 0 {\n                    self.completed[index] = self.completed[index].wrapping_add(1);\n                }\n                self.down &= !bit;\n            }",
        replace: "            KeyState::Released => {\n                self.completed[index] = self.completed[index].wrapping_add(1);\n                self.down &= !bit;\n            }",
        breaks: "an interrupt witness accepts a release with no preceding press",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        id: 169,
        file: "kernel/src/queue.rs",
        find: "            if published == 0 || published > SUBMISSIONS {",
        replace: "            if published == 0 {",
        breaks: "a runaway submission tail is bounded",
        reddens: Caught::Check("a rewritten submission cannot change what was served"),
    },
    // Without the fixup the fault is terminal, so its panic is the causal observer.
    Mutation {
        id: 174,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            if usercopy::recover(frame, fault) =>",
        replace: "            if false && usercopy::recover(frame, fault) =>",
        breaks: "a faulting user copy resumes instead of stopping the machine",
        reddens: Caught::Panic {
            marker: "faulting address 0x0000600000000000",
            why: "without the fixup the fault is terminal, so the kernel dies before it \
                  reaches a verdict at all",
        },
    },
    // Trap-context logging is caught by ThreadLock's fail-stop (D-122).
    Mutation {
        id: 175,
        file: "kernel/src/user.rs",
        find: "            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);",
        replace: "            crate::warn!(\"refused\");",
        breaks: "an unserved call does not log from trap context",
        reddens: Caught::Panic {
            marker: "a trap handler may not take a thread lock",
            why: "logging from trap context trips `ThreadLock`'s assert, which is the \
                  defect being a defect",
        },
    },
    // Inspecting the ring under the process-table guard recreates nested leaf locking.
    Mutation {
        id: 178,
        file: "kernel/src/user.rs",
        find: "    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))",
        replace: "    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot).and_then(Option::as_ref)?;
    let _ = bootstrap.ring.shape();
    Some((
        alloc::sync::Arc::clone(&bootstrap.ring),
        alloc::sync::Arc::clone(&bootstrap.table),
    ))",
        breaks: "no lock is taken while a leaf guard is held",
        reddens: Caught::Check("no lock is taken while a leaf guard is held"),
    },
    Mutation {
        id: 179,
        file: "kernel/src/user.rs",
        find: "        .filter(|birth| birth.table == spawned.observed_table_incarnation)",
        replace: "        .filter(|birth| {\n            birth.table == spawned.observed_table_incarnation.wrapping_add(1)\n        })",
        breaks: "birth evidence belongs to the exact authority-table incarnation",
        reddens: Caught::Check("a spawned program holds exactly one handle"),
    },
    Mutation {
        id: 182,
        file: "kernel/src/user.rs",
        find: "    let binding = construction\n        .space()?\n        .bind()\n        .map_err(SpawnError::NotBindable)?;",
        replace: "    let _duplicate_binding = AddressSpace::bind(construction.space()?);\n    let binding = construction\n        .space()?\n        .bind()\n        .map_err(SpawnError::NotBindable)?;",
        breaks: "a second production owner can bind one address space",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        id: 183,
        file: "kernel/src/user.rs",
        find: "            authority: Some(BootstrapAuthority::Bound(bound)),",
        replace: "            authority: None,",
        breaks: "a committed process discards its live address-space binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        id: 184,
        file: "kernel/src/user.rs",
        find: "        BootstrapAuthority::Bound(bound) => match task::release_address_space(bound) {\n            Ok(released) => Some(BootstrapAuthority::Released(released)),\n            Err(bound) => {\n                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));\n                return Err(ReclaimRetention {\n                    refusal: ReclaimRefusal::TaskRelease,\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }\n        },",
        replace: "        BootstrapAuthority::Bound(bound) => Some(BootstrapAuthority::Bound(bound)),",
        breaks: "zombie reclamation skips release of a live address-space binding",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        id: 185,
        file: "kernel/src/user.rs",
        find: "    let tables = match bootstrap.space.reclaim(Some(released)) {",
        replace: "    let tables = match bootstrap.space.reclaim(None) {",
        breaks: "address-space reclamation withholds its exact release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_production_owner_binds_and_retires_each_address_space",
        },
    },
    Mutation {
        id: 186,
        file: "kernel/src/devfs.rs",
        find: "pub(crate) const DEVICES: [Device; 2] = [Device::Storage, Device::Console];",
        replace: "pub(crate) const DEVICES: [Device; 1] = [Device::Storage];",
        breaks: "a device variant is absent from the devfs listing",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_devfs_source_inventory_matches_every_variant",
        },
    },
    Mutation {
        id: 187,
        file: "kernel/src/selftest.rs",
        find: "        let Ok(mut victim) = AddressSpace::for_user() else {\n            if let Err(reason) = observer.reclaim(None) {\n                warn!(\"selftest: observer cleanup after victim-space setup refusal: {reason}\");\n            }\n            return false;\n        };",
        replace: "        let Ok(mut victim) = AddressSpace::for_user() else {\n            let _ = observer.reclaim(None);\n            return false;\n        };",
        breaks: "a setup refusal discards address-space cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    // Retained Phase 0-5 audit checks (D-135 to D-145).
    Mutation {
        id: 188,
        file: "kernel/src/console.rs",
        find: "        self.hide_cursor();
        self.put(text);
        self.newline();
        self.show_cursor();",
        replace: "        self.hide_cursor();
        self.put(text);
        self.newline();",
        breaks: "a line break leaves its new cursor hidden",
        reddens: Caught::Check("a scroll moves one visible cursor"),
    },
    Mutation {
        id: 189,
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
}",
        replace: "pub(crate) fn with_console(f: impl FnOnce(&mut Console)) {
    let disabled = wost::arch::current::interrupt::disable();
    let mut guard = CONSOLE.lock();
    if let Some(console) = guard.as_mut() {
        f(console);
    }
    disabled.restore();
}",
        breaks: "console work keeps interrupts available",
        reddens: Caught::Check("a console scroll does not block interrupts"),
    },
    Mutation {
        id: 194,
        file: "kernel/src/handle.rs",
        find: "        let slot = &mut self.slots[index];
        let generation = slot.generation;
        slot.entry = Some(entry);",
        replace: "        let generation = self.slots[index].generation;
        let _ = entry;",
        breaks: "a claimed slot is filled by the call that claimed it",
        reddens: Caught::Check("filling the arena never repeats a slot"),
    },
    // Charging one slot too few leaves the final insertion no prepaid room,
    // the boundary that would allocate or panic in trap context (D-182).
    Mutation {
        id: 197,
        file: "kernel/src/handle.rs",
        find: "        let id = commit_table(quota)?;",
        replace: "        let id = commit_table(quota.saturating_sub(1))?;",
        breaks: "a table's final quota slot is backed before any trap can use it",
        reddens: Caught::Panic {
            marker: "a charged handle had no committed arena slot",
            why: "the first charged handle reaches the missing arena slot before a verdict",
        },
    },
    Mutation {
        id: 198,
        file: "kernel/src/handle.rs",
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.table.fill(object, granted);",
        replace: "        let granted = offered;
        let _ = declared;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.table.fill(object, granted);",
        breaks: "rights attenuate on arrival",
        reddens: Caught::Check("a completion carries a handle with narrowed rights"),
    },
    Mutation {
        id: 199,
        file: "kernel/src/object.rs",
        find: "            Self::Channel => Rights::NONE,",
        replace: "            Self::Channel => Rights::READ,",
        breaks: "a bootstrap identity admits no speculative channel data rights",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        id: 200,
        file: "kernel/src/object.rs",
        find: "            Self::File => Rights::READ,",
        replace: "            Self::File => Rights::READ.union(Rights::WRITE),",
        breaks: "a file refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    Mutation {
        id: 201,
        file: "kernel/src/object.rs",
        find: "            Self::Directory => Rights::TRAVERSE.union(Rights::ENUMERATE),",
        replace: "            Self::Directory => Rights::TRAVERSE
                .union(Rights::ENUMERATE)
                .union(Rights::MODIFY),",
        breaks: "a directory refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    // The kernel reads Echo's argument from the old handle field while the
    // independent ring-3 emitter keeps writing the widened offset.
    Mutation {
        id: 202,
        file: "kernel/src/queue.rs",
        find: "    pub(super) const ARGUMENT: usize = 24;",
        replace: "    pub(super) const ARGUMENT: usize = 8;",
        breaks: "the emitted program and kernel use one submission layout",
        reddens: Caught::Panic {
            marker: "a reserved completion slot was not there",
            why: "the mismatched submission layout consumes a completion reservation without its \
                  matching operation",
        },
    },
    // Skipping release leaks one promised handle slot after every refusal.
    Mutation {
        id: 203,
        file: "kernel/src/handle.rs",
        find: "        if !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        replace: "        if false && !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        breaks: "a refused operation gives its promised slot back",
        reddens: Caught::Check("a refused handle operation returns its promise"),
    },
    // Only a pre-deadline observation distinguishes deferral from immediate completion.
    Mutation {
        id: 204,
        file: "kernel/src/queue.rs",
        find: "self.begin_settlement(|entry| {
            entry
                .kind
                .deadline()
                .is_some_and(|deadline| !clock_live || deadline <= now_ns)
        })",
        replace: "self.begin_settlement(|entry| entry.kind.deadline().is_some())",
        breaks: "an operation is not due before its deadline",
        reddens: Caught::Check("an operation completes after its deadline and only once"),
    },
    // Cancellation is a second operation, not a substitute for its target.
    Mutation {
        id: 205,
        file: "kernel/src/queue.rs",
        find: "        if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        replace: "        if false && !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        breaks: "a cancelled operation still gets its own completion",
        reddens: Caught::Panic {
            marker: "a reserved completion slot was not there",
            why: "the skipped settlement consumes the cancellation's reserved slot before a verdict",
        },
    },
    // A plausible shifted clock is exposed only by comparing both publication doors.
    Mutation {
        id: 206,
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift());",
        replace: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift() - 1);",
        breaks: "the page and the kernel clock tell the same time",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // Leaving the sequence odd makes every correct reader spin until timeout.
    Mutation {
        id: 207,
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));",
        replace: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));",
        breaks: "a published clock closes its update window",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // Publishing counter mode after revocation defeats the fail-closed reader path.
    Mutation {
        id: 208,
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::MODE, mode);",
        replace: "    write_u32(page, CLOCK_AT + clock::MODE, MODE_COUNTER);",
        breaks: "a revoked clock reads as revoked",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    // The time page mapped writable, which is a program able to tell every
    // other program a lie about the time.
    Mutation {
        id: 209,
        file: "kernel/src/timepage.rs",
        find: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);",
        replace: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE.union(Flags::WRITABLE));",
        breaks: "the time page is read-only to ring 3",
        reddens: Caught::Check("a program sees the time page and cannot write it"),
    },
    // Forgetting dead work strands both its completion and its reservation.
    Mutation {
        id: 210,
        file: "kernel/src/queue.rs",
        find: "            if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
                break;
            }
            drop(FILE_WORK.remove(self, index, entry.id));
            ended += 1;",
        replace: "            let _ = entry;
            self.pending.lock_leaf().slots[index] = None;
            ended += 1;",
        breaks: "a dead program's work is finished, not forgotten",
        reddens: Caught::Check("work in flight is ended before the ring is released"),
    },
    // A separately allocated table can die beneath the deferred promise.
    Mutation {
        id: 211,
        file: "kernel/src/handle.rs",
        find: "        Ok(SlotReservation {
            table: Arc::clone(self),
            spent: false,
        })",
        replace: "        Ok(SlotReservation {
            table: if wost::arch::current::interrupt::in_trap() {
                Arc::clone(self)
            } else {
                Arc::new(HandleTable::new(self.quota).expect(\"mutation table\"))
            },
            spent: false,
        })",
        breaks: "a promise holds the table it is against",
        reddens: Caught::Panic {
            marker: "a live reservation refused quota release",
            why: "the foreign table keeps the reservation live when its real table drops before a verdict",
        },
    },
    // Q35 leaves a bus-zero-only walk looking plausible, so the proof counts
    // buses rather than functions (D-168).
    Mutation {
        id: 212,
        file: "kernel/src/pci.rs",
        find: "                if entry.class.is_bridge()
                    && entry.header.is_bridge()",
        replace: "                if false
                    && entry.header.is_bridge()",
        breaks: "enumeration descends through bridges",
        reddens: Caught::Check("the bus walk descends through a bridge"),
    },
    // The ports a device decodes left free for anyone to claim, which is the
    // arrangement D-77 said proves the wrong thing.
    Mutation {
        id: 214,
        file: "kernel/src/pci.rs",
        find: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])",
        replace: "        let owned = wost::arch::current::port::claim_spans::<BARS>(&requested[..0])",
        breaks: "a device claim covers the ports it decodes",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Consume the span handles without returning their bits; the witness
    // reacquires the same BDF, so registry-only cleanup cannot pass.
    Mutation {
        id: 215,
        file: "kernel/src/pci.rs",
        find: "        if let Err(failure) = wost::arch::current::port::release_spans(ports) {
            self.ports = failure.spans;
            return ReleaseOutcome::refused(ReleaseError::Ports, self);
        }",
        replace: "        drop(ports);",
        breaks: "a transient pci lease returns every dynamic io span",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Record the BDF before its fallible device/resource preparation. Any
    // later refusal then poisons a permanent registry slot.
    Mutation {
        id: 235,
        file: "kernel/src/pci.rs",
        find: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    if let Err(failure) = pci::release_function(function) {
        retain_failed_function(failure.into_function());
        return ClaimError::FunctionRetained;
    }
    reason
}",
        replace: "fn release_clean_function(function: pci::FunctionClaim, reason: ClaimError) -> ClaimError {
    drop(function);
    reason
}",
        breaks: "a refused device preparation leaves its registry unchanged",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // Replace the controller-bound validation with one known-good request;
    // the submission interceptor catches it before any doorbell.
    Mutation {
        id: 247,
        file: "kernel/src/nvme.rs",
        find: "        let request = self.validate_read(lba, blocks, buffer_bytes)?;",
        replace: "        let request = self\n            .validate_read(lba, blocks, buffer_bytes)\n            .or_else(|_| {\n                ReadTransfer::new(\n                    0,\n                    1,\n                    self.blocks,\n                    u64::from(self.block_bytes),\n                    u64::MAX,\n                )\n                .map_err(BringUpError::InvalidTransfer)\n            })?;",
        breaks: "raw nvme reads cannot bypass controller-bound validation",
        reddens: Caught::Check("invalid nvme reads stop before submission"),
    },
    // Replace refused device data with a known-good synthetic namespace. The
    // self-test queue interceptor catches the bypass before allocation or MMIO.
    Mutation {
        id: 257,
        file: "kernel/src/nvme.rs",
        find: "        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;",
        replace: "        let namespace = Namespace::parse(identify)\n            .or_else(|_| {\n                let mut accepted = [0u8; NAMESPACE_IDENTIFY_BYTES];\n                accepted[0] = 1;\n                accepted[128 + 2] = 9;\n                Namespace::parse(&accepted)\n            })\n            .map_err(BringUpError::InvalidNamespace)?;",
        breaks: "invalid namespace data cannot bypass io queue admission",
        reddens: Caught::Check("invalid nvme namespace stops before io queues"),
    },
    // The phase tag never flips: fine until the queue wraps, so the check
    // must issue more commands than the queue has entries.
    Mutation {
        id: 260,
        file: "kernel/src/nvme.rs",
        find: "                if completion_head == 0 {
                    queue.phase = !queue.phase;
                }",
        replace: "                if false {
                    queue.phase = !queue.phase;
                }",
        breaks: "a completion queue wraps and stays readable",
        reddens: Caught::Check("the queues wrap and the phase tag still works"),
    },
    // The volume mounted at sector zero, which is where a reader that could
    // not be given an offset would look.
    Mutation {
        id: 265,
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 0;",
        breaks: "the volume is where the partition puts it",
        reddens: Caught::Check("the volume mounts and says what a fat32 volume says"),
    },
    // Globally unique fixture names hide a missing parent comparison, so two
    // fixture directories deliberately contain `MOTD` (D-172).
    Mutation {
        id: 269,
        file: "kernel/src/tmpfs.rs",
        find: "        if index == ROOT || entry.parent != directory {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        replace: "        if index == ROOT {
            continue;
        }
        if &entry.name[..entry.name_len] != name {",
        breaks: "a directory holds only its own entries",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // A path walked *through* a device, which has nothing inside it — the
    // "everything is a file" conflation arriving as a defect.
    Mutation {
        id: 270,
        file: "kernel/src/vfs.rs",
        find: "    if rest.is_empty() {
        Ok(Node::Device(device))
    } else {
        Err(VfsError::NotADirectory)
    }",
        replace: "    let _ = rest;
    Ok(Node::Device(device))",
        breaks: "a device is not a directory",
        reddens: Caught::Check("a device is not a file"),
    },
    // A tmpfs read bounded by the arena rather than by the file, which hands
    // back the next file's bytes.
    Mutation {
        id: 271,
        file: "kernel/src/tmpfs.rs",
        find: "    let wanted = into.len().min(len - offset);",
        replace: "    let wanted = into.len();",
        breaks: "a read stops at the end of the file, in memory too",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // A list-only directory handle must not become traversal authority
    // (D-173).
    Mutation {
        id: 272,
        file: "kernel/src/queue.rs",
        find: "        if !held.contains(Rights::TRAVERSE) {
            return Some(refuse(token, STATUS_DENIED));
        }",
        replace: "        if false {
            return Some(refuse(token, STATUS_DENIED));
        }",
        breaks: "an open needs the right to traverse",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // A file handle granted WRITE, which nothing can honour: there is no write
    // path, and a right that grants nothing is a syscall name in disguise.
    Mutation {
        id: 273,
        file: "kernel/src/queue.rs",
        find: "                    request
                        .held
                        .intersection(Rights::UNIVERSAL)
                        .union(Rights::READ),
                ),",
        replace: "                    request
                        .held
                        .intersection(Rights::UNIVERSAL)
                        .union(Rights::READ)
                        .union(Rights::WRITE),
                ),",
        breaks: "an open grants only what it can honour",
        reddens: Caught::Check("a completion carries a file handle"),
    },
    // A file opened through an attenuated directory regains the universal
    // rights its parent gave up. TRAVERSE authorizes READ, not re-delegation.
    Mutation {
        id: 274,
        file: "kernel/src/queue.rs",
        find: "                    request
                        .held
                        .intersection(Rights::UNIVERSAL)
                        .union(Rights::READ),
                ),",
        replace: "                    Rights::UNIVERSAL.union(Rights::READ),
                ),",
        breaks: "an open never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // The same widening defect on the other branch: copying a directory's
    // identity must not copy rights its parent did not hold.
    Mutation {
        id: 275,
        file: "kernel/src/queue.rs",
        find: "                crate::vfs::Node::Directory(directory) => {
                    (crate::object::directory(directory), request.held)
                }",
        replace: "                crate::vfs::Node::Directory(directory) => (
                    crate::object::directory(directory),
                    Rights::UNIVERSAL.union(request.held),
                ),",
        breaks: "an opened directory never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // A malformed OpenAt path inspected before its slot is promised; a full
    // table must report quota first, as Duplicate does.
    Mutation {
        id: 276,
        file: "kernel/src/queue.rs",
        find: "        let Ok(reservation) = table.reserve_slot() else {
            return Some(refuse(token, STATUS_NO_HANDLE_SLOT));
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return Some(refuse(token, STATUS_BAD_BUFFER));
        };",
        replace: "        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return Some(refuse(token, STATUS_BAD_BUFFER));
        };
        let Ok(reservation) = table.reserve_slot() else {
            return Some(refuse(token, STATUS_NO_HANDLE_SLOT));
        };",
        breaks: "every handle-producing operation reserves its result before other admission work",
        reddens: Caught::Check("a full table refuses a handle operation at submission"),
    },
    // Duplicate's rights check pulled ahead of the promise; a full table
    // must report quota even for malformed rights.
    Mutation {
        id: 277,
        file: "kernel/src/queue.rs",
        find: "        let named = submission.handle;",
        replace: "        let named = submission.handle;
        if u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
            .is_none()
        {
            return refuse(token, STATUS_DENIED);
        }",
        breaks: "Duplicate reserves its result before validating requested rights",
        reddens: Caught::Check("a full table refuses a handle operation at submission"),
    },
    // A read that ignores its offset. Nothing holds a cursor, so the offset
    // alone says where the bytes came from.
    Mutation {
        id: 278,
        file: "kernel/src/queue.rs",
        find: "        match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {",
        replace: "        match crate::vfs::read(request.file, 0, &mut scratch[..request.wanted]) {",
        breaks: "a read starts where it was told to",
        reddens: Caught::Check("a read is a submission with its own offset"),
    },
    // The buffer bound moved after the read. The output is unchanged, so
    // only the driver's command counter can tell the kernels apart (D-173).
    Mutation {
        id: 279,
        file: "kernel/src/queue.rs",
        find: "    (end <= DATA_BYTES).then_some((at, len))",
        replace: "    let _ = end;
    Some((at, len))",
        breaks: "a buffer is checked before the device is asked",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // A rights word read as its low half, so undefined bits are dropped
    // rather than refused (D-174).
    Mutation {
        id: 280,
        file: "kernel/src/queue.rs",
        find: "        let Some(declared) = u32::try_from(submission.argument)
            .ok()
            .and_then(Rights::from_bits)
        else {",
        replace: "        let Some(declared) = Rights::from_bits(submission.argument as u32) else {",
        breaks: "a rights word that does not fit is refused",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // A file offset wrapped into the file rather than past it, so byte four
    // billion and one is answered with byte one.
    Mutation {
        id: 281,
        file: "kernel/src/queue.rs",
        find: "        let offset = u32::try_from(submission.offset).unwrap_or(u32::MAX);",
        replace: "        let offset = submission.offset as u32;",
        breaks: "an offset that does not fit is past the end",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // A cancellation token truncated, so a program ends an operation it did
    // not name.
    Mutation {
        id: 282,
        file: "kernel/src/queue.rs",
        find: "        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };",
        replace: "        let target = submission.argument as u32;",
        breaks: "a token that does not fit names no operation",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // D-133's exemption, put back. The waiter still runs on the slice already
    // armed, so only the counter distinguishes this from a working wake.
    Mutation {
        id: 283,
        file: "kernel/src/thread.rs",
        find: "    if woken > 0 {
        #[cfg(feature = \"selftest\")]
        if interrupt::in_trap() {",
        replace: "    if woken > 0 && !interrupt::in_trap() {
        #[cfg(feature = \"selftest\")]
        if interrupt::in_trap() {",
        breaks: "a syscall wake arms the timer for what it woke",
        reddens: Caught::Check("a syscall wake arms the timer for what it woke"),
    },
    // A discriminant and its `match` arm drifting apart, which is how a
    // diagnosis comes to name the wrong guard (D-176).
    Mutation {
        id: 284,
        file: "kernel/src/time.rs",
        find: "            6 => Self::Disagreed,",
        replace: "            6 => Self::NotBelievable,",
        breaks: "the clock's refusal decodes to the one that was recorded",
        reddens: Caught::Check("every refusal the clock can record reads back as itself"),
    },
    // Launch the program on its Echo route; only the handle count proves the
    // trap exercised Duplicate. Kernel source, because the driver cannot rebuild itself.
    Mutation {
        id: 286,
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-duplicate\", 6, DUPLICATE)",
        replace: "    spawn(\"user-duplicate\", 6, BEHAVE)",
        breaks: "the shipped program uses its bootstrap authority through Duplicate",
        reddens: Caught::Check("a program duplicates its bootstrap handle"),
    },
    // The identity read as a translating controller would rewrite it, which is
    // the state the check exists to catch (D-178).
    Mutation {
        id: 287,
        file: "kernel/src/selftest.rs",
        find: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);",
        replace: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);",
        breaks: "the keyboard's identity is the untranslated one",
        reddens: Caught::Check("the keyboard identifies as one the decoder knows"),
    },
    // QEMU makes the bit and identity agree, so only omitted re-derivation is observable.
    Mutation {
        id: 289,
        file: "kernel/src/input.rs",
        find: "    keyboard.decoder = decoder_for(&mut keyboard.controller);",
        replace: "    let _ = &mut keyboard.controller;",
        breaks: "translation leaves the decoder on its previous numbering",
        reddens: Caught::Check("translation updates the decoder from the controller identity"),
    },
    // Seeding against the stream makes an unadopted numbering change observable.
    Mutation {
        id: 290,
        file: "kernel/src/input.rs",
        find: "        if translated != self.decoder.reads_translated() {",
        replace: "        if false && translated != self.decoder.reads_translated() {",
        breaks: "the numbering the stream proved is the one the decoder adopts",
        reddens: Caught::Check("the keyboard's numbering and the decoder's are the same"),
    },
    Mutation {
        id: 294,
        file: "kernel/src/input.rs",
        find: "pub(crate) const HUMAN_WAIT_MS: u64 = 45_000;",
        replace: "pub(crate) const HUMAN_WAIT_MS: u64 = 90_000;",
        breaks: "three unanswered keystroke rounds end as a red check, not a hang",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_human_wait_fits_the_harness_patience",
        },
    },
    // Treat an unverified stop as verified; a fake device completes after
    // the allocator could already reuse the command's memory.
    Mutation {
        id: 298,
        file: "kernel/src/nvme.rs",
        find: "            self.disposition = DmaDisposition::Retained;",
        replace: "            self.disposition = DmaDisposition::Quiesced;",
        breaks: "an unverified controller stop retains its dma memory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unproven_stop_retains_every_reachable_run",
        },
    },
    // Repair follows the red observation so the outstanding command remains observable.
    Mutation {
        id: 300,
        file: "kernel/src/nvme.rs",
        find: "            core::hint::spin_loop();\n        }\n        self.fail_submitted(BringUpError::NotReady, data, isolation, data_mapped)",
        replace: "            core::hint::spin_loop();\n        }\n        Err(BringUpError::NotReady)",
        breaks: "a command timeout settles dma before returning",
        reddens: Caught::Check("an nvme timeout quiesces before dma memory returns"),
    },
    // Re-export the private closure that can hand any safe kernel module the
    // running `AddressSpace`, including its reclaim and arbitrary-unmap doors.
    Mutation {
        id: 301,
        file: "kernel/src/mem.rs",
        find: "fn with_active_space<T>(",
        replace: "pub(crate) fn with_active_space<T>(",
        breaks: "the running address space has no broad safe borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::no_item_mem_exposes_names_the_running_address_space",
        },
    },
    // Skip terminal table reclamation while reporting success.
    Mutation {
        id: 303,
        file: "kernel/src/user.rs",
        find: "        match space.reclaim(released) {",
        replace: "        match { drop(released); Ok::<usize, wost::arch::current::paging::ReclaimFailure>(0) } {",
        breaks: "a refused process construction returns its page tables",
        reddens: Caught::Check("every refused spawn returns every resource"),
    },
    // Admit an entry outside every executable segment. The image is otherwise
    // valid, so this reaches mapping unless the entry participates in preflight.
    Mutation {
        id: 304,
        file: "kernel/src/user.rs",
        find: "    if !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        replace: "    if false && !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        breaks: "an ELF entry must be inside executable program memory",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    // Return after the first injected retirement refusal without completing
    // the task-table retirement or publishing the scheduler slot.
    Mutation {
        id: 306,
        file: "kernel/src/thread.rs",
        find: "    if refuse(SpawnPoint::Fabricated) {\n        if let Err(reason) = retire_prepared(slot) {\n            finish_refused_retirement(slot, reason);\n            return Err(PreparedFailure::before_binding(\n                SpawnError::Retirement { slot, reason },\n                binding,\n            ));\n        }\n        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));\n    }",
        replace: "    if refuse(SpawnPoint::Fabricated) {\n        if let Err(reason) = retire_prepared(slot) {\n            return Err(PreparedFailure::before_binding(\n                SpawnError::Retirement { slot, reason },\n                binding,\n            ));\n        }\n        return Err(PreparedFailure::before_binding(SpawnError::Full, binding));\n    }",
        breaks: "a first retirement refusal is completed before slot publication",
        reddens: Caught::Check("a retirement refusal is settled before slot reuse"),
    },
    Mutation {
        id: 307,
        file: "kernel/src/user.rs",
        find: "const MAX_REGIONS: usize = MAX_SEGMENTS + 1;",
        replace: "const MAX_REGIONS: usize = MAX_SEGMENTS;",
        breaks: "the largest image and its stack both fit the cleanup ledger",
        reddens: Caught::Check("a maximum image and stack fit the cleanup ledger"),
    },
    Mutation {
        id: 308,
        file: "kernel/src/user.rs",
        find: "                if let Err(refusal) = undo(space, &placements[..done]) {\n                    return Err(LoadError::Rollback {\n                        reason: refusal.reason,\n                        pages: refusal.pages,\n                    });\n                }",
        replace: "                if let Err(refusal) = undo(space, &placements[..done]) {\n                    let _ = refusal;\n                }",
        breaks: "a refused loader rollback is returned with its exact progress",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        id: 309,
        file: "kernel/src/user.rs",
        find: "                    #[cfg(feature = \"selftest\")]\n                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {\n                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);\n                        continue;\n                    }",
        replace: "                    #[cfg(feature = \"selftest\")]\n                    if CLEANUP_REFUSAL_INJECTED.swap(false, Ordering::AcqRel) {\n                        REFUSED_MAPPING_RECLAIMS.fetch_add(1, Ordering::AcqRel);\n                        return;\n                    }",
        breaks: "construction cleanup retries the exact mapping that refused once",
        reddens: Caught::Check("cleanup refusals retain reachable tables"),
    },
    Mutation {
        id: 310,
        file: "kernel/src/user.rs",
        find: "    if !extended_state_is_isolated() {\n        return Err(SpawnError::NoExtendedState);\n    }",
        replace: "    if false && !extended_state_is_isolated() {\n        return Err(SpawnError::NoExtendedState);\n    }",
        breaks: "ring three stays closed without isolated extended state",
        reddens: Caught::Check("missing extended state closes the user boundary"),
    },
    Mutation {
        id: 311,
        file: "kernel/src/selftest.rs",
        find: "    let Some(Ok(returned)) = mem::unmap_for_test(&mut context(), mapping) else {\n        return false;\n    };\n    drop(returned);",
        replace: "    drop(mapping);",
        breaks: "the live-root mapping check returns its owned frame",
        reddens: Caught::Check("a mapping added after the switch is live"),
    },
    Mutation {
        id: 312,
        file: "kernel/src/selftest.rs",
        find: "    let checked = check(&mut space);\n    let reclaimed = space.reclaim(None).is_ok();\n    drop(space);",
        replace: "    let checked = check(&mut space);\n    drop(space);\n    let reclaimed = true;",
        breaks: "throwaway address spaces explicitly return every table frame",
        reddens: Caught::Check("a second address space is refused"),
    },
    Mutation {
        id: 316,
        file: "kernel/src/mem.rs",
        find: "    match space.reclaim(None) {",
        replace: "    match {\n        drop(space);\n        Ok::<usize, ReclaimFailure>(0)\n    } {",
        breaks: "a failed boot-space build reclaims its partially built tables",
        reddens: Caught::Check("a failed boot-space build returns every table frame"),
    },
    Mutation {
        id: 317,
        file: "kernel/src/user.rs",
        find: "            Err(bound) => {\n                bootstrap.authority = Some(BootstrapAuthority::Bound(bound));\n                return Err(ReclaimRetention {\n                    refusal: ReclaimRefusal::TaskRelease,\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }",
        replace: "            Err(bound) => {\n                drop(bound);\n                return Ok(ReclaimSuccess {\n                    reclaimed: Reclaimed::default(),\n                    #[cfg(feature = \"selftest\")]\n                    observed_returned_pages: 0,\n                });\n            }",
        breaks: "a refused task release retains its linear retry authority",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 318,
        file: "kernel/src/thread.rs",
        find: "            .find(|slot| threads.queue.slots[*slot].state == State::Free && !zombie_slots[*slot])",
        replace: "            .find(|slot| threads.queue.slots[*slot].state == State::Free)",
        breaks: "scheduler admission excludes every slot with a retained zombie",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    // Consume a pending operation when its terminal post refuses; only a
    // witness inspecting ownership before the retry sees the loss.
    Mutation {
        id: 321,
        file: "kernel/src/queue.rs",
        find: "            pending.slots[index] = Some(Pending {\n                settling: false,\n                ..entry\n            });\n            #[cfg(feature = \"selftest\")]\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        replace: "            pending.slots[index] = None;\n            #[cfg(feature = \"selftest\")]\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        breaks: "a refused terminal post retains its pending operation",
        reddens: Caught::Check("a refused settlement retains every owner"),
    },
    // Continue construction after the boot stack guard refused; only the
    // explicit boundary state distinguishes this from a normal launch.
    Mutation {
        id: 322,
        file: "kernel/src/user.rs",
        find: "    if !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        replace: "    if false && !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        breaks: "an unguarded boot stack cannot open the user boundary",
        reddens: Caught::Check("an unguarded boot stack closes the user boundary"),
    },
    // D-390 makes PCI configuration mandatory before memory initialization,
    // so removing the unconditional survey is the reachable behavior change.
    Mutation {
        id: 324,
        file: "kernel/src/main.rs",
        find: "    survey_the_bus(dma_non_requester, dma_quiesced);\n    survey_the_filesystems();\n    start_interrupts(routing.as_ref());",
        replace: "    survey_the_bus(dma_non_requester, dma_quiesced);\n    start_interrupts(routing.as_ref());",
        breaks: "every admitted boot publishes its hardware-free namespaces",
        reddens: Caught::BootMarker(qemu::DEGRADED_FILESYSTEM_MARKER),
    },
    // Restore everything but stop enumeration anyway, reproducing the
    // first-candidate-only selector without conflating it with cleanup.
    Mutation {
        id: 332,
        file: "kernel/src/main.rs",
        find: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Seeking
}",
        replace: "    info!(\"nvme: candidate {ordinal} retired cleanly; trying next\");
    StorageSelection::Halted
}",
        breaks: "nvme selection advances after an exact candidate retirement",
        reddens: Caught::Check("a later nvme controller survives an earlier failure"),
    },
    // Publish queue addresses before moving them into their cleanup owner.
    Mutation {
        id: 333,
        file: "kernel/src/nvme.rs",
        find: "    // Cleanup owns every published queue address before the enable write.
    let controller = Controller {",
        replace: "    let _ = write(&mut registers, register::CC, cc);
    let controller = Controller {",
        breaks: "nvme cleanup owns every published queue before controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::nvme_cleanup_and_bus_master_authority_precede_the_enable_write",
        },
    },
    Mutation {
        id: 337,
        file: "kernel/src/queue.rs",
        find: "const WORK_DEPTH: usize = 8;",
        replace: "const WORK_DEPTH: usize = 7;",
        breaks: "file-work admission accepts its exact documented finite capacity",
        reddens: Caught::Check("file work pressure is bounded at admission"),
    },
    Mutation {
        id: 338,
        file: "kernel/src/queue.rs",
        find: "        matches!(
            self,
            Self::Timer { .. }
                | Self::OpenAt {
                    phase: WorkPhase::Queued
                }
                | Self::Read {
                    phase: WorkPhase::Queued,
                    ..
                }
        )",
        replace: "        matches!(
            self,
            Self::Timer { .. }
                | Self::OpenAt {
                    phase: WorkPhase::Queued
                }
                | Self::Read { .. }
        )",
        breaks: "a running file read remains owned through teardown until its worker settles",
        reddens: Caught::Check(
            "a delayed read preserves interrupt admission and exact teardown",
        ),
    },
    Mutation {
        id: 339,
        file: "kernel/src/wait.rs",
        find: "            let mut waiters = self.waiters.lock(context);
            if ready() {",
        replace: "            let mut waiters = self.waiters.lock(context);
            if false && ready() {",
        breaks: "a waiter blocks without rereading its condition after registration",
        reddens: Caught::Check("a waiter rereads its condition before blocking"),
    },
    Mutation {
        id: 340,
        file: "kernel/src/wait.rs",
        find: "        let switched = registered && thread::wait_from_trap(frame, until);",
        replace: "        let switched = registered && false && thread::wait_from_trap(frame, until);",
        breaks: "RingWait hands its caller away through the live trap frame",
        reddens: Caught::Check("a program waits through its trap frame"),
    },
    Mutation {
        id: 341,
        file: "kernel/src/thread.rs",
        find: "            entity: Entity::new(weight),",
        replace: "            entity: Entity::new(BASE_WEIGHT),",
        breaks: "the weighted scheduler check constructs two equal entities",
        reddens: Caught::Check("runtime divides in proportion to weight"),
    },
    Mutation {
        id: 342,
        file: "kernel/src/user.rs",
        find: "    if !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        replace: "    if false && !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        breaks: "ring three requires enforced non-executable mappings",
        reddens: Caught::Check("no-execute protection closes the user boundary"),
    },
    Mutation {
        id: 343,
        file: "kernel/src/user.rs",
        find: "    if !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        replace: "    if false && !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        breaks: "an uncalibrated boot never launches an unpreemptible program",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        id: 344,
        file: "kernel/src/queue.rs",
        find: "                if !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        replace: "                if false && !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        breaks: "a clockless Timer is refused before it becomes pending",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        id: 345,
        file: "kernel/src/thread.rs",
        find: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);\n        return;\n    }",
        replace: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_STARTING, Ordering::Release);\n        return;\n    }",
        breaks: "a refused idle spawn leaves its reservation permanently burned",
        reddens: Caught::Check("a failed idle start can be retried exactly once"),
    },
    Mutation {
        id: 346,
        file: "kernel/src/user.rs",
        find: "    if !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        replace: "    if false && !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        breaks: "ring three opens without a thread able to reclaim it",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        id: 347,
        file: "kernel/src/user.rs",
        find: "    if stored {\n        crate::queue::request_reap();\n    }",
        replace: "    let _ = stored;",
        breaks: "retirement leaves the service worker asleep beside a zombie",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        id: 349,
        file: "kernel/src/handle.rs",
        find: "        let successor = handle_space::next_table_identity(self.next_table)\n            .ok_or(HandleError::IdentityExhausted)?;",
        replace: "        let successor = self.next_table.wrapping_add(1);",
        breaks: "handle-table construction crosses the exhausted identity boundary",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        id: 350,
        file: "kernel/src/handle.rs",
        find: "        self.next_table = successor;",
        replace: "        if successor != u64::MAX {\n            self.next_table = successor;\n        }",
        breaks: "two simultaneously live handle tables receive one ownership identity",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        id: 407,
        file: "kernel/src/panic.rs",
        find: "        let _ = write_record(uart, thread, info);",
        replace: "        let _ = writeln!(uart, \"[error] KERNEL PANIC on thread '{thread}': {info}\");",
        breaks: "the kernel writes PanicInfo bytes around the single-line adapter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 412,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "        if !self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY) {\n            return Err(fmt::Error);\n        }",
        replace: "        let _ = self.wait_for(line_status::TRANSMIT_HOLDING_EMPTY);",
        breaks: "the UART reports a refused byte as transmitted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 413,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "                self.write_byte(b'\\r')?;",
        replace: "                let _ = self.write_byte(b'\\r');",
        breaks: "the UART formatter hides a refused carriage return",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 414,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "            self.write_byte(byte)?;",
        replace: "            let _ = self.write_byte(byte);",
        breaks: "the UART formatter hides a refused payload byte",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_uart_reports_every_refused_byte_to_its_formatter",
        },
    },
    Mutation {
        id: 491,
        file: "kernel/src/queue.rs",
        find: "        let tail = ring.protocol.completions().tail();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        replace: "        let tail = ring.protocol.completions().tail();\n        drop(ring);\n        let mut ring = self.ring.lock_leaf();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        breaks: "a later kernel completion can publish past an unwritten earlier slot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::kernel_completion_producers_publish_in_tail_order",
        },
    },
    Mutation {
        id: 492,
        file: "kernel/src/handle.rs",
        find: "        if entry.owner != self.id {\n            return Err(HandleError::Stale);\n        }",
        replace: "",
        breaks: "a resolved handle cannot cross its ownership table",
        reddens: Caught::Check("a handle reveals no foreign occupancy"),
    },
    Mutation {
        id: 501,
        file: "kernel/src/time.rs",
        find: "            if failures >= WATCHDOG_FAILURES {",
        replace: "            if failures > WATCHDOG_FAILURES {",
        breaks: "the required watchdog failures revoke the live timing source",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        id: 502,
        file: "kernel/src/timepage.rs",
        find: "    DESIRED_MODE.store(mode, Ordering::Release);",
        replace: "    DESIRED_MODE.store(MODE_COUNTER, Ordering::Release);",
        breaks: "clock revocation survives the time page's temporary mapping absence",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    Mutation {
        id: 503,
        file: "kernel/src/time.rs",
        find: "        Some(now.saturating_add(elapsed))",
        replace: "        Some(now.saturating_add(elapsed.saturating_sub(elapsed)))",
        breaks: "the retained PIT advances scheduler time after public-clock revocation",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        id: 524,
        file: "kernel/src/queue.rs",
        find: "            if !entry.settling && wanted(*entry) {",
        replace: "            if wanted(*entry) {",
        breaks: "two terminal paths can claim one pending operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_terminal_path_claims_one_settlement_until_publication",
        },
    },
    Mutation {
        id: 531,
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-baseline\", 8, MEASURE)",
        replace: "    spawn(\"user-baseline\", 8, 0)",
        breaks: "the ring boundary baseline never enables its timestamp witness",
        reddens: Caught::Check("a ring enter round trip is measurable"),
    },
    Mutation {
        id: 532,
        file: "kernel/src/nvme.rs",
        find: "        self.read_blocks(0, blocks, into).ok()?;",
        replace: "        self.read_blocks(self.blocks, blocks, into).ok()?;",
        breaks: "the storage baseline never submits inside the namespace",
        reddens: Caught::Check("a full-buffer nvme read is measurable"),
    },
    Mutation {
        id: 561,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        self.write_config(value)\n            .then_some(())\n            .ok_or(I8042Error::ConfigurationRefused)\n    }",
        replace: "    fn require_config(&mut self, value: u8) -> Result<(), I8042Error> {\n        let _ = self.write_config(value);\n        Ok(())\n    }",
        breaks: "a required controller configuration refusal is silently accepted",
        reddens: Caught::Check(
            "a refused controller configuration is typed and changes nothing",
        ),
    },
    Mutation {
        id: 562,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        if !Self::flush(self) {\n            return Err(InterruptEnableError::Unavailable);\n        }",
        replace: "    pub(crate) fn enable_interrupt(&mut self) -> Result<(), InterruptEnableError> {\n        let _ = Self::flush(self);",
        breaks: "an i8042 transaction proceeds through stale undrained output",
        reddens: Caught::Check(
            "a failed controller flush stops every following transaction",
        ),
    },
    Mutation {
        id: 563,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        if !self.controller_command(command) {\n            return None;\n        }\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        replace: "    fn controller_query(&mut self, command: u8) -> Option<u8> {\n        let _ = self.controller_command(command);\n        self.read_reply(RESPONSE_ATTEMPTS)\n    }",
        breaks: "a refused i8042 command consumes an unrelated reply byte",
        reddens: Caught::Check(
            "a refused controller command is typed and consumes no reply",
        ),
    },
    Mutation {
        id: 565,
        file: "kernel/src/main.rs",
        find: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {\n    assert!(\n        wost::arch::current::port::pci_configuration().is_some(),\n        \"the degraded boot could not reserve the PCI configuration ports\"\n    );\n}",
        replace: "#[cfg(feature = \"refuse-pci\")]\nfn refuse_pci_for_degraded_boot() {}",
        breaks: "the degraded image silently retains live PCI configuration ports",
        reddens: Caught::BootMarker(qemu::NO_PCI_HANDOFF_PANIC),
    },
    Mutation {
        id: 566,
        file: "kernel/src/queue.rs",
        find: "            assert!(posted, \"a reserved completion slot was not there\");",
        replace: "            debug_assert!(posted, \"a reserved completion slot was not there\");",
        breaks: "a missing reserved completion wedges only release kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 567,
        file: "kernel/src/main.rs",
        find: "cpu: extended state unavailable; ring 3 stays closed",
        replace: "cpu: extended state unavailable",
        breaks: "the degraded CPU no longer proves the extended-state veto it exercised",
        reddens: Caught::BootMarker(qemu::DEGRADED_XSTATE_MARKER),
    },
    Mutation {
        id: 568,
        file: "kernel/src/user.rs",
        find: "maskable interrupts are not admitted; ring 3 is closed",
        replace: "interrupt admission unavailable; ring 3 is closed",
        breaks: "the degraded boot loses the exact first user-boundary refusal",
        reddens: Caught::BootMarker(qemu::DEGRADED_USER_MARKER),
    },
    Mutation {
        id: 569,
        file: "kernel/src/time.rs",
        find: "        Err(apic::TimerError::Unavailable)\n            if !Timebase::from_bits(NANOSECONDS_TO_APIC.load(Ordering::Relaxed))\n                .is_calibrated() => {}\n",
        replace: "",
        breaks: "a boot with no admitted APIC timer panics while cleaning up idle scheduler state",
        reddens: Caught::DegradedPanic {
            marker: "the local timer would not stop: the local apic timer is unavailable",
            why: "the no-x2APIC boot reaches scheduler cleanup before any local timer can be admitted, so this exact terminal refusal is the defect",
        },
    },
    Mutation {
        id: 570,
        file: "kernel/src/handle.rs",
        find: "        assert!(committed, \"a live reservation refused quota commit\");",
        replace: "        debug_assert!(committed, \"a live reservation refused quota commit\");",
        breaks: "a broken handle commitment is terminal only in debug kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 573,
        file: "kernel/src/user.rs",
        find: "    assert!(answered, \"the live syscall frame could not be answered\");",
        replace: "    debug_assert!(answered, \"the live syscall frame could not be answered\");",
        breaks: "a release syscall can return with an unanswered stale register frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 574,
        file: "kernel/src/nvme.rs",
        find: "            self.stop_dma(Some(data));",
        replace: "            let _ = self.fail_submitted(BringUpError::NotReady, Some(data));",
        breaks: "NVMe self-test repair hides cleanup behind a guaranteed error",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        id: 592,
        file: "kernel/src/user.rs",
        find: "            Some((process, bound)) => {\n                prepared = Some(process);\n                Err(bound)\n            }",
        replace: "            Some((process, bound)) => {\n                core::mem::forget(process);\n                Err(bound)\n            }",
        breaks: "a commit refusal loses the still-bound process owner",
        reddens: Caught::Check("a commit refusal reunites cleanup authority"),
    },
    Mutation {
        id: 594,
        file: "kernel/src/selftest.rs",
        find: "    paging::refuse_mapping_after_for_test(1);\n    let result = space.map_owned(BASE, run, flags);",
        replace: "    paging::refuse_mapping_after_for_test(1);\n    paging::refuse_mapping_undo_after_for_test(0);\n    let result = space.map_owned(BASE, run, flags);",
        breaks: "a rollback walk refusal returns ordinary failure and frees reachable frames",
        reddens: Caught::Panic {
            marker: "mapping rollback lost a written leaf: injected refusal",
            why: "an incomplete leaf rollback cannot return the still-addressable segment to the allocator, so the contradiction is terminal before ordinary failure escapes",
        },
    },
    Mutation {
        id: 595,
        file: "kernel/src/handle.rs",
        find: "        let committed = self.table.step(Quota::commit);\n        assert!(committed, \"a live reservation refused quota commit\");\n        self.spent = true;",
        replace: "        self.spent = true;",
        breaks: "a kept handle discards its promised quota transition",
        reddens: Caught::Check("a completion carries a handle with narrowed rights"),
    },
    Mutation {
        id: 601,
        file: "kernel/src/user.rs",
        find: "        if !at.is_multiple_of(PAGE_SIZE) {\n            return Err(LoadError::Unaligned(at));\n        }",
        replace: "",
        breaks: "the loader reaches page mapping with an unaligned segment address",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    Mutation {
        id: 602,
        file: "kernel/src/user.rs",
        find: "    bootstrap.abandoned = bootstrap\n        .abandoned\n        .checked_add(abandoned)\n        .unwrap_or_else(|| panic!(\"zombie abandonment accounting overflowed\"));",
        replace: "    bootstrap.abandoned = bootstrap.abandoned.saturating_add(abandoned);",
        breaks: "zombie abandonment accounting silently saturates",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        id: 605,
        file: "kernel/src/input.rs",
        find: "fn record_interrupt_handoff(latch: &Once) {\n    assert!(\n        latch.claim(),\n        \"the keyboard interrupt handoff happened more than once\"\n    );\n}",
        replace: "fn record_interrupt_handoff(latch: &Once) {\n    let _ = latch;\n}",
        breaks: "the routed keyboard never records its interrupt-driven handoff",
        reddens: Caught::Check("the keyboard interrupt handoff records its ownership"),
    },
    Mutation {
        id: 606,
        file: "kernel/src/time.rs",
        find: "    assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        replace: "    debug_assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        breaks: "release scheduler-time accounting stops proving its unconditional update",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 607,
        file: "kernel/src/main.rs",
        find: "        assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        replace: "        debug_assert_eq!(\n            timepage::publish(mode, time::origin_ticks(), time::conversion()),",
        breaks: "release boot silently accepts refusal from an initialized time page",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 609,
        file: "kernel/src/user.rs",
        find: "        crate::UserIsolation::Closed => return Err(SpawnError::NoUserIsolation),",
        replace: "        crate::UserIsolation::Closed => AddressSpace::for_user(),",
        breaks: "ring three opens without an admitted shared or paired isolation mode",
        reddens: Caught::Check(
            "shared kernel mappings without an isolation proof are refused",
        ),
    },
    Mutation {
        id: 683,
        file: "kernel/src/queue.rs",
        find: "                && current == self.expected,",
        replace: "                && current != self.expected,",
        breaks: "refused file work can roll back a queued operation it no longer owns",
        reddens: Caught::Panic {
            marker: "refused file work no longer owns its queued operation",
            why: "the refusal path has no authority to remove a different pending operation, so an ownership mismatch must stop before mutating the queue",
        },
    },
    Mutation {
        id: 684,
        file: "kernel/src/queue.rs",
        find: "                !entry.settling && *entry == self.work.expected,",
        replace: "                entry.settling && *entry == self.work.expected,",
        breaks: "running file work can start after its pending entry began settling",
        reddens: Caught::Panic {
            marker: "running file work no longer owns its operation",
            why: "a settling entry has already transferred terminal authority, so starting its operation would let two paths consume the same queued work",
        },
    },
    Mutation {
        id: 685,
        file: "kernel/src/queue.rs",
        find: "            entry.settling && current == entry,",
        replace: "            !entry.settling && current == entry,",
        breaks: "terminal file work can clear a pending entry before settlement owns it",
        reddens: Caught::Panic {
            marker: "terminal work no longer owns its pending entry",
            why: "only the exact settling snapshot carries terminal removal authority, so clearing any other state would discard or replace live work",
        },
    },
    // The forged head keeps every other empty-ring relation valid (D-396).
    Mutation {
        id: 686,
        file: "kernel/src/queue.rs",
        find: "        if self.head >= WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        replace: "        if self.head > WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        breaks: "a work queue accepts its exclusive head bound as an index",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 687,
        file: "kernel/src/queue.rs",
        find: "            pending.checked_add(1)",
        replace: "            Some(pending.wrapping_add(1))",
        breaks: "reap request admission wraps its outstanding count through zero",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 688,
        file: "kernel/src/queue.rs",
        find: "            pending.checked_sub(1)",
        replace: "            Some(pending.wrapping_sub(1))",
        breaks: "an empty reap request counter wraps to the maximum",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 712,
        file: "kernel/src/user.rs",
        find: "        return if another_collector {
            CollectOutcome::Retained
        } else {
            CollectOutcome::Empty
        };",
        replace: "        return if another_collector {
            CollectOutcome::Empty
        } else {
            CollectOutcome::Empty
        };",
        breaks: "an in-flight collector can lose the worker retry that owns its progress",
        reddens: Caught::Check("an in-flight collector keeps the worker retry live"),
    },
    Mutation {
        id: 716,
        file: "kernel/src/user.rs",
        find: "            if zombies.settling[slot] {\n                return None;\n            }",
        replace: "            if false && zombies.settling[slot] {\n                return None;\n            }",
        breaks: "the same zombie can acquire a second settlement collector",
        reddens: Caught::Timeout {
            seconds: 60,
            why: "the duplicate settlement collector prevents zombie ownership from converging, \
                  so the self-test cannot reach a terminal verdict",
        },
    },
    Mutation {
        id: 721,
        file: "kernel/src/user.rs",
        find: "    if !crate::maskable_interrupts_are_ready() {\n        warn!(\"user {which}: maskable interrupts are not admitted\");\n        refuse_cross(slot);\n    }",
        replace: "    if false {\n        warn!(\"user {which}: maskable interrupts are not admitted\");\n        refuse_cross(slot);\n    }",
        breaks: "aged interrupt-readiness evidence can cross the final user-transition door",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        id: 723,
        file: "kernel/src/user.rs",
        find: "fn refuse_cross(slot: usize) -> ! {\n    let _disabled = interrupt::disable();\n    orphan(slot);\n    thread::exit()\n}",
        replace: "fn refuse_cross(slot: usize) -> ! {\n    let _disabled = interrupt::disable();\n    let _ = slot;\n    thread::exit()\n}",
        breaks: "a final user-transition refusal can abandon its committed process owner",
        reddens: Caught::Check("unadmitted interrupts cannot be enabled by a user transition"),
    },
    Mutation {
        id: 772,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }",
        replace: "                if let Some(eoi) = eoi {\n                    drop(eoi);\n                }",
        breaks: "the ping interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::Check("the interrupt is dismissed, so the next one arrives"),
    },
    Mutation {
        id: 773,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "    drop(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        replace: "    core::mem::forget(eoi);\n    let reclaimed = apic::claim_eoi(frame, vector::PING);",
        breaks: "abandoning an EOI token can permanently strand its live-claim latch",
        reddens: Caught::Panic {
            marker: "interrupts became enabled inside an exclusion",
            why: "forgetting the EOI token strands its interrupt exclusion, so trap return \
                  re-enables interrupts inside that exclusion before the next claim",
        },
    },
    Mutation {
        id: 775,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            dismiss(eoi);\n            crate::thread::reschedule(frame);",
        replace: "            drop(eoi);\n            crate::thread::reschedule(frame);",
        breaks: "the timer interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 776,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            let woke = crate::input::on_interrupt();\n            dismiss(eoi);",
        replace: "            let woke = crate::input::on_interrupt();\n            drop(eoi);",
        breaks: "the keyboard interrupt arm drops its completion authority without issuing EOI",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 786,
        file: "kernel/src/selftest.rs",
        find: "    let raw = an_admitted_canonical_page_can_be_invalidated as *const () as u64;",
        replace: "    let raw = 0x0000_8000_0000_0000;",
        breaks: "the production page-invalidation wrapper receives a noncanonical raw operand",
        reddens: Caught::Check("an admitted canonical page can be invalidated"),
    },
    Mutation {
        id: 829,
        file: "kernel/src/selftest.rs",
        find: "    let written = xstate::write_probe(REUSED_SLOT_SECRET);",
        replace: "    let written = false;",
        breaks: "the task-slot reuse witness accepts a secret that was never planted",
        reddens: Caught::Check("an exited slot is reused"),
    },
    Mutation {
        id: 836,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        write(&mut self.registers, low, entry::MASKED)?;\n        let (observed_low, observed_high) = self.entry_words(line)?;\n        if observed_low != entry::MASKED {\n            return Err(MaskError::Refused {\n                low: observed_low,\n                high: observed_high,\n            });\n        }\n        write(&mut self.registers, low + 1, 0)?;",
        replace: "        write(&mut self.registers, low, entry::MASKED)?;\n        write(&mut self.registers, low + 1, 0)?;",
        breaks: "an unproven takeover mask permits a destination write",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 837,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        if low != entry::MASKED || high != 0 {\n            return Err(MaskError::Refused { low, high });\n        }",
        replace: "        let _ = (low, high);",
        breaks: "an ignored takeover destination clear is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 838,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "            Ok((low, high)) if low == expected_low && high == destination => Ok(()),\n            Ok((low, high)) => self.refuse_route(line, expected_low, destination, low, high),",
        replace: "            Ok((_low, _high)) => Ok(()),",
        breaks: "an ignored unmask write is accepted",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 839,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        match self.entry_words(line) {\n            Ok((low, high)) if low == entry::MASKED && high == destination => {}\n            Ok((low, high)) => {\n                return self.refuse_route(line, expected_low, destination, low, high);\n            }\n            Err(error) => {\n                self.mask_line_and_verify(line)\n                    .map_err(RoutingError::from)?;\n                return Err(RoutingError::Registers(error));\n            }\n        }",
        replace: "        let _ = self.entry_words(line);",
        breaks: "an IOAPIC destination is unmasked before its readback",
        reddens: Caught::Check(
            "a refused keyboard route restores configuration, masking and mappings",
        ),
    },
    Mutation {
        id: 840,
        file: "kernel/src/pci.rs",
        find: "        Ok(()) => release_clean_function(function, reason),",
        replace: "        Ok(()) => {\n            drop(function);\n            reason\n        },",
        breaks: "a clean post-claim PCI refusal releases its function authority",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    Mutation {
        id: 928,
        file: "kernel/src/pci.rs",
        find: "fn claim_as(at: Address, lifetime: ClaimLifetime) -> Result<Device, ClaimError> {",
        replace: "fn claim_as(at: Address, lifetime: bool) -> Result<Device, ClaimError> {",
        breaks: "PCI claim lifetime becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 929,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {",
        replace: "fn start_interrupts(external_controllers: bool) {",
        breaks: "interrupt startup receives controller-mask proof as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 930,
        file: "kernel/src/main.rs",
        find: "const fn external_interrupts_are_admitted(mask: Option<&mem::ExternalControllersMasked>) -> bool {",
        replace: "const fn external_interrupts_are_admitted(mask: bool) -> bool {",
        breaks: "external-interrupt admission receives controller state as an unnamed boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 931,
        file: "kernel/src/main.rs",
        find: "fn route_keyboard(routing: Option<mem::ExternalControllersMasked>) {",
        replace: "fn route_keyboard(routing: Option<bool>) {",
        breaks: "keyboard routing receives unnamed boolean external-controller authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 932,
        file: "kernel/src/input.rs",
        find: "pub(crate) fn set_translation(translation: crate::arch::current::i8042::Translation) -> bool {",
        replace: "pub(crate) fn set_translation(translation: bool) -> bool {",
        breaks: "input translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 933,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "    pub(crate) fn set_translation(&mut self, translation: Translation) -> bool {",
        replace: "    pub(crate) fn set_translation(&mut self, translation: bool) -> bool {",
        breaks: "i8042 translation policy becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 934,
        file: "kernel/src/nvme.rs",
        find: "    wanted: Readiness,",
        replace: "    wanted: bool,",
        breaks: "NVMe ready-state polling becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 935,
        file: "kernel/src/thread.rs",
        find: "    fn choose(&self, current: usize, eligibility: CurrentEligibility) -> Option<usize> {",
        replace: "    fn choose(&self, current: usize, eligibility: bool) -> Option<usize> {",
        breaks: "scheduler current-thread eligibility becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 936,
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn show_cursor(&mut self, _visible: bool) {",
        breaks: "cursor display regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 937,
        file: "kernel/src/console.rs",
        find: "    fn hide_cursor(&mut self) {",
        replace: "    fn hide_cursor(&mut self, _visible: bool) {",
        breaks: "cursor hiding regains a redundant unnamed visibility boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 938,
        file: "kernel/src/console.rs",
        find: "    fn show_cursor(&mut self) {",
        replace: "    fn set_cursor(&mut self, visible: bool) {\n        let colour = if visible {\n            self.palette.foreground\n        } else {\n            self.palette.background\n        };\n        self.paint_cursor(colour);\n    }\n\n    fn show_cursor(&mut self) {",
        breaks: "cursor visibility regrows one boolean-shaped operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 939,
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Clone)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "external-controller mask authority can be duplicated after one hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 940,
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}",
        replace: "pub(crate) struct ExternalControllersMasked {\n    pub(crate) routing: Option<Routing>,\n}",
        breaks: "kernel peers can forge external-controller mask authority through a visible field",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 941,
        file: "kernel/src/mem.rs",
        find: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nstruct ControllersReleased {",
        replace: "pub(crate) struct ExternalControllersMasked {\n    routing: Option<Routing>,\n}\n\nuse self::ExternalControllersMasked as ForgedMask;\n\nstruct ControllersReleased {",
        breaks: "external-controller mask authority gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 942,
        file: "kernel/src/mem.rs",
        find: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}",
        replace: "impl ExternalControllersMasked {\n    /// The selected keyboard controller, when the proven inventory has one.\n    pub(crate) fn into_routing(self) -> Option<Routing> {\n        self.routing\n    }\n}\n\nfn forged_mask_authority() -> ExternalControllersMasked {\n    ExternalControllersMasked { routing: None }\n}",
        breaks: "external-controller mask authority gains a mint outside its checked transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 943,
        file: "kernel/src/mem.rs",
        find: "            controllers_masked: false,",
        replace: "            controllers_masked: true,",
        breaks: "an unproven external-controller inventory manufactures mask authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 944,
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}",
        replace: "#[derive(Clone)]\nstruct ControllersReleased {\n    _private: (),\n}",
        breaks: "one exact controller cleanup can duplicate its release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 945,
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "impl RoutingPreparation {\n    const fn polled_without_release() -> Self {\n        Self {\n            routing: None,\n            controllers_masked: true,\n        }\n    }\n\n    fn ready(routing: Routing) -> Self {",
        breaks: "polled routing can manufacture mask authority without controller-release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 946,
        file: "kernel/src/mem.rs",
        find: "impl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        replace: "fn forged_routing_preparation() -> RoutingPreparation {\n    RoutingPreparation {\n        routing: None,\n        controllers_masked: true,\n    }\n}\n\nimpl RoutingPreparation {\n    fn ready(routing: Routing) -> Self {",
        breaks: "routing preparation gains a raw constructor outside its audited transitions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 947,
        file: "kernel/src/mem.rs",
        find: "struct ControllersReleased {\n    _private: (),\n}\n\nimpl RoutingPreparation {",
        replace: "struct ControllersReleased {\n    _private: (),\n}\n\nfn forged_controller_release() -> ControllersReleased {\n    ControllersReleased { _private: () }\n}\n\nimpl RoutingPreparation {",
        breaks: "controller-release evidence gains a mint outside exact cleanup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 948,
        file: "kernel/src/mem.rs",
        find: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return None;",
        replace: "            retain_routing_registers(registers, core::mem::take(controllers));\n            return Some(ControllersReleased { _private: () });",
        breaks: "failed controller cleanup still mints release evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 949,
        file: "kernel/src/mem.rs",
        find: "            warn!(\"ioapic: candidate mapping remained owned during cleanup\");\n            retain_routing_registers(registers, core::mem::take(controllers));",
        replace: "            assert!(controllers.is_empty());\n            retain_routing_registers(registers, core::mem::take(controllers));",
        breaks: "routing cleanup can panic before retaining a refused device owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 950,
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if true => routing,",
        breaks: "keyboard routing proceeds without observing enabled maskable interrupts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 951,
        file: "kernel/src/main.rs",
        find: "        Some(routing) if interrupt::are_enabled() => routing,",
        replace: "        Some(routing) if { let _ = interrupt::are_enabled(); true } => routing,",
        breaks: "keyboard routing observes interrupt state but ignores its value",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 952,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn enable_before_mask_proof() {\n    interrupt::enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    enable_before_mask_proof();\n    if let Err(reason) = pic::silence() {",
        breaks: "an interrupt-enable helper bypasses external-controller mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 953,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "fn retain_interrupt_enable() {\n    let _ = interrupt::enable;\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an indirect function reference outside its audited calls",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 954,
        file: "kernel/src/main.rs",
        find: "fn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        replace: "use wost::arch::current::interrupt::enable as alias_enable;\n\nfn call_aliased_interrupt_enable() {\n    alias_enable();\n}\n\nfn start_interrupts(external_controllers: Option<&mem::ExternalControllersMasked>) {\n    if let Err(reason) = pic::silence() {",
        breaks: "interrupt enable gains an imported alias outside mask admission",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 955,
        file: "kernel/src/main.rs",
        find: "    mask.is_some()",
        replace: "    mask.is_none()",
        breaks: "missing external-controller mask authority is admitted while real authority is refused",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::interrupt_enable_and_routing_require_the_mask_authority",
        },
    },
    Mutation {
        id: 958,
        file: "kernel/src/nvme.rs",
        find: "    wait_for_ready(registers, *capabilities, Readiness::NotReady)?;",
        replace: "    wait_for_ready(registers, *capabilities, Readiness::Ready)?;",
        breaks: "controller-stop evidence is minted after observing the ready bit set",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 959,
        file: "kernel/src/nvme.rs",
        find: "    let cc = read(registers, register::CC)?;",
        replace: "    let cc = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh configuration read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 960,
        file: "kernel/src/nvme.rs",
        find: "    let csts = read(registers, register::CSTS)?;",
        replace: "    let csts = 0;",
        breaks: "controller-stop evidence substitutes a constant for its fresh status read",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 961,
        file: "kernel/src/nvme.rs",
        find: "    let observed = DisabledRegisters::from_bits(cc, csts).map_err(|_| BringUpError::NotReady)?;",
        replace: "    let observed = DisabledRegisters::from_bits(0, 0).map_err(|_| BringUpError::NotReady)?;",
        breaks: "controller-stop evidence classifies constants instead of its fresh register pair",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 962,
        file: "kernel/src/nvme.rs",
        find: "            self.dma.disposition = DmaDisposition::Quiesced;",
        replace: "            self.dma.disposition = DmaDisposition::Retained;",
        breaks: "proven controller-stop evidence retains recyclable DMA ownership",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 963,
        file: "kernel/src/nvme.rs",
        find: "    dma: &'controller mut DmaState,",
        replace: "    dma: &'controller DmaState,",
        breaks: "controller-stop evidence loses mutable authority over the exact DMA state it retires",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 964,
        file: "kernel/src/nvme.rs",
        find: "    fn quiesce(self) {",
        replace: "    fn quiesce(&mut self) {",
        breaks: "one controller-stop observation can authorize more than one DMA recycle transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 965,
        file: "kernel/src/nvme.rs",
        find: r#"    Ok(ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    })
}"#,
        replace: r#"    Ok(mint_stopped(registers, observed, dma))
}

fn mint_stopped<'controller>(
    registers: &'controller MappedBar,
    observed: DisabledRegisters,
    dma: &'controller mut DmaState,
) -> ControllerStopped<'controller> {
    ControllerStopped {
        _registers: registers,
        _observed: observed,
        dma,
    }
}"#,
        breaks: "stop evidence can bind a hardware observation and DMA state from different controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 969,
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}",
        replace: "#[derive(Debug)]\nstruct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nconst _: () = {\n    impl Clone for ControllerStopped<'_> {\n        fn clone(&self) -> Self {\n            unreachable!()\n        }\n    }\n};",
        breaks: "a nested trait implementation can duplicate one observed controller stop into multiple recycle authorities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 970,
        file: "kernel/src/nvme.rs",
        find: "#[derive(Debug, PartialEq, Eq)]\nenum DmaDisposition {",
        replace: "#[derive(Debug, Clone, Copy, PartialEq, Eq)]\nenum DmaDisposition {",
        breaks: "a controller's DMA retirement disposition becomes freely copyable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 971,
        file: "kernel/src/nvme.rs",
        find: "impl ControllerStopped<'_> {\n    fn quiesce(self) {",
        replace: "impl ControllerStopped<'_> {\n    fn duplicate(self) -> Self {\n        self\n    }\n\n    fn quiesce(self) {",
        breaks: "controller-stop evidence gains an extra inherent transfer path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 972,
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\ntype ControllerStoppedAlias<'controller> = ControllerStopped<'controller>;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains a constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 973,
        file: "kernel/src/nvme.rs",
        find: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nimpl ControllerStopped<'_> {",
        replace: "struct ControllerStopped<'controller> {\n    _registers: &'controller MappedBar,\n    _observed: DisabledRegisters,\n    dma: &'controller mut DmaState,\n}\n\nuse self::ControllerStopped as ForgedStopped;\n\nimpl ControllerStopped<'_> {",
        breaks: "controller-stop evidence gains an imported constructor alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_evidence_has_one_hardware_mint",
        },
    },
    Mutation {
        id: 974,
        file: "kernel/src/nvme.rs",
        find: "    const fn is_settled(&self) -> bool {\n        !matches!(&self.disposition, DmaDisposition::Active)\n    }",
        replace: "    const fn is_settled(&self) -> bool {\n        true\n    }",
        breaks: "active DMA is reported settled before hardware quiescence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 975,
        file: "kernel/src/nvme.rs",
        find: "        if self.dma.disposition == DmaDisposition::Active {",
        replace: "        if true {",
        breaks: "consumed stop evidence can rewrite an already terminal DMA disposition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::proven_stop_consumes_evidence_before_recycling",
        },
    },
    Mutation {
        id: 976,
        file: "kernel/src/nvme.rs",
        find: "        if self.dma.must_retain() {\n            self.admin.retain();\n            if let Some(queue) = self.io.as_mut() {\n                queue.retain();\n            }\n            if let Some(segment) = data {\n                segment.retain();\n            }\n        }",
        replace: "",
        breaks: "a failed controller stop retains none of its reachable DMA runs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 977,
        file: "kernel/src/nvme.rs",
        find: "    fn retain(&mut self) {\n        self.submission.retain();\n        self.completion.retain();\n    }",
        replace: "    fn retain(&mut self) {\n        self.submission.retain();\n    }",
        breaks: "failed controller cleanup recycles the completion queue while hardware may reach it",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 978,
        file: "kernel/src/nvme.rs",
        find: "        if !self.dma.is_quiesced() {\n            return quarantine_controller(self);\n        }",
        replace: "        if !self.dma.is_quiesced() {\n            return ControllerRetirement::Retired;\n        }",
        breaks: "an unverified controller stop drops its complete quarantine owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        id: 979,
        file: "kernel/src/nvme.rs",
        find: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::memory_quarantine(reason),",
        replace: "        mem::MappedPciBarRetirement::Quarantined => PrepareFailure::Retired(reason),",
        breaks: "memory-owned BAR quarantine is misreported as successful retirement",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::mapped_bar_cleanup_quarantine_cannot_be_reported_retired",
        },
    },
    Mutation {
        id: 980,
        file: "kernel/src/nvme.rs",
        find: "    *retained = Some(RetainedController {\n        _controller: controller,\n        device: None,\n    });",
        replace: "    drop(controller);",
        breaks: "controller quarantine mints its token after dropping the controller owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_retirement_returns_every_cleanup_owner",
        },
    },
    Mutation {
        id: 981,
        file: "kernel/src/nvme.rs",
        find: "impl Drop for Controller {\n    fn drop(&mut self) {\n        self.stop_dma(None);\n    }\n}",
        replace: "impl Drop for Controller {\n    fn drop(&mut self) {}\n}",
        breaks: "dropping a controller skips the final DMA stop-or-retain boundary",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::failed_stop_retention_and_retirement_sinks_are_exact",
        },
    },
    Mutation {
        id: 982,
        file: "kernel/src/nvme.rs",
        find: "        if observed == wanted {",
        replace: "        if observed == wanted || wanted == Readiness::NotReady {",
        breaks: "a disable wait succeeds before observing the controller ready bit clear",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        id: 983,
        file: "kernel/src/nvme.rs",
        find: "        let observed = if bits & status::READY != 0 {",
        replace: "        let observed = if bits & status::READY == 0 {",
        breaks: "the readiness poll reverses the controller's live ready-bit observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::controller_stop_waits_for_the_requested_hardware_state",
        },
    },
    Mutation {
        id: 984,
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn swap_dma_states(left: &mut Controller, right: &mut Controller) {\n    core::mem::swap(&mut left.dma, &mut right.dma);\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA retirement state can be swapped between two controllers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 985,
        file: "kernel/src/nvme.rs",
        find: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        replace: "    const fn must_retain(&self) -> bool {\n        matches!(&self.disposition, DmaDisposition::Retained)\n    }\n}\n\nfn alias_dma_state(controller: &Controller) {\n    let _ = &controller.dma;\n}\n\n/// A mapped controller observed disabled while PCI bus mastering was off.",
        breaks: "DMA state gains an unaudited whole-state access alias",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dma_disposition_cannot_move_between_controllers",
        },
    },
    Mutation {
        id: 987,
        file: "kernel/src/queue.rs",
        find: "            WORKER_IS_PAUSED.store(true, Ordering::Release);",
        replace: "            WORKER_IS_PAUSED.store(false, Ordering::Release);",
        breaks: "the queue worker never acknowledges its release-race pause",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 993,
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 2048;\n\nfn with_volume() {}",
        breaks: "filesystem volume ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 994,
        file: "kernel/src/nvme.rs",
        find: "const COMMAND_TIMEOUT_MS: u64 = 5_000;",
        replace: "const COMMAND_TIMEOUT_MS: u64 = 5_000;\n\nfn with_controller() {}",
        breaks: "NVMe controller ownership regrows a callback authority door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 995,
        file: "kernel/src/tmpfs.rs",
        find: "pub(crate) const ROOT: usize = 0;",
        replace: "pub(crate) const ROOT: usize = 0;\n\nfn walk() {}",
        breaks: "tmpfs regrows a speculative internal pathname walker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1061,
        file: "kernel/src/mem.rs",
        find: "#[derive(Debug)]\npub(crate) struct ExternalControllersMasked {",
        replace: "#[derive(Debug, Default)]\npub(crate) struct ExternalControllersMasked {",
        breaks: "default construction forges external-controller mask authority without a hardware proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "routing_contracts::external_controller_mask_authority_is_private_linear_and_exact",
        },
    },
    Mutation {
        id: 1170,
        file: "kernel/src/user.rs",
        find: "    xstate::user_state_is_isolated()",
        replace: "    xstate::is_enabled()",
        breaks: "ring three trusts remembered extended-state enablement instead of the live isolation predicate",
        reddens: Caught::Check("missing extended state closes the user boundary"),
    },
    Mutation {
        id: 1256,
        file: "kernel/src/selftest.rs",
        find: r#"            error!("selftest: {name} FAILED");"#,
        replace: r#"            warn!("selftest: {name} FAILED");"#,
        breaks: "the failed-check producer emits the wrong serial severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1257,
        file: "kernel/src/selftest.rs",
        find: r#"            "selftest: verdict v{} PASS passed={} failed={} expected={}","#,
        replace: r#"            "selftest: verdict v{} OK passed={} failed={} expected={}","#,
        breaks: "the kernel PASS verdict producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1258,
        file: "kernel/src/selftest.rs",
        find: r#"            "selftest: verdict v{} FAIL passed={} failed={} expected={}","#,
        replace: r#"            "selftest: verdict v{} ERROR passed={} failed={} expected={}","#,
        breaks: "the kernel FAIL verdict producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1259,
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: baseline v1 boot_ns={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}","#,
        replace: r#"        "selftest: baseline v1 boot={} frames_free={} heap_slab={} heap_large={} heap_live={} handles_committed={} handles_live={} trap_ticks={} storage_bytes={} storage_ticks={} counter_hz={}","#,
        breaks: "the kernel performance-baseline producer drifts from the parser contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1260,
        file: "kernel/src/selftest.rs",
        find: r#"fn awaiting(marker: &str, press: &str) {
    info!("{marker}");"#,
        replace: r#"fn awaiting(marker: &str, press: &str) {
    warn!("{marker}");"#,
        breaks: "the shared interactive marker producer emits the wrong serial severity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1261,
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting storage-delay keystroke","#,
        replace: r#"        "selftest: awaiting storage delay keystroke","#,
        breaks: "the storage-delay producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1262,
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting injected keystroke","#,
        replace: r#"        "selftest: awaiting injected key","#,
        breaks: "the injected-keystroke producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1263,
        file: "kernel/src/selftest.rs",
        find: r#"        "selftest: awaiting translated keystroke","#,
        replace: r#"        "selftest: awaiting translated key","#,
        breaks: "the translated-keystroke producer literal drifts from its injection enrollment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1271,
        file: "kernel/src/arch/x86_64/vector.rs",
        find: "pub(crate) const SYSCALL: InterruptVector = SOURCES[4];",
        replace: "pub(crate) const SYSCALL: InterruptVector = SOURCES[3];",
        breaks: "the syscall gate aliases the local APIC timer's assigned vector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        id: 1272,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "    fn route_line(\n",
        replace: "    #[allow(dead_code)]\n    fn route_raw_for_mutation(\n        &mut self,\n        line: u32,\n        vector: u8,\n        wiring: Wiring,\n        destination: u32,\n    ) -> Result<(), RoutingError> {\n        let Some(vector) = InterruptVector::new(vector) else {\n            return Ok(());\n        };\n        self.route_line(line, vector, wiring, destination)\n    }\n\n    fn route_line(\n",
        breaks: "an I/O APIC programming door recovers a raw vector parameter",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::one_assignment_and_typed_signatures_own_every_vector_door",
        },
    },
    Mutation {
        id: 1273,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        && controller.registers.low == u32::from(super::vector::KEYBOARD.get())",
        replace: "        && controller.registers.low == u32::from(super::vector::TIMER.get())",
        breaks: "the isolated I/O APIC route accepts another source's encoded vector",
        reddens: Caught::Check("an isolated I/O APIC route encodes its typed vector"),
    },
    Mutation {
        id: 1298,
        file: "kernel/src/main.rs",
        find: "        page_start < page_end && page_end <= 1_u64 << physical_bits,",
        replace: "        false && page_start < page_end && page_end <= 1_u64 << physical_bits,",
        breaks: "the live NVMe BAR witness withholds its admitted physical-width result",
        reddens: Caught::Check("a live nvme bar obeys maxphyaddr"),
    },
    Mutation {
        id: 1329,
        file: "kernel/src/selftest.rs",
        find: "    fn capture() -> Self {\n        let disabled = interrupt::disable();\n        let snapshot = Self {\n            free_frames: mm::free_frames(),\n            heap: mm::heap::stats(),\n        };\n        disabled.restore();\n        snapshot\n    }",
        replace: "    fn capture() -> Self {\n        Self {\n            free_frames: mm::free_frames(),\n            heap: mm::heap::stats(),\n        }\n    }",
        breaks: "an allocator ownership snapshot spans interrupt epochs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::allocator_ledgers_hold_one_interrupt_epoch",
        },
    },
    Mutation {
        id: 1330,
        file: "kernel/src/selftest.rs",
        find: "fn spawn_resources() -> SpawnResources {\n    let disabled = interrupt::disable();\n    let allocator = AllocatorSnapshot::capture();\n    let resources = SpawnResources {\n        frames: allocator.free_frames,\n        heap: allocator.heap,\n        arena: HandleTable::arena_usage_for_test(),\n        channels: object::live_channels_for_test(),\n        rings: queue::live_for_test(),\n        thread_slots: thread::free_slots(),\n        process_records: user::process_records_for_test(),\n        mapping_refusals: user::refused_mapping_reclaims(),\n        table_refusals: user::refused_table_reclaims(),\n        task_retirements: user::refused_task_retirements(),\n    };\n    disabled.restore();\n    resources\n}",
        replace: "fn spawn_resources() -> SpawnResources {\n    let allocator = AllocatorSnapshot::capture();\n    SpawnResources {\n        frames: allocator.free_frames,\n        heap: allocator.heap,\n        arena: HandleTable::arena_usage_for_test(),\n        channels: object::live_channels_for_test(),\n        rings: queue::live_for_test(),\n        thread_slots: thread::free_slots(),\n        process_records: user::process_records_for_test(),\n        mapping_refusals: user::refused_mapping_reclaims(),\n        table_refusals: user::refused_table_reclaims(),\n        task_retirements: user::refused_task_retirements(),\n    }\n}",
        breaks: "a multi-owner spawn ledger spans interrupt epochs",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::allocator_ledgers_hold_one_interrupt_epoch",
        },
    },
    Mutation {
        id: 1331,
        file: "kernel/src/user.rs",
        find: "                table: bootstrap.table.identity_for_test(),",
        replace: "                table: bootstrap.table.identity_for_test().wrapping_add(1),",
        breaks: "a zombie record names a different process incarnation",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 1332,
        file: "kernel/src/user.rs",
        find: "    (launches, bootstraps, zombies)",
        replace: "    (\n        launches,\n        bootstraps + usize::from(zombies != 0),\n        zombies,\n    )",
        breaks: "a retained zombie is also counted as a live bootstrap record",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 1333,
        file: "kernel/src/user.rs",
        find: "            RECLAIMED_PAGES.fetch_add(success.observed_returned_pages, Ordering::AcqRel);",
        replace: "            RECLAIMED_PAGES.fetch_add(\n                success.observed_returned_pages + 1,\n                Ordering::AcqRel,\n            );",
        breaks: "successful collection overcounts returned user pages",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 1334,
        file: "kernel/src/user.rs",
        find: "            RECLAIMED_TABLES.fetch_add(success.reclaimed.tables, Ordering::AcqRel);",
        replace: "            RECLAIMED_TABLES.fetch_add(\n                success.reclaimed.tables + 1,\n                Ordering::AcqRel,\n            );",
        breaks: "successful collection overcounts returned page-table frames",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 1338,
        file: "kernel/src/user.rs",
        find: "#[cfg(feature = \"selftest\")]\nstatic REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);",
        replace: "static REFUSED_MAPPING_RECLAIMS: AtomicUsize = AtomicUsize::new(0);",
        breaks: "mapping-reclaim evidence state reaches the shipped kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1346,
        file: "kernel/src/dev.rs",
        find: "pub(crate) fn keep_io_apic(io_apic: IoApic, #[cfg(feature = \"selftest\")] gsi: u32) {",
        replace: "pub(crate) fn keep_io_apic(io_apic: IoApic, gsi: u32) {",
        breaks: "the routed-GSI evidence parameter reaches the shipped helper",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1347,
        file: "kernel/src/main.rs",
        find: "        #[cfg(feature = \"selftest\")]\n        gsi,",
        replace: "        gsi,",
        breaks: "the routed-GSI evidence argument reaches the shipped call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1349,
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "            #[cfg(not(feature = \"selftest\"))]\n            dismiss(eoi);",
        replace: "            #[cfg(not(feature = \"selftest\"))]\n            {\n                let eoi = Some(eoi);\n                if let Some(eoi) = eoi {\n                    dismiss(eoi);\n                }\n            }",
        breaks: "the shipped ping path wraps completion authority for test-only recovery",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::every_interrupt_arm_has_explicit_completion_ownership",
        },
    },
    Mutation {
        id: 1407,
        file: "kernel/src/selftest.rs",
        find: "    let retired = last_rites_for(&spawned, 2_000).is_some();\n    if !retired {",
        replace: "    let retired = last_rites_for(&spawned, 2_000).is_none();\n    if !retired {",
        breaks: "the first ring-three crossing rejects its exact child-retirement evidence",
        reddens: Caught::Check("a thread crosses into ring three and calls back"),
    },
    Mutation {
        id: 1408,
        file: "kernel/src/selftest.rs",
        find: "    let this_child_called = call.count > before.count && call.slot == spawned.slot;",
        replace: "    let this_child_called = call.count > before.count && call.slot != spawned.slot;",
        breaks: "the first ring-three crossing accepts only another thread's call record",
        reddens: Caught::Check("a thread crosses into ring three and calls back"),
    },
    Mutation {
        id: 1526,
        file: "kernel/src/user.rs",
        find: "        if !x86_paging::user_mapping_fits(at, mapped_len) || segment.memory_size() == 0 {",
        replace: "        if mapped_end > usercopy::USER_END || segment.memory_size() == 0 {",
        breaks: "the loader places a segment on the last canonical page",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    Mutation {
        id: 1532,
        file: "kernel/src/arch/x86_64/ioapic.rs",
        find: "        let Some(entries) = entries_from_version(version) else {\n            return Err(TakeFailure {\n                registers,\n                reason: TakeError::ImplausibleVersion(version),\n            });\n        };",
        replace: "        let entries = ((version >> 16) & 0xFF) + 1;",
        breaks: "a window firmware aimed elsewhere is taken over as an I/O APIC",
        reddens: Caught::Check("a window that is not an I/O APIC is never taken over"),
    },
    Mutation {
        id: 1533,
        file: "kernel/src/arch/x86_64/debug_exit.rs",
        find: "    if let Some(mut device) = port::debug_exit() {",
        replace: "    if let Some(mut device) = port::claim::<u32>(0xF4) {",
        breaks: "a production driver claims a port by its number",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1536,
        file: "kernel/src/selftest.rs",
        find: "    let this_child = fault.slot == spawned.slot && fault.rax == 1;",
        replace: "    let this_child = fault.slot != spawned.slot && fault.rax == 1;",
        breaks: "the faulting witness accepts a fault from another slot as its child's",
        reddens: Caught::Check("a faulting program dies alone"),
    },
    Mutation {
        id: 1537,
        file: "kernel/src/selftest.rs",
        find: "    let this_child = fault.slot == spawned.slot && fault.rax == start;",
        replace: "    let this_child = fault.slot != spawned.slot && fault.rax == start;",
        breaks: "the copy-window witness accepts a fault from another slot as its child's",
        reddens: Caught::Check("a program cannot aim at the copy window"),
    },
    Mutation {
        id: 1538,
        file: "kernel/src/selftest.rs",
        find: "    let reaped_this_child = last_rites_for(&spawned, 2_000).is_some()",
        replace: "    let reaped_this_child = last_rites_for(&spawned, 2_000).is_none()",
        breaks: "the busy-reap witness accepts a reap without its child's own last rites",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        id: 1539,
        file: "kernel/src/queue.rs",
        find: "        *phase = WorkPhase::Running;",
        replace: "        *phase = WorkPhase::Queued;",
        breaks: "a read begins without advancing its pending entry into the running state",
        reddens: Caught::Panic {
            marker: "running file work was made without a running read",
            why: "the request path destructures a running read's state, so a read that begins while its entry stays queued reaches the unreachable arm the ring model proves cannot occur",
        },
    },
    Mutation {
        id: 1549,
        file: "kernel/src/main.rs",
        find: "        iommu::EstablishOutcome::Unavailable => warn!(\"storage: no DMA isolation\"),",
        replace: "        iommu::EstablishOutcome::Unavailable => warn!(\"storage: DMA isolation unavailable\"),",
        breaks: "the unisolated boot loses its exact refusal record",
        reddens: Caught::BootMarker(qemu::NO_ISOLATION_MARKER),
    },
    Mutation {
        id: 1570,
        file: "kernel/src/main.rs",
        find: "    *isolated = true;\n}",
        replace: "    *isolated = true;\n    *stage = admit_candidate(stage, &mut thread::ThreadContext::require());\n}",
        breaks: "the isolation stage runs the admit stage inside its own frame, so both debug frames share the boot stack",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::only_the_candidate_sequencer_calls_its_stages",
        },
    },
    Mutation {
        id: 1581,
        file: "kernel/src/arch/x86_64/uart.rs",
        find: "        let registers = port::com1().ok_or(UartError::PortsUnavailable)?;",
        replace: "        let registers = { use wost::arch::current::port::claim_block; claim_block::<8>(0x3F8) }.ok_or(UartError::PortsUnavailable)?;",
        breaks: "a production driver claims a port through an imported raw door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1582,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "        let (data, control) = port::i8042().ok_or(I8042Error::PortsUnavailable)?;",
        replace: "        let (data, control) = { use wost::arch::current::port::claim_pair as pair; pair::<u8, u8>(0x60, 0x64) }.ok_or(I8042Error::PortsUnavailable)?;",
        breaks: "a production driver claims a port through a renamed raw door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1583,
        file: "kernel/src/arch/x86_64/pit.rs",
        find: "        let registers = port::pit().ok_or(PitError::PortsUnavailable)?;",
        replace: "        let registers = { use wost::arch::current::port as io; io::claim_block::<4>(0x40) }.ok_or(PitError::PortsUnavailable)?;",
        breaks: "a production driver claims a port through a renamed port module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1584,
        file: "kernel/src/arch/x86_64/debug_exit.rs",
        find: "    if let Some(mut device) = port::debug_exit() {",
        replace: "    if let Some(mut device) = { use wost::arch::current::port::*; claim::<u32>(0xF4) } {",
        breaks: "a production driver claims a port through a glob import of the port module",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::production_kernel_code_claims_no_port_by_number",
        },
    },
    Mutation {
        id: 1589,
        file: "kernel/src/iommu.rs",
        find: "    reserved.next().is_none()",
        replace: "    let _ = reserved.next();\n    true",
        breaks: "a storage candidate turns a firmware RMRR into DMA mapping authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::firmware_reserved_dma_regions_never_become_mapping_authority",
        },
    },
    Mutation {
        id: 1590,
        file: "kernel/src/nvme.rs",
        find: "fn finish_preparation(\n    registers: MappedBar,\n    capabilities: Capabilities,\n    admin: QueuePair,\n    capacity: Capacity,\n    #[cfg(feature = \"selftest\")] observed_window: u64,\n) -> PrepareOutcome {\n",
        replace: "#[inline(never)]\nfn finish_preparation(\n    registers: MappedBar,\n    capabilities: Capabilities,\n    admin: QueuePair,\n    capacity: Capacity,\n    #[cfg(feature = \"selftest\")] observed_window: u64,\n) -> PrepareOutcome {\n    #[cfg(debug_assertions)]\n    let stack_pressure = [0u8; 8 * 1024];\n    #[cfg(not(debug_assertions))]\n    let stack_pressure = [0u8; 16 * 1024];\n    core::hint::black_box(&stack_pressure);\n",
        breaks: "a frame below preparation's register accesses consumes the boot-stack margin unseen by the live-leaf meter",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 1593,
        file: "kernel/src/nvme.rs",
        find: "fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {\n    #[cfg(feature = \"selftest\")]\n    observe_candidate_stack();",
        replace: "fn read(registers: &MappedBar, offset: usize) -> Result<u32, BringUpError> {\n    observe_candidate_stack();",
        breaks: "every shipped NVMe register read runs the stack meter's leaf hook",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::machine_evidence_writes_are_absent_from_shipped_paths",
        },
    },
    Mutation {
        id: 1630,
        file: "kernel/src/user.rs",
        find: "    if refused == 0 || REPORTED.swap(1, Ordering::AcqRel) != 0 {",
        replace: "    if refused == 0 || REPORTED.swap(refused, Ordering::AcqRel) == refused {",
        breaks: "each new unknown syscall can re-arm synchronous UART warning output",
        reddens: Caught::Check("an unserved call is refused without stopping the machine"),
    },
    Mutation {
        id: 1643,
        file: "kernel/src/nvme.rs",
        find: "        if !completion_mapped {\n            if isolation.unmap_segment(&mut self.submission).is_ok() {\n                self.submission_mapped = false;\n            }\n            return false;\n        }",
        replace: "        if !completion_mapped {\n            return false;\n        }",
        breaks: "a completion-queue mapping refusal leaves the submission queue reachable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::a_partial_queue_mapping_rolls_back_the_submission_queue",
        },
    },
    Mutation {
        id: 1649,
        file: "kernel/src/user.rs",
        find: "const INVALID_RETURN_STACK: u64 = 6;",
        replace: "const INVALID_RETURN_STACK: u64 = 0;",
        breaks: "the noncanonical-stack machine witness launches an ordinary program",
        reddens: Caught::Check(
            "a noncanonical user return stack retires only its program",
        ),
    },
    Mutation {
        id: 1650,
        file: "kernel/src/user.rs",
        find: "const INVALID_RETURN_FLAGS: u64 = 7;",
        replace: "const INVALID_RETURN_FLAGS: u64 = 0;",
        breaks: "the nested-task machine witness launches an ordinary program",
        reddens: Caught::Check("unsafe user return flags retire only their program"),
    },
    Mutation {
        id: 1653,
        file: "kernel/src/nvme.rs",
        find: "        if !self.unmap_queues() {\n            warn!(\"nvme: a queue mapping stayed reachable during retirement\");\n            return quarantine_controller(self);\n        }",
        replace: "",
        breaks: "clean controller retirement drops queue owners without invalidating their domain leaves",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::rejected_controller_retirement_fences_and_unmaps_every_queue",
        },
    },
    Mutation {
        id: 1654,
        file: "kernel/src/nvme.rs",
        find: "        self.retain_mapped();",
        replace: "        let _still_mapped = self.has_mappings();",
        breaks: "an accidentally dropped mapped queue can recycle device-reachable frames",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::rejected_controller_retirement_fences_and_unmaps_every_queue",
        },
    },
    Mutation {
        id: 1656,
        file: "kernel/src/main.rs",
        find: "            let retirement = controller.retire(&mut device, context);\n            iommu::report_faults();",
        replace: "            iommu::report_faults();\n            let retirement = controller.retire(&mut device, context);",
        breaks: "device-controlled VT-d diagnostics run before the failed controller reaches its DMA fence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::device_fault_reporting_is_bounded_and_follows_retirement",
        },
    },
    Mutation {
        id: 1657,
        file: "kernel/src/nvme.rs",
        find: "        self.io = Some(pair);",
        replace: "        let _orphaned_queue_owner = pair;",
        breaks: "an uncertain partial I/O queue map falls out of the controller retirement inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::uncertain_io_queue_mapping_stays_reachable_for_retirement",
        },
    },
    Mutation {
        id: 1660,
        file: "kernel/src/nvme.rs",
        find: "            Ok(()) => self.submission_mapped = true,",
        replace: "            Ok(()) => {}",
        breaks: "a successfully mapped submission queue is absent from conservative teardown state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1661,
        file: "kernel/src/nvme.rs",
        find: "        if self.submission_mapped {\n            self.submission.retain();\n        }",
        replace: "        let _submission_may_be_mapped = self.submission_mapped;",
        breaks: "the mapped-queue drop backstop omits a conservatively reachable submission segment",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1662,
        file: "kernel/src/nvme.rs",
        find: "        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {\n            self.submission_mapped = false;\n        }",
        replace: "        if self.submission_mapped && isolation.unmap_segment(&mut self.submission).is_ok() {\n            self.submission_mapped = true;\n        }",
        breaks: "a successfully invalidated submission queue stays marked live and forces false quarantine",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::queue_mapping_state_conserves_every_reachable_segment",
        },
    },
    Mutation {
        id: 1665,
        file: "kernel/src/iommu.rs",
        find: "    for requester in requesters.iter() {",
        replace: "    for requester in requesters.iter().skip(1) {",
        breaks: "the first governed requester is omitted from the unit-wide DMA fence boundary",
        reddens: Caught::Check(
            "every shared vtd requester was fenced across controller fallback",
        ),
    },
    Mutation {
        id: 1674,
        file: "kernel/src/main.rs",
        find: "    if !iommu::release_candidate(context) {\n        warn!(\"nvme: candidate {ordinal} could not return its dma domain; selection halted\");\n        pci::retain_failed_release(device);\n        return StorageSelection::Halted;\n    }\n    if let Some((reason, device)) = device.release(context).into_retained() {",
        replace: "    if let Some((reason, device)) = device.release(context).into_retained() {",
        breaks: "the selected function authority is returned before the unit-wide VT-d disable transition",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1679,
        file: "kernel/src/main.rs",
        find: "fn isolate_candidate(stage: &mut CandidateStage) {\n    let outcome = match stage {",
        replace: "fn isolate_candidate(stage: &mut CandidateStage) {\n    #[cfg(debug_assertions)]\n    let stack_regression = [0u8; 4096];\n    #[cfg(not(debug_assertions))]\n    let stack_regression = [0u8; 16 * 1024];\n    core::hint::black_box(&stack_regression);\n    let outcome = match stage {",
        breaks: "unit isolation again consumes a large candidate-stack frame instead of borrowing its owners in a thin sibling stage",
        reddens: Caught::Check("nvme candidates preserve two pages of boot-stack headroom"),
    },
    Mutation {
        id: 1689,
        file: "kernel/src/iommu.rs",
        find: "    if !pci::topology_is_sealed() {\n        return Err(LookupError::UnsafeTopology);\n    }",
        replace: "    let _unsealed_topology = pci::topology_is_sealed();",
        breaks: "DMAR authority can be derived before the complete PCI topology is sealed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1690,
        file: "kernel/src/pci.rs",
        find: "    TOPOLOGY.lock_leaf().reset();\n    for bus in 0..=u8::MAX {",
        replace: "    TOPOLOGY.lock_leaf().reset();\n    for bus in 0..u8::MAX {",
        breaks: "the sealed PCI snapshot omits every function on bus 255",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1701,
        file: "kernel/src/pci.rs",
        find: "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n) -> Result<(), TopologyError> {\n    TOPOLOGY.lock_leaf().reset();",
        replace: "pub(crate) fn snapshot_topology(\n    mut non_requester: Option<QualifiedNonRequester>,\n    mut quiesced: Option<QuiescedIsaRequester>,\n) -> Result<(), TopologyError> {\n    let stack_topology = PciTopology::new();\n    core::hint::black_box(&stack_topology);\n    TOPOLOGY.lock_leaf().reset();",
        breaks: "the complete PCI topology consumes a large fixed boot-stack frame",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::dmar_authority_uses_one_complete_sealed_pci_topology",
        },
    },
    Mutation {
        id: 1719,
        file: "kernel/src/iommu.rs",
        find: "    let Ok(dmar) = Dmar::new(table) else {\n        return false;\n    };\n    if !dmar.interrupt_remapping() || dmar.x2apic_opt_out() {",
        replace: "    let Ok(dmar) = Dmar::new(table) else {\n        return false;\n    };\n    if false && (!dmar.interrupt_remapping() || dmar.x2apic_opt_out()) {",
        breaks: "kernel VT-d lookup ignores firmware's missing IR support or x2APIC opt-out",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1720,
        file: "kernel/src/iommu.rs",
        find: "    if !covers_every_present_requester(&requesters) {",
        replace: "    if false && !covers_every_present_requester(&requesters) {",
        breaks: "interrupt authority is accepted for a VT-d unit that misses live PCI requesters",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1721,
        file: "kernel/src/iommu.rs",
        find: "fn covers_every_present_requester(\n    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,\n) -> bool {\n    for bus in 0..=u8::MAX {",
        replace: "fn covers_every_present_requester(\n    requesters: &RequesterInventory<{ pci::FUNCTION_AUTHORITY_LIMIT }>,\n) -> bool {\n    for bus in 0..u8::MAX {",
        breaks: "the interrupt boundary never checks live requesters on PCI bus 255",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1723,
        file: "kernel/src/main.rs",
        find: "    if !iommu::interrupt_boundary_active() {\n        warn!(\"iommu: complete PCI interrupt boundary missing; interrupts stay off\");\n        return;\n    }",
        replace: "    if false && !iommu::interrupt_boundary_active() {\n        warn!(\"iommu: complete PCI interrupt boundary missing; interrupts stay off\");\n        return;\n    }",
        breaks: "the CPU enables maskable interrupts while live PCI requesters lack interrupt isolation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1724,
        file: "kernel/src/main.rs",
        find: "    if !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {",
        replace: "    if false && !matches!(pci::topology_presence(), pci::TopologyPresence::Empty) {",
        breaks: "a PCI boot routes the legacy keyboard line outside the default-deny remapping table",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1725,
        file: "kernel/src/input.rs",
        find: "        if crate::time::is_calibrated() && interrupt::are_enabled() {\n            crate::thread::sleep_ms(&mut context, POLL_INTERVAL_MS);",
        replace: "        if crate::time::is_calibrated() {\n            crate::thread::sleep_ms(&mut context, POLL_INTERVAL_MS);",
        breaks: "the keyboard poller sleeps forever when timer state exists but interrupts stay off",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1731,
        file: "kernel/src/main.rs",
        find: "        if !rollback_keyboard_route(routing) {\n            warn!(\"keyboard: masked routing owner remains quarantined; staying polled\");\n            return;\n        }",
        replace: "        let _unchecked_masked_routing = routing;",
        breaks: "the PCI keyboard path reports polling without returning its masked IOAPIC owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1733,
        file: "kernel/src/pci.rs",
        find: "    if !topology_is_sealed() {\n        return TopologyPresence::Unsealed;\n    }",
        replace: "    if !topology_is_sealed() {\n        return TopologyPresence::Empty;\n    }",
        breaks: "a malformed unsealed PCI inventory is mistaken for a machine with no requesters",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1736,
        file: "kernel/src/iommu.rs",
        find: "    if dmar.sole_unit_segment() != Some(SEGMENT) {\n        return Err(LookupError::IncompleteInterruptBoundary);\n    }",
        replace: "    let _unchecked_remapping_units = dmar.sole_unit_segment();",
        breaks: "interrupt admission ignores a second or foreign-segment VT-d unit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1768,
        file: "kernel/src/selftest.rs",
        find: "    let woke_through_polling = runs_until(1_000, || {\n        READER_SAW.load(Ordering::Acquire) != 0 && input::poller_passes() > passes_before\n    });",
        replace: "    let woke_through_polling =\n        runs_until(1_000, || READER_SAW.load(Ordering::Acquire) != 0);",
        breaks: "the polled-reader witness samples poller progress after an earlier condition ends its wait",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_serial_contract_matches_the_kernel_source",
        },
    },
    Mutation {
        id: 1771,
        file: "kernel/src/queue.rs",
        find: "        if !held.contains(Rights::TRAVERSE) {\n            return Some(refuse(token, STATUS_DENIED));\n        }\n\n        if !worker_is_ready() {",
        replace: "        if !held.contains(Rights::TRAVERSE) {\n            return Some(refuse(token, STATUS_DENIED));\n        }\n\n        if crate::vfs::resolve(directory, &path[..len]).is_err() {\n            return Some(refuse(token, STATUS_NO_ENTRY));\n        }\n        if !worker_is_ready() {",
        breaks: "OpenAt reaches filesystem and controller locks from RingEnter trap context",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::open_at_storage_work_is_deferred_out_of_trap_context",
        },
    },
    Mutation {
        id: 1772,
        file: "kernel/src/queue.rs",
        find: "        // If the worker has not claimed this operation, remove the payload\n        // now. For OpenAt that drops its handle-slot promise before reporting\n        // cancellation; for Read it avoids spending worker time on a tombstone.\n        drop(FILE_WORK.remove(self, index, entry.id));",
        replace: "        // Mutated: leave the cancelled payload and its handle promise queued.",
        breaks: "a cancelled queued OpenAt keeps its reserved handle after the ring becomes quiet",
        reddens: Caught::Check(
            "a cancelled open releases its reserved handle before quiet",
        ),
    },
    Mutation {
        id: 1792,
        file: "kernel/src/nvme.rs",
        find: "            && isolation.map_segment_for_device_write(buffer).is_err()",
        replace: "            && isolation.map_segment(buffer).is_err()",
        breaks: "an NVMe read buffer gives the device read access to host residue",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1793,
        file: "kernel/src/main.rs",
        find: "    if !segment.zero() {",
        replace: "    if false {",
        breaks: "the startup read maps an uncleared allocator page for device DMA",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1798,
        file: "kernel/src/nvme.rs",
        find: "map_segment_for_device_read(&mut self.submission)",
        replace: "map_segment(&mut self.submission)",
        breaks: "the NVMe submission queue grants the device write access",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1799,
        file: "kernel/src/nvme.rs",
        find: "map_segment_for_device_write(&mut self.completion)",
        replace: "map_segment(&mut self.completion)",
        breaks: "the NVMe completion queue grants the device read access",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1800,
        file: "kernel/src/main.rs",
        find: "    Some(wost::mm::DmaSegment::from(segment))",
        replace: "    let segment = wost::mm::allocate_contiguous(1, 1)?;\n    Some(wost::mm::DmaSegment::from(segment))",
        breaks: "startup clears one allocation but grants DMA authority over a second uncleared one",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::vtd_publication_and_nvme_buffers_keep_exact_dma_authority",
        },
    },
    Mutation {
        id: 1804,
        file: "kernel/src/main.rs",
        find: "    let early_boundary = thread::ThreadContext::claim()\n        .is_some_and(|mut context| iommu::establish_fabric_boundary(&mut context));\n    if !early_boundary {\n        warn!(\"iommu: no early default-deny PCI boundary; storage and interrupts stay closed\");\n    }\n\n    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {",
        replace: "    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {\n        Ok(()) => info!(\"pci: complete segment-zero topology sealed\"),\n        Err(reason) => warn!(\"pci: DMA topology refused: {reason}\"),\n    }\n\n    let early_boundary = thread::ThreadContext::claim()\n        .is_some_and(|mut context| iommu::establish_fabric_boundary(&mut context));\n    if !early_boundary {\n        warn!(\"iommu: no early default-deny PCI boundary; storage and interrupts stay closed\");\n    }\n\n    match pci::snapshot_topology(dma_non_requester, dma_quiesced) {",
        breaks: "PCI topology is observed before the default-deny DMA owner exists",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1805,
        file: "kernel/src/iommu.rs",
        find: "        pci::TopologyPresence::Empty | pci::TopologyPresence::Present => {}",
        replace: "        pci::TopologyPresence::Empty => return true,\n        pci::TopologyPresence::Present => {}",
        breaks: "an empty PCI snapshot substitutes for a real installed VT-d owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1814,
        file: "kernel/src/iommu.rs",
        find: "        Establishment::Retained { reason, isolation } => {\n            warn!(\"iommu: the early default-deny takeover is uncertain: {reason}\");\n            install(InstalledUnit {\n                register_base,\n                isolation,\n                fences: UnitFences::empty(),\n                state: InstalledState::Quarantined,\n            });",
        replace: "        Establishment::Retained { reason, isolation } => {\n            warn!(\"iommu: the early default-deny takeover is uncertain: {reason}\");\n            install(InstalledUnit {\n                register_base,\n                isolation,\n                fences: UnitFences::empty(),\n                state: InstalledState::Ready,\n            });",
        breaks: "a failed post-enable takeover drain is recorded as a reusable early owner",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1815,
        file: "kernel/src/iommu.rs",
        find: "    if owner.state != InstalledState::Ready {\n        warn!(\"iommu: the early owner is quarantined and cannot be retargeted\");",
        replace: "    if false && owner.state != InstalledState::Ready {\n        warn!(\"iommu: the early owner is quarantined and cannot be retargeted\");",
        breaks: "a quarantined early owner can be revived by a later candidate retarget",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::unit_wide_vtd_transition_is_fenced_drained_and_owner_preserving",
        },
    },
    Mutation {
        id: 1816,
        file: "kernel/src/iommu.rs",
        find: "    (coverage.segment() == SEGMENT && coverage.register_base() == register_base)\n        .then_some(register_base)",
        replace: "    (coverage.segment() == SEGMENT).then_some(register_base)",
        breaks: "a platform coverage attestation can name a different physical remapping unit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1817,
        file: "kernel/src/iommu.rs",
        find: "acpi::find_unique_table(firmware, root, Sdt::DMA_COVERAGE)",
        replace: "acpi::find_table(firmware, root, Sdt::DMA_COVERAGE)",
        breaks: "the first of multiple platform coverage attestations can authorize takeover",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1818,
        file: "kernel/src/iommu.rs",
        find: "acpi::find_unique_table(firmware, root, Sdt::DMAR)",
        replace: "acpi::find_table(firmware, root, Sdt::DMAR)",
        breaks: "the first of multiple DMAR tables can corroborate early takeover authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1819,
        file: "kernel/src/acpi.rs",
        find: "                if require_unique {\n                    warn!(\"acpi: duplicate {signature:?} tables refuse unique authority\");\n                    return None;\n                }",
        replace: "                if false && require_unique {\n                    warn!(\"acpi: duplicate {signature:?} tables refuse unique authority\");\n                    return None;\n                }",
        breaks: "unique ACPI authority lookup silently returns the first duplicate table",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::pci_interrupts_are_default_deny_before_bus_mastering_or_sti",
        },
    },
    Mutation {
        id: 1827,
        file: "kernel/src/mem.rs",
        find: "    _dma: EarlyDmaBoundary,",
        replace: "    _dma: (),",
        breaks: "memory initialization no longer consumes the entry DMA proof",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1828,
        file: "kernel/src/iommu.rs",
        find: "#[derive(Debug)]\npub(crate) struct EarlyDmaBoundary {",
        replace: "#[derive(Debug, Clone)]\npub(crate) struct EarlyDmaBoundary {",
        breaks: "the one-shot entry DMA proof can be duplicated",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1829,
        file: "kernel/src/acpi.rs",
        find: "        if found.replace(table).is_some() {",
        replace: "        if false && found.replace(table).is_some() {",
        breaks: "the entry boundary accepts the first of multiple DMA handoff authorities",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1834,
        file: "kernel/src/iommu.rs",
        find: "    let platform = cpu::dma_handoff_platform().ok_or(EarlyDmaError::UnauthenticatedPlatform)?;",
        replace: "    let platform = cpu::DmaHandoffPlatform::Vmware;",
        breaks: "writable ACPI bytes authenticate their own DMA handoff without CPU-resident platform evidence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1835,
        file: "kernel/src/acpi.rs",
        find: "            return Err(EarlyTableError::RootEntry);",
        replace: "            continue;",
        breaks: "an unreadable ACPI root target can hide a competing DMA handoff authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1836,
        file: "kernel/src/iommu.rs",
        find: "    compiler_fence(Ordering::SeqCst);",
        replace: "    let _ = Ordering::SeqCst;",
        breaks: "LTO may move privileged memory mutation above nomem entry-fence port instructions",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1840,
        file: "kernel/src/iommu.rs",
        find: "        cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::vmware_440bx(),",
        replace: "        cpu::DmaHandoffPlatform::Vmware => HandoffFencePolicy::strict(),",
        breaks: "the authenticated VMware platform no longer selects its exact qualified non-requester profile",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1847,
        file: "kernel/src/pci.rs",
        find: "                    topology.observe_non_requester(\n                        authority,\n                        FunctionObservation::new(\n                            entry.at,\n                            entry.identity,\n                            entry.class_bits,\n                            entry.command,\n                            entry.class,\n                            entry.header,\n                            bus_numbers,\n                        ),\n                    )?;",
        replace: "                    let _ = authority;\n                    topology.observe(entry.at, entry.class, entry.header, bus_numbers)?;",
        breaks: "the exact VMware host bridge re-enters the later VT-d requester inventory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1852,
        file: "kernel/src/iommu.rs",
        find: "        .is_some_and(pci::topology_contains_requester)",
        replace: "        .is_some_and(pci::topology_contains_function)",
        breaks: "VT-d inventory repopulates a platform-qualified non-requester from mere PCI presence",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1853,
        file: "kernel/src/iommu.rs",
        find: "                if !pci::topology_contains_requester(at) {",
        replace: "                if !pci::topology_contains_function(at) {",
        breaks: "whole-unit coverage treats the qualified VMware host bridge as a DMA requester again",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1859,
        file: "kernel/src/main.rs",
        find: "    let dma_non_requester = early_dma.take_non_requester();",
        replace: "    let dma_non_requester = None;",
        breaks: "the exact entry-fence authority is discarded before the later topology snapshot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 1860,
        file: "kernel/src/pci.rs",
        find: "    if let Some(authority) = non_requester {",
        replace: "    if let Some(authority) = Option::<QualifiedNonRequester>::None {",
        breaks: "an entry-qualified non-requester may disappear before the sealed snapshot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2083,
        file: "kernel/src/selftest.rs",
        find: "fn spawn_resources_are_restored(before: &SpawnResources) -> bool {\n    !user::has_zombies() && spawn_resources().has_same_ownership(before)\n}",
        replace: "fn spawn_resources_are_restored(before: &SpawnResources) -> bool {\n    let _ignored_ownership = before;\n    !user::has_zombies()\n}",
        breaks: "a live process that has not become a zombie can satisfy self-test cleanup",
        reddens: Caught::Check("an unserved call is refused without stopping the machine"),
    },
    Mutation {
        id: 2284,
        file: "kernel/src/pci.rs",
        find: "    if let Some(record) = quiesced {\n        return fence_quiesced_requester(function, record);\n    }",
        replace: "    let _ = quiesced;",
        breaks: "a unit-wide fence tries to clear the quiesced requester's read-only BME",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2285,
        file: "kernel/src/pci.rs",
        find: "    if identity == Some(record.identity())",
        replace: "    if identity.is_some()",
        breaks: "the unit-wide fence holds a different function at the quiesced address",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2286,
        file: "kernel/src/pci.rs",
        find: "        && command == Some(record.command())",
        replace: "        && command.is_some()",
        breaks: "the unit-wide fence holds the quiesced requester after its command changed",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2287,
        file: "kernel/src/pci.rs",
        find: "        && pci::isa_dma_quiet()\n",
        replace: "        && true\n",
        breaks: "the unit-wide fence holds the quiesced requester without rechecking its 8237s",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "dma_contracts::memory_initialization_requires_the_exhaustive_entry_dma_boundary",
        },
    },
    Mutation {
        id: 2295,
        file: "kernel/src/main.rs",
        find: "at its masked 8237 controllers; it remains a requester\"",
        replace: "at its masked 8237 controllers\"",
        breaks: "the kernel's ISA quiesce record drifts from the VMware verifier marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "vmware::tests::the_vmware_isa_dma_marker_is_required_by_both_selectors",
        },
    },
    Mutation {
        id: 2298,
        file: "kernel/src/user.rs",
        find: "        EXECUTED.store(true, Ordering::Release);",
        replace: "        let _ = &EXECUTED;",
        breaks: "the shipped image reports no observation after a real user call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2299,
        file: "kernel/src/user.rs",
        find: "    if frame.code_selector() & 0b11 == 3 {\n        EXECUTED.store",
        replace: "    if true {\n        EXECUTED.store",
        breaks: "a call without saved CPL 3 can earn the shipped execution marker",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2300,
        file: "kernel/src/thread.rs",
        find: "crate::user::report_execution() || crate::user::report_refusals()",
        replace: "crate::user::report_refusals()",
        breaks: "the idle thread never publishes the shipped user execution observation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2301,
        file: "kernel/src/user.rs",
        find: " || EXECUTION_REPORTED.swap(true, Ordering::AcqRel)",
        replace: " || false",
        breaks: "the shipped user execution marker is repeated on every idle pass",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2302,
        file: "kernel/src/user.rs",
        find: "!EXECUTED.load(Ordering::Acquire) || EXECUTION_REPORTED.swap",
        replace: "EXECUTION_REPORTED.swap",
        breaks: "the idle thread publishes shipped execution without any user call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2305,
        file: "kernel/src/user.rs",
        find: "info!(\"user: ring 3 call observed\");",
        replace: "info!(\"user: ring 3 call expected\");",
        breaks: "the sole shipped user execution producer drifts from both consumers",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2306,
        file: "kernel/src/user.rs",
        find: "static EXECUTED: AtomicBool = AtomicBool::new(false);",
        replace: "static EXECUTED: AtomicBool = AtomicBool::new(true);",
        breaks: "the shipped execution observation starts earned before the first instruction",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
        },
    },
    Mutation {
        id: 2371,
        file: "kernel/src/handle.rs",
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;",
        replace: "        let granted = match offered.attenuate(declared) {\n            Some(granted) => granted,\n            None => {\n                core::mem::forget(self);\n                return Err(HandleError::Denied);\n            }\n        };",
        breaks: "the attenuation-only refusal leaks its already-reserved handle slot",
        reddens: Caught::Check("a refused handle operation returns its promise"),
    },
    Mutation {
        id: 2372,
        file: "kernel/src/selftest.rs",
        find: "            0xF003,\n            attenuation_source,\n            u64::from(attenuation_request.bits()),",
        replace: "            0xF003,\n            undupable,\n            u64::from(attenuation_request.bits()),",
        breaks: "the claimed attenuation fixture actually refuses at the earlier duplicate-right gate",
        reddens: Caught::Check("a refused handle operation returns its promise"),
    },
    Mutation {
        id: 2430,
        file: "kernel/src/selftest.rs",
        find: "    let exact_pair = progressed\n        && read().is_some_and(|pressed| {",
        replace: "    let exact_pair = (progressed || true)\n        && read().is_some_and(|pressed| {",
        breaks: "a refused wait consumes the finite queued pair and can report successful input",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2431,
        file: "kernel/src/selftest.rs",
        find: "    nvme::hold_command_for_test(false);\n    exact_pair\n}",
        replace: "    if progressed {\n        nvme::hold_command_for_test(false);\n    }\n    exact_pair\n}",
        breaks: "a failed wait returns without releasing the armed command hold",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2432,
        file: "kernel/src/selftest.rs",
        find: "fn runs_until(ms: u64, done: impl Fn() -> bool) -> bool {\n    if !the_clock_is_running() {\n        return false;\n    }\n    let deadline = time::uptime_ms() + ms;\n    runs_until_deadline(deadline, done)\n}\n\nfn runs_until_deadline(deadline: u64, done: impl Fn() -> bool) -> bool {\n    loop {\n        if !time::is_calibrated() {",
        replace: "fn runs_until(ms: u64, done: impl Fn() -> bool) -> bool {\n    if false && !the_clock_is_running() {\n        return false;\n    }\n    let deadline = time::uptime_ms() + ms;\n    runs_until_deadline(deadline, done)\n}\n\nfn runs_until_deadline(deadline: u64, done: impl Fn() -> bool) -> bool {\n    loop {\n        if false && !time::is_calibrated() {",
        breaks: "an unavailable clock admits an otherwise-ready predicate as a successful wait",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2433,
        file: "kernel/src/selftest.rs",
        find: "        && read().is_some_and(|pressed| {\n            pressed.event.key == ps2_keys::Key::C",
        replace: "        && read().is_none_or(|pressed| {\n            pressed.event.key == ps2_keys::Key::C",
        breaks: "an absent first event is admitted and consumes another event after the pair is already impossible",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2434,
        file: "kernel/src/selftest.rs",
        find: "        && read().is_some_and(|released| {\n            released.event.key == ps2_keys::Key::C",
        replace: "        && read().is_none_or(|released| {\n            released.event.key == ps2_keys::Key::C",
        breaks: "a press without a release is accepted as the completed key pair",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2435,
        file: "kernel/src/input.rs",
        find: "pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {\n    EVENTS.lock_leaf().pop()\n}",
        replace: "pub(crate) fn try_read_key_for_test() -> Option<Keystroke> {\n    None\n}",
        breaks: "the nonblocking self-test reader refuses even an available queued event",
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 2436,
        file: "kernel/src/input.rs",
        find: "    let exclusion = interrupt::disable();\n    let mut keystrokes = [None; BATCH];\n    let count = drain(&mut keystrokes);\n    after_drain();",
        replace: "    let mut keystrokes = [None; BATCH];\n    let count = drain(&mut keystrokes);\n    after_drain();\n    let exclusion = interrupt::disable();",
        breaks: "Acquiring exclusion only after the drain-to-delivery boundary lets a competing producer admit its later batch before the first batch.",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        id: 2437,
        file: "kernel/src/input.rs",
        find: "    if admitted {\n        observe(keystroke.event);\n    }",
        replace: "    observe(keystroke.event);",
        breaks: "A rejected full-queue press/release pair advances the self-test arrival watch even though no event was admitted.",
        reddens: Caught::Check("the controller route and ordered interrupt witness are exact"),
    },
    Mutation {
        id: 2441,
        file: "kernel/src/time.rs",
        find: "    elapsed.convert(timestamp.wrapping_sub(ORIGIN.load(Ordering::Relaxed)))\n}",
        replace: "    elapsed.convert(timestamp.wrapping_sub(ORIGIN.load(Ordering::Relaxed))) / 1_000 * 1_000\n}",
        breaks: "public clock conversion rounds every adjacent counter reading to whole microseconds",
        reddens: Caught::Check("the clock advances in steps finer than a microsecond"),
    },
    Mutation {
        id: 2442,
        file: "kernel/src/selftest.rs",
        find: "        if sample < previous {\n            sweep.monotonic = false;\n        }",
        replace: "        let sample = (sample, previous).1;\n        if sample < previous {\n            sweep.monotonic = false;\n        }",
        breaks: "the public clock stays frozen while raw adjacent counter readings still advance",
        reddens: Caught::Check("the clock advances in steps finer than a microsecond"),
    },
    Mutation {
        id: 2444,
        file: "kernel/src/selftest.rs",
        find: "fn runs_until_deadline(deadline: u64, done: impl Fn() -> bool) -> bool {\n    loop {\n        if !time::is_calibrated() {\n            return false;\n        }",
        replace: "fn runs_until_deadline(deadline: u64, done: impl Fn() -> bool) -> bool {\n    loop {",
        breaks: "an admitted self-test deadline keeps waiting after the live clock is revoked",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        id: 2462,
        file: "kernel/src/pci.rs",
        find: "if self.claimed[usize::from(bus)] || self.len == MAX_BUSES {",
        replace: "if self.len == MAX_BUSES {",
        breaks: "a repeated bridge secondary takes a worklist slot again",
        reddens: Caught::Check("a repeated bridge secondary cannot hide a later bus"),
    },
    Mutation {
        id: 2463,
        file: "kernel/src/arch/x86_64/i8042.rs",
        find: "        discard();
    }
    !full()
}",
        replace: "        discard();
    }
    false
}",
        breaks: "a controller that drains on its last allowed read is reported absent",
        reddens: Caught::Check("an 8042 backlog of exactly the flush bound drains"),
    },
    Mutation {
        id: 2464,
        file: "kernel/src/selftest.rs",
        find: "    let early = queue.expire(deadline - 1) == 0 && queue.in_flight() == 1;",
        replace: "    let early = queue.expire(crate::time::now_ns()) == 0 && queue.in_flight() == 1;",
        breaks: "the early timer sweep reads the live clock, so a late sweep sees the deadline passed",
        reddens: Caught::Check("an operation completes after its deadline and only once"),
    },
];
