use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 169,
        file: "kernel/src/queue/ring.rs",
        find: "            if published == 0 || published > SUBMISSIONS {",
        replace: "            if published == 0 {",
        breaks: "a runaway submission tail is bounded",
        reddens: Caught::Check("a rewritten submission cannot change what was served"),
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
    // A list-only directory handle must not become traversal authority
    // (D-173).
    Mutation {
        id: 272,
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
        find: "        match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {",
        replace: "        match crate::vfs::read(request.file, 0, &mut scratch[..request.wanted]) {",
        breaks: "a read starts where it was told to",
        reddens: Caught::Check("a read is a submission with its own offset"),
    },
    // The buffer bound moved after the read. The output is unchanged, so
    // only the driver's command counter can tell the kernels apart (D-173).
    Mutation {
        id: 279,
        file: "kernel/src/queue/ring.rs",
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
        file: "kernel/src/queue/work.rs",
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
    Mutation {
        id: 337,
        file: "kernel/src/queue/work.rs",
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
        reddens: Caught::Check("a delayed read preserves interrupt admission and exact teardown"),
    },
    Mutation {
        id: 339,
        file: "kernel/src/wait.rs",
        find: "            let mut waiters = self.waiters.lock(context);
            if ready() {",
        replace: "            let mut waiters = self.waiters.lock(context);
            if false && ready() {",
        breaks: "a waiter registers and blocks without checking its condition under the queue lock",
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
        id: 344,
        file: "kernel/src/queue.rs",
        find: "                if !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        replace: "                if false && !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        breaks: "a clockless Timer is refused before it becomes pending",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        id: 491,
        file: "kernel/src/queue/ring.rs",
        find: "        let tail = ring.protocol.completions().tail();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        replace: "        let tail = ring.protocol.completions().tail();\n        drop(ring);\n        let mut ring = self.ring.lock_leaf();\n\n        let at = COMPLETIONS_AT + slot * COMPLETION_BYTES;",
        breaks: "a later kernel completion can publish past an unwritten earlier slot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::publication::tests::kernel_completion_producers_publish_in_tail_order",
        },
    },
    Mutation {
        id: 501,
        file: "kernel/src/time.rs",
        find: "            if failures >= WATCHDOG_FAILURES {",
        replace: "            if failures > WATCHDOG_FAILURES {",
        breaks: "the required watchdog failures revoke the live timing source",
        reddens: Caught::Check("a clock disagreement revokes time without stranding timers"),
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
        reddens: Caught::Check("a clock disagreement revokes time without stranding timers"),
    },
    Mutation {
        id: 524,
        file: "kernel/src/queue.rs",
        find: "            if !entry.settling && wanted(*entry) {",
        replace: "            if wanted(*entry) {",
        breaks: "two terminal paths can claim one pending operation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::publication::tests::every_terminal_path_claims_one_settlement_until_publication",
        },
    },
    Mutation {
        id: 566,
        file: "kernel/src/queue.rs",
        find: "            assert!(posted, \"a reserved completion slot was not there\");",
        replace: "            debug_assert!(posted, \"a reserved completion slot was not there\");",
        breaks: "a missing reserved completion wedges only release kernels",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::production_invariants_are_not_debug_only",
        },
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
        id: 606,
        file: "kernel/src/time.rs",
        find: "    assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        replace: "    debug_assert!(\n        SCHEDULER_NOW_NS\n            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {",
        breaks: "release scheduler-time accounting stops proving its unconditional update",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::production_invariants_are_not_debug_only",
        },
    },
    Mutation {
        id: 683,
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
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
        file: "kernel/src/queue/work.rs",
        find: "        if self.head >= WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        replace: "        if self.head > WORK_DEPTH || self.tail >= WORK_DEPTH || self.len > WORK_DEPTH {",
        breaks: "a work queue accepts its exclusive head bound as an index",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 687,
        file: "kernel/src/queue/work.rs",
        find: "            pending.checked_add(1)",
        replace: "            Some(pending.wrapping_add(1))",
        breaks: "reap request admission wraps its outstanding count through zero",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 688,
        file: "kernel/src/queue/work.rs",
        find: "            pending.checked_sub(1)",
        replace: "            Some(pending.wrapping_sub(1))",
        breaks: "an empty reap request counter wraps to the maximum",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    Mutation {
        id: 987,
        file: "kernel/src/queue/work.rs",
        find: "            WORKER_IS_PAUSED.store(true, Ordering::Release);",
        replace: "            WORKER_IS_PAUSED.store(false, Ordering::Release);",
        breaks: "the queue worker never acknowledges its release-race pause",
        reddens: Caught::Check("a task release refusal retains the zombie for retry"),
    },
    Mutation {
        id: 1539,
        file: "kernel/src/queue/work.rs",
        find: "        *phase = WorkPhase::Running;",
        replace: "        *phase = WorkPhase::Queued;",
        breaks: "a read begins without advancing its pending entry into the running state",
        reddens: Caught::Panic {
            marker: "running file work was made without a running read",
            why: "the request path destructures a running read's state, so a read that begins while its entry stays queued reaches the unreachable arm the ring model proves cannot occur",
        },
    },
    Mutation {
        id: 1771,
        file: "kernel/src/queue/work.rs",
        find: "        if !held.contains(Rights::TRAVERSE) {\n            return Some(refuse(token, STATUS_DENIED));\n        }\n\n        if !worker_is_ready() {",
        replace: "        if !held.contains(Rights::TRAVERSE) {\n            return Some(refuse(token, STATUS_DENIED));\n        }\n\n        if crate::vfs::resolve(directory, &path[..len]).is_err() {\n            return Some(refuse(token, STATUS_NO_ENTRY));\n        }\n        if !worker_is_ready() {",
        breaks: "OpenAt reaches filesystem and controller locks from RingEnter trap context",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::doors::open_at_storage_work_is_deferred_out_of_trap_context",
        },
    },
    Mutation {
        id: 1772,
        file: "kernel/src/queue.rs",
        find: "        // If the worker has not claimed this operation, remove the payload\n        // now. For OpenAt that drops its handle-slot promise before reporting\n        // cancellation; for Read it avoids spending worker time on a tombstone.\n        drop(FILE_WORK.remove(self, index, entry.id));",
        replace: "        // Mutated: leave the cancelled payload and its handle promise queued.",
        breaks: "a cancelled queued OpenAt keeps its reserved handle after the ring becomes quiet",
        reddens: Caught::Check("a cancelled open releases its reserved handle before quiet"),
    },
    Mutation {
        id: 2441,
        file: "kernel/src/time.rs",
        find: "    elapsed.convert(timestamp.wrapping_sub(ORIGIN.load(Ordering::Relaxed)))\n}",
        replace: "    elapsed.convert(timestamp.wrapping_sub(ORIGIN.load(Ordering::Relaxed))) / 1_000 * 1_000\n}",
        breaks: "public clock conversion rounds every adjacent counter reading to whole microseconds",
        reddens: Caught::Check("the clock advances in steps finer than a microsecond"),
    },
];
