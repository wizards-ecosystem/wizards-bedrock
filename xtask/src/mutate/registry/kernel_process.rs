use crate::qemu;

use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
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
            name: "contracts::address_space::one_production_owner_binds_and_retires_each_address_space",
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
            name: "contracts::address_space::one_production_owner_binds_and_retires_each_address_space",
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
            name: "contracts::address_space::one_production_owner_binds_and_retires_each_address_space",
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
            name: "contracts::address_space::one_production_owner_binds_and_retires_each_address_space",
        },
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
    // Skipping release leaks one promised handle slot after every refusal.
    Mutation {
        id: 203,
        file: "kernel/src/handle.rs",
        find: "        if !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        replace: "        if false && !self.spent {\n            let released = self.table.step(Quota::release);\n            assert!(released, \"a live reservation refused quota release\");\n        }",
        breaks: "a refused operation gives its promised slot back",
        reddens: Caught::Check("a refused handle operation returns its promise"),
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
        id: 492,
        file: "kernel/src/handle.rs",
        find: "        if entry.owner != self.id {\n            return Err(HandleError::Stale);\n        }",
        replace: "",
        breaks: "a resolved handle cannot cross its ownership table",
        reddens: Caught::Check("a handle reveals no foreign occupancy"),
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
        id: 568,
        file: "kernel/src/user.rs",
        find: "maskable interrupts are not admitted; ring 3 is closed",
        replace: "interrupt admission unavailable; ring 3 is closed",
        breaks: "the degraded boot loses the exact first user-boundary refusal",
        reddens: Caught::BootMarker(qemu::DEGRADED_USER_MARKER),
    },
    Mutation {
        id: 570,
        file: "kernel/src/handle.rs",
        find: "        assert!(committed, \"a live reservation refused quota commit\");",
        replace: "        debug_assert!(committed, \"a live reservation refused quota commit\");",
        breaks: "a broken handle commitment is terminal only in debug kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::production_invariants_are_not_debug_only",
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
            name: "contracts::errors::production_invariants_are_not_debug_only",
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
            name: "contracts::errors::known_fallible_results_are_not_discarded",
        },
    },
    Mutation {
        id: 609,
        file: "kernel/src/user.rs",
        find: "        crate::UserIsolation::Closed => return Err(SpawnError::NoUserIsolation),",
        replace: "        crate::UserIsolation::Closed => AddressSpace::for_user(),",
        breaks: "ring three opens without an admitted shared or paired isolation mode",
        reddens: Caught::Check("shared kernel mappings without an isolation proof are refused"),
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
        id: 935,
        file: "kernel/src/thread.rs",
        find: "    fn choose(&self, current: usize, eligibility: CurrentEligibility) -> Option<usize> {",
        replace: "    fn choose(&self, current: usize, eligibility: bool) -> Option<usize> {",
        breaks: "scheduler current-thread eligibility becomes an unnamed boolean selector",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
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
            name: "contracts::machine::machine_evidence_writes_are_absent_from_shipped_paths",
        },
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
        id: 1630,
        file: "kernel/src/user.rs",
        find: "    if refused == 0 || REPORTED.swap(1, Ordering::AcqRel) != 0 {",
        replace: "    if refused == 0 || REPORTED.swap(refused, Ordering::AcqRel) == refused {",
        breaks: "each new unknown syscall can re-arm synchronous UART warning output",
        reddens: Caught::Check("an unserved call is refused without stopping the machine"),
    },
    Mutation {
        id: 1649,
        file: "kernel/src/user.rs",
        find: "const INVALID_RETURN_STACK: u64 = 6;",
        replace: "const INVALID_RETURN_STACK: u64 = 0;",
        breaks: "the noncanonical-stack machine witness launches an ordinary program",
        reddens: Caught::Check("a noncanonical user return stack retires only its program"),
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
        id: 2298,
        file: "kernel/src/user.rs",
        find: "        EXECUTED.store(true, Ordering::Release);",
        replace: "        let _ = &EXECUTED;",
        breaks: "the shipped image reports no observation after a real user call",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
            name: "qemu::tests::kernel_source::shipped_execution_evidence_requires_a_user_call_and_deferred_once_only_report",
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
];
