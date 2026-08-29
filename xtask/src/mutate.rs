//! Breaking the kernel on purpose, and requiring the checks to notice.
//!
//! D-28 says every new check is broken once and watched go red, because a
//! check that cannot fail is worth less than no check — it is also a claim.
//! Five adversarial audits have now found roughly twenty checks that could not
//! fail, several of them in the same commit as an ADR asserting they had been
//! broken. Doing it by hand is the problem: it happens once, at the moment the
//! check is written, and never again — so a check that *stops* being able to
//! fail, because the code around it changed, is invisible from then on.
//!
//! This makes it a task. Each mutation below is a real defect: a bound
//! removed, a flag not stripped, a guard inverted. Applying one and running
//! must turn some check **red**. A mutation that survives is a hole, and the
//! report names it (D-119).
//!
//! **Two tiers, and the cheap one matters more.** `cargo test` checks that
//! every mutation's pattern still occurs exactly once in its file — no boot, no
//! build, milliseconds. A pattern that has stopped matching is a mutation that
//! silently stopped testing anything, which is precisely the failure mode this
//! whole file exists to catch, one level up. The expensive tier — build, run,
//! require red, restore — is `cargo xtask mutate`. Machine mutations boot;
//! pure host logic runs the exact host test it names. It is not in `ci`
//! because the machine entries still cost one QEMU boot apiece.
//!
//! **The checkout is never the mutation target.** D-208 requires a clean
//! `HEAD`, copies only tracked bytes into a commit- and registry-pinned
//! workspace, and journals each active mutation before changing that copy.
//! Builds and boots use the copy and a private target directory. A killed run
//! resumes from its newest complete journal, restores the active file from an
//! immutable snapshot, and never needs to repair the user's source or index.
//!
//! **A red run is causal only after green.** D-209 proves each witness class
//! green before touching the snapshot. Machine mutations must then produce
//! their exact named check or registry-pinned panic marker; any other failure
//! is inconclusive rather than caught. D-232 permits one bounded second
//! observation after an inconclusive first; a survivor or exact catch is
//! terminal, and two inconclusive observations still fail qualification.
//!
//! D-210 keeps one consequential production-source break alive in every pure
//! crate. A host command must print the exact test's own pass or failure line;
//! a successful command which selected zero tests proves nothing.

use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::mutation_workspace::{Outcome, Session};
use crate::paths::Paths;
use crate::{qemu, sha256, tasks};

/// One deliberate defect, and what it is a defect *in*.
#[derive(Debug, Clone, Copy)]
pub struct Mutation {
    /// Workspace-relative path.
    pub file: &'static str,
    /// The exact text to replace. Must occur exactly once.
    pub find: &'static str,
    /// What to put there.
    pub replace: &'static str,
    /// The property this removes, in the register a check name is written in.
    ///
    /// Human-facing, and deliberately *not* the check's name: only four of
    /// these are, and the paraphrases read better in a progress line than the
    /// checks they stand for. [`Mutation::reddens`] carries the name.
    pub breaks: &'static str,
    /// The check this must turn red, or why nothing names it.
    pub reddens: Caught,
}

/// What a mutation must produce, and how precisely it is required.
#[derive(Debug, Clone, Copy)]
pub enum Caught {
    /// The exact `report.check("…")` name that must appear as `FAILED`.
    Check(&'static str),
    /// An exact host test whose own FAILED verdict must name the defect.
    HostTest {
        /// Cargo package containing the test.
        package: &'static str,
        /// Fully qualified test name accepted by `--exact`.
        name: &'static str,
    },
    /// The compiler-resolved API audit must name this exact dead item.
    ApiAudit(&'static str),
    /// A shipped degraded boot must emit one exact serial marker.
    BootMarker(&'static str),
    /// The defect kills the machine before the verdict.
    Panic {
        /// Exact text which must occur in the serial panic line.
        marker: &'static str,
        /// Why a terminal outcome is the defect's expected witness.
        why: &'static str,
    },
}

/// The defects, each of which some check must notice.
///
/// Chosen to be *load-bearing rather than merely different*: every one removes
/// a guard, a bound or a strip that something depends on, and none is a
/// cosmetic edit that would fail a build instead. A mutation that cannot
/// compile tests nothing.
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.preflight(&range, flags, grain)? == MappingState::Identical {\n            return Ok(());\n        }",
        replace: "        let _ = self.preflight(&range, flags, grain);",
        breaks: "mapping rollback clears only leaves this transaction created",
        reddens: Caught::Check("a refused remap preserves every older entry"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.shared.push(run.keepalive());",
        replace: "        drop(run.keepalive());",
        breaks: "a shared mapping retains ownership of every frame it reaches",
        reddens: Caught::Check("a shared mapping keeps its frame"),
    },
    // The strip in `map_boot`, not the one in `map_run`. The first version of
    // this mutation targeted `map_run`, which adds `OWNED` back on the next
    // line — so stripping the caller's changed nothing and the mutation
    // survived. The tool found a defect in itself, which is the same class of
    // finding and exactly as welcome (D-119).
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            flags.difference(NOT_THE_CALLERS),
            Grain::Largest,",
        replace: "            flags,
            Grain::Largest,",
        breaks: "a boot mapping cannot launder a frame into the allocator",
        reddens: Caught::Check("a boot mapping cannot claim ownership"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        space.reach = Reach::UserHalfOnly;",
        replace: "        space.reach = Reach::Everywhere;",
        breaks: "a user space cannot write the kernel's half",
        reddens: Caught::Check("a user space cannot reach the kernel half"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if flags.contains(Flags::USER) {\n            return Err(MapError::NotYours(virtual_base));\n        }",
        replace: "",
        breaks: "kernel memory cannot be published to ring 3",
        reddens: Caught::Check("kernel memory cannot be published to ring three"),
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if segment.zero() {\n            return Some(segment);\n        }",
        replace: "        return Some(segment);\n        #[allow(unreachable_code)]\n        if segment.zero() {\n            return Some(segment);\n        }",
        breaks: "a reclaimed frame is cleared before the kernel sees it",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        if !held.contains(Rights::TRANSFER) {\n            return Err(HandleError::Denied);\n        }",
        replace: "",
        breaks: "a handle without TRANSFER cannot be sent",
        reddens: Caught::Check("a handle without transfer is refused at the sender"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        // The next line is part of the pattern, and has to be: `install`
        // opened a second attenuation with the identical first line, so the
        // one-site rule started failing here rather than silently mutating
        // whichever came first (D-163).
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        self.insert(object, granted)",
        replace: "        let granted = offered.attenuate(declared).unwrap_or(offered);
        self.insert(object, granted)",
        breaks: "a receiver cannot widen what it was offered",
        reddens: Caught::Check("a handle crosses with the rights both ends declared"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            if published == 0 || published > SUBMISSIONS {",
        replace: "            if published == 0 {",
        breaks: "a runaway submission tail is bounded",
        reddens: Caught::Check("a rewritten submission cannot change what was served"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if xstate::is_enabled() {",
        replace: "    if false && xstate::is_enabled() {",
        breaks: "extended state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    table.slots[current].thread_base = read_thread_base();",
        replace: "",
        breaks: "thread-local storage follows the thread",
        reddens: Caught::Check("thread local storage follows the thread"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "    if wanted != 0 && wanted != cpu::page_table_base() {",
        replace: "    if false && wanted != cpu::page_table_base() {",
        breaks: "a thread's address space is installed by the switch",
        reddens: Caught::Check("a second thread crosses onto its own stack"),
    },
    // Its failure mode is a *panic* rather than a red check, because without
    // the fixup the fault is terminal and the self-test never reaches its
    // verdict. The harness treats both as a failure, which is right: a kernel
    // that died is a kernel that noticed (D-120).
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "&& usercopy::recover(frame) =>",
        replace: "&& false && usercopy::recover(frame) =>",
        breaks: "a faulting user copy resumes instead of stopping the machine",
        reddens: Caught::Panic {
            marker: "faulting address 0x0000600000000000",
            why: "without the fixup the fault is terminal, so the kernel dies before it \
                  reaches a verdict at all",
        },
    },
    // Its failure mode is a panic: the assert in `ThreadLock` fires, which is
    // the defect being a defect. A program reaching this halted the machine
    // (D-122).
    Mutation {
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
    // The dropping road no check can observe: those frames go to the
    // allocator, and reading them again needs it to hand back the same ones.
    // This is what covers it (D-123).
    // Its failure mode is a red check rather than a panic: a ring-3 thread
    // aiming `RIP` into the copy window gets recovered instead of retired, and
    // the recovery counter moves where the check requires it not to (D-126).
    Mutation {
        file: "kernel/src/arch/x86_64/trap.rs",
        find: "if frame.code_selector() & 0b11 == 0 && usercopy::recover(frame) =>",
        replace: "if usercopy::recover(frame) =>",
        breaks: "a user copy is only recovered when the kernel was the one copying",
        reddens: Caught::Check("a program cannot aim at the copy window"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "            if frames.zero() {",
        replace: "            if false && frames.zero() {",
        breaks: "a dropped user range is cleared before its frames go back",
        reddens: Caught::Check("a user page comes back cleared"),
    },
    // D-125's re-walk. The root comparison alone is what this was before, and
    // it answers a question from the past: which space the CPU is walking, not
    // whether the stack is still in it.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        matches!(
            translate(last),",
        replace: "        return true;
        #[allow(unreachable_code)]
        matches!(
            translate(last),",
        breaks: "a stack that is no longer mapped is no longer installed",
        reddens: Caught::Check("an unmapped stack is no longer installed"),
    },
    // D-126's detector, exercised by putting a violation back. Holding the
    // process table's guard while inspecting its ring takes a second leaf
    // underneath it.
    Mutation {
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
        file: "kernel/src/user.rs",
        find: "    Ok(Spawned {
        slot,
        channel: ours,
    })",
        replace: "    let channel = match object::channel() {
        Some((unrelated, _)) => unrelated,
        None => ours,
    };
    Ok(Spawned { slot, channel })",
        breaks: "a spawned program returns the parent end of its own bootstrap channel",
        reddens: Caught::Check("a spawned program holds exactly one handle"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.bound {
            match released {",
        replace: "        if false && self.bound {
            match released {",
        breaks: "a bound space is not reclaimed under its own thread",
        reddens: Caught::Check("a bound space is not reclaimed"),
    },
    // D-127's wake. The `Drop` still runs and still reaches the peer; it just
    // says nothing, which is the state the kernel was in before it existed. A
    // receiver already blocked then sleeps for the rest of the boot, so the
    // check that drops the far end under one is what has to notice.
    // The nine checks the Phase 0-5 audit's fixes landed with (D-135 to D-145).
    // Each is the exact mutation its break-test used.
    Mutation {
        file: "kernel/src/console.rs",
        find: "        self.set_cursor(false);
        self.put(text);
        self.newline();
        self.set_cursor(true);",
        replace: "        self.write_text(text);
        self.newline();",
        breaks: "a line break happens with the cursor down",
        reddens: Caught::Check("a scroll leaves no cursor behind"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/port.rs",
        find: "    if !CLAIMED.lock_leaf().reserve_all(&spans) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        replace: "    if !reserve(first, size_of::<T>()) {
        return None;
    }
    if !reserve(second, size_of::<U>()) {
        return None;
    }
    // SAFETY: the registry has just recorded every port both accesses drive,",
        breaks: "two claims are taken all at once or not at all",
        reddens: Caught::Check("a refused pair of claims strands neither"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "fn enable_write_protection() {
    write_cr0(read_cr0() | CR0_WRITE_PROTECT);
}",
        replace: "fn enable_write_protection() {
    write_cr0(read_cr0());
}",
        breaks: "the kernel enables supervisor write protection itself",
        reddens: Caught::Check("supervisor writes obey read-only pages"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/cpu.rs",
        find: "    unsafe { msr::write(EFER, msr::read(EFER) & !EFER_SYSCALL) };",
        replace: "",
        breaks: "the syscall door is shut before ring 3 runs",
        reddens: Caught::Check("ring three has no fast-syscall door"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if self.is_reclaimed() {
            RECLAIMED_REFUSALS.fetch_add(1, Ordering::Relaxed);
            return None;
        }
        translate_in(self.root, virtual_address)",
        replace: "        translate_in(self.root, virtual_address)",
        breaks: "a reclaimed space is not walked as physical frame zero",
        reddens: Caught::Check("a reclaimed space answers nothing"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "    if !root.is_live() {
        STALE_ROOT_REFUSALS.fetch_add(1, Ordering::Relaxed);
        return None;
    }",
        replace: "",
        breaks: "a root outliving its thread authorises nothing",
        reddens: Caught::Check("a stale root authorises no walk"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let slot = &mut self.slots[index];
        let generation = slot.generation;
        slot.entry = Some(entry);",
        replace: "        let generation = self.slots[index].generation;
        let _ = entry;",
        breaks: "a claimed slot is filled by the call that claimed it",
        reddens: Caught::Check("filling the arena never repeats a slot"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "    BYTES.store(bytes, Ordering::Release);",
        replace: "    BYTES.store(bytes / 2, Ordering::Release);",
        breaks: "a save area is sized for the form that writes it",
        reddens: Caught::Check("a save stays inside its area"),
    },
    Mutation {
        file: "kernel/src/object.rs",
        find: "            if let Some(message) = self.receive() {
                return Some(message);
            }
            if self.is_orphaned() {",
        replace: "            if self.is_orphaned() {
                return None;
            }
            if let Some(message) = self.receive() {
                return Some(message);
            }
            if false {",
        breaks: "a closing channel is drained before it is refused",
        reddens: Caught::Check("a closed channel still yields what it was sent"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "    \"verw word ptr [rsp]\",",
        replace: "",
        breaks: "the buffer flush is the instruction, not its neighbour",
        reddens: Caught::Check("the return to ring three clears buffers"),
    },
    Mutation {
        file: "kernel/src/object.rs",
        find: "            far.waiters.wake_all();",
        replace: "            let _ = far;",
        breaks: "a blocked receiver learns its peer is gone",
        reddens: Caught::Check("a blocked receiver learns its peer is gone"),
    },
    // D-182 replaced D-128's on-demand growth with a whole-table commitment,
    // so its mutation replaces the old growth mutation too. Charging one slot
    // too few leaves the final insertion with no prepaid room: exactly the
    // boundary that would otherwise allocate or panic in trap context.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let id = commit_table(quota)?;",
        replace: "        let id = commit_table(quota.saturating_sub(1))?;",
        breaks: "a table's final quota slot is backed before any trap can use it",
        reddens: Caught::Check("the handle arena backs a table's whole quota"),
    },
    // Slice 1's four properties, one defect each (D-119).
    //
    // Duplicate made the ring a same-object delegation path in Phase 6, and
    // that path must narrow exactly as channel transfer does. This is the whole
    // of it: `install` hands out what was offered instead of the narrower of
    // offered and declared. Cross-type OpenAt derivation has its own rule.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.fill(object, granted)?;",
        replace: "        let granted = offered;
        let _ = declared;
        if !granted.are_meaningful_for(object.kind().per_type_rights()) {
            return Err(HandleError::Meaningless);
        }
        let handle = self.fill(object, granted)?;",
        breaks: "rights attenuate on arrival",
        reddens: Caught::Check("a completion carries a handle with narrowed rights"),
    },
    // A reserved file right admitted before any file operation consults it.
    // An inert admitted right is a syscall name in disguise (D-78).
    Mutation {
        file: "kernel/src/object.rs",
        find: "            Self::File => Rights::READ,",
        replace: "            Self::File => Rights::READ.union(Rights::WRITE),",
        breaks: "a file refuses a right no operation consults",
        reddens: Caught::Check("a meaningless right is refused"),
    },
    // The same failure on the namespace side: MODIFY exists in the raw word,
    // but no operation currently makes it meaningful for a directory.
    Mutation {
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
        file: "kernel/src/queue.rs",
        find: "    pub(super) const ARGUMENT: usize = 24;",
        replace: "    pub(super) const ARGUMENT: usize = 8;",
        breaks: "the emitted program and kernel use one submission layout",
        reddens: Caught::Check("a program drives its own ring"),
    },
    // The destructor that returns an unspent promise, removed. Not a crash:
    // a table that quietly shrinks by one every time a program asks for
    // something it cannot have — `012` §10's `-ENFILE`-at-completion, arrived
    // at from the other direction.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        if !self.spent {
            self.table.step(Quota::release);
        }",
        replace: "        if false {
            self.table.step(Quota::release);
        }",
        breaks: "a refused operation gives its promised slot back",
        reddens: Caught::Check("a refused handle operation returns its promise"),
    },
    // A deferred operation made due the moment it is admitted, which is what
    // "asynchronous" degrades into if nothing checks the deadline. The
    // completion still arrives exactly once, so only a check that looks
    // *before* the deadline can tell the two kernels apart.
    Mutation {
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
    // The cancellation stops posting the target's completion and reports only
    // its own — the double-completion class from `012` §8 with the sign
    // flipped, and the reason cancellation is an operation rather than a
    // substitute.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        replace: "        if false && !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
            // The cancellation itself still gets the completion reserved for",
        breaks: "a cancelled operation still gets its own completion",
        reddens: Caught::Check("a cancelled operation and its cancellation complete separately"),
    },
    // Slice 2, one defect per property (D-119).
    //
    // The published shift moved by one, which halves or doubles every reading
    // a program takes while the syscall keeps answering correctly. Silent: the
    // numbers stay plausible and only a comparison between the two doors shows
    // it.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift());",
        replace: "    write_u32(page, CLOCK_AT + clock::SHIFT, conversion.shift() - 1);",
        breaks: "the page and the kernel clock tell the same time",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // The window left open: the counter goes odd and never comes back, so
    // every correct reader spins for ever and gives up.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));",
        replace: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));",
        breaks: "a published clock closes its update window",
        reddens: Caught::Check("the time page and the kernel clock tell the same time"),
    },
    // The revocation gate ignored. A kernel that has stopped trusting its
    // counter goes on telling every program the counter is good, which is the
    // one failure the gate exists to prevent.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::MODE, mode);",
        replace: "    write_u32(page, CLOCK_AT + clock::MODE, MODE_COUNTER);",
        breaks: "a revoked clock reads as revoked",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    // The time page mapped writable, which is a program able to tell every
    // other program a lie about the time.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE);",
        replace: "    let result = space.map_shared(TIME, &shared, Flags::NO_EXECUTE.union(Flags::WRITABLE));",
        breaks: "the time page is read-only to ring 3",
        reddens: Caught::Check("a program sees the time page and cannot write it"),
    },
    // Slice 3. Teardown that forgets an operation rather than finishing it:
    // no completion, and the reservation held for ever. Not a crash — a ring
    // whose accounting says work is outstanding after the program is gone,
    // which is invisible until something asks (D-167).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            if !self.settle_pending(index, entry, STATUS_CANCELLED, 0) {
                break;
            }
            ended += 1;",
        replace: "            let _ = entry;
            self.pending.lock_leaf().slots[index] = None;
            ended += 1;",
        breaks: "a dead program's work is finished, not forgotten",
        reddens: Caught::Check("work in flight is ended before the ring is released"),
    },
    // A promise made outside trap context is given a table of its own, so the
    // one it was made against dies under it. The trap arm remains allocation-
    // free; otherwise D-182's earlier panic would hide the use-after-free
    // D-163's `Arc` exists to prevent.
    Mutation {
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
        reddens: Caught::Check("a promise outlives the program that made it"),
    },
    // Slice 4. A bus zero scan that calls itself an enumeration: every
    // function q35 has is on bus zero, so the count stays plausible and only
    // a check that counts *buses* can tell the two kernels apart (D-168).
    Mutation {
        file: "kernel/src/pci.rs",
        find: "                if entry.class.is_bridge()
                    && entry.header.is_bridge()",
        replace: "                if false
                    && entry.header.is_bridge()",
        breaks: "enumeration descends through bridges",
        reddens: Caught::Check("the bus walk descends through a bridge"),
    },
    // Two offsets in one header treated as two device identities. Exact-offset
    // duplicate checks cannot catch this; the claim must canonicalise first.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "    let at = at.function_base();
    let mut claimed = CLAIMED.lock(context);",
        replace: "    let at = at;
    let mut claimed = CLAIMED.lock(context);",
        breaks: "configuration offsets alias one function claim",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // The ports a device decodes left free for anyone to claim, which is the
    // arrangement D-77 said proves the wrong thing.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "                let owned =
                    wost::arch::current::port::claim_spans::<BARS>(&requested[..port_count])",
        replace: "                let owned =
                    wost::arch::current::port::claim_spans::<BARS>(&requested[..0])",
        breaks: "a device claim covers the ports it decodes",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // Consume the dynamic span handles without returning their bitmap bits.
    // The real QEMU I/O-window witness releases and reacquires the same BDF,
    // so registry-only cleanup cannot make this pass.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "        if !wost::arch::current::port::release_spans(ports) {
            return Err(ReleaseError::Ports);
        }",
        replace: "        drop(ports);",
        breaks: "a transient pci lease returns every dynamic io span",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // A bridge's bus numbers decoded as a base address and then probed —
    // all ones written into the register that says which bus is behind it.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "            let registers = if layout.is_bridge() {
                pci_config::BRIDGE_BARS
            } else {
                BARS
            };",
        replace: "            let registers = BARS;
            let _ = layout;",
        breaks: "a bridge has two base address registers, not six",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // Decode left enabled while BARs temporarily contain all ones. A fake
    // device observes the command cycle, where real QEMU hardware may forgive
    // it and make a boot-only mutation vacuous.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let decode_off = command & !(command::IO_SPACE | command::MEMORY_SPACE);",
        replace: "    let decode_off = command;",
        breaks: "bar probing disables both decode spaces",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // The high half is no longer all ones when the low mask is read.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if wide {
            modified[index + 1] = true;",
        replace: "        if false && wide {
            modified[index + 1] = true;",
        breaks: "both halves of a wide bar are probed together",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // A successful probe returns while the device still holds probe values.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    if let Some(error) = restore(&mut access, &original, &modified, command, command_modified) {",
        replace: "    if false
        && let Some(error) = restore(&mut access, &original, &modified, command, command_modified)
    {",
        breaks: "a successful bar probe restores every register",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bar_probe_is_one_restored_decode_off_transaction",
        },
    },
    // Rollback restores BARs but forgets the command register after an access
    // fails; exhaustive cycle injection reaches every such exit.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    match restore(access, original, modified, command, command_modified) {",
        replace: "    match restore(access, original, modified, command, false) {",
        breaks: "a failed bar probe restores decode state",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_probe_access_failure_restores_the_device",
        },
    },
    // Record each requested run before the all-or-none preflight finishes.
    // A conflict later in the list then strands all earlier runs.
    Mutation {
        file: "libs/port-space/src/lib.rs",
        find: "            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        replace: "            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);
            }
            room[index] = Some(span);
        }

        for span in room.iter().flatten() {
            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a refused multi-span reservation records no earlier span",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_refused_multi_span_reservation_takes_nothing",
        },
    },
    // Keep the unique handles consumed but leave their bitmap bits set.
    Mutation {
        file: "libs/port-space/src/lib.rs",
        find: "                self.reserved[port / PORTS_PER_WORD] &= !bit(port);",
        replace: "                self.reserved[port / PORTS_PER_WORD] |= bit(port);",
        breaks: "a transient multi-span release clears every exact port",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_multi_span_release_is_exact_and_all_or_none",
        },
    },
    // An address keeps its register offset when asked for its device identity.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        Self(self.0 & !0xFF)",
        replace: "        Self(self.0)",
        breaks: "a function identity contains no register offset",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_offset_of_one_function_has_one_identity",
        },
    },
    // Report release while leaving the canonical function slot occupied.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        *slot = None;
        true",
        replace: "        *slot = Some(at);
        true",
        breaks: "a transient pci claim returns its exact registry slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_claim_release_is_exact_and_makes_room_for_the_same_function",
        },
    },
    // A wide BAR in a bridge's last slot would consume its bus-number field.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        let wide = original[index] & 0b110 == 0b100;
        if wide && index + 1 == registers {
            return Err(ProbeError::Malformed);
        }",
        replace: "        let wide = original[index] & 0b110 == 0b100;
        if false && wide && index + 1 == registers {
            return Err(ProbeError::Malformed);
        }",
        breaks: "a wide bar cannot consume the field after the header's last slot",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_wide_bar_in_the_final_slot_is_refused_before_mutation",
        },
    },
    // A non-power-of-two mask happens to be aligned and otherwise plausible.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if !size.is_power_of_two()
            || !base.is_multiple_of(size)",
        replace: "        if false
            || !base.is_multiple_of(size)",
        breaks: "a bar window size is a power of two",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::an_invalid_bar_mask_is_refused_after_restoration",
        },
    },
    // The first refused restore aborts rollback instead of continuing through
    // the command register and every older BAR.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        if modified[index]
            && let Err(error) = access(ProbeAccess::WriteBar(index, original[index]))
            && failed.is_none()
        {
            failed = Some(error);
        }",
        replace: "        if modified[index]
            && let Err(error) = access(ProbeAccess::WriteBar(index, original[index]))
            && failed.is_none()
        {
            failed = Some(error);
            return failed;
        }",
        breaks: "bar rollback continues after a restore is refused",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_restore_failure_still_attempts_every_later_restore",
        },
    },
    // Record the BDF before its fallible device/resource preparation. Any
    // later refusal then poisons a permanent registry slot.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "        let prepared = prepare().map_err(RegistryError::Prepare)?;
        self.slots[room] = Some(at);",
        replace: "        self.slots[room] = Some(at);
        let prepared = prepare().map_err(RegistryError::Prepare)?;",
        breaks: "a refused device preparation leaves its registry unchanged",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_refused_claim_preparation_leaves_the_registry_unchanged",
        },
    },
    // Require memory decode but omit bus master from the transaction. MMIO
    // works and every DMA queue then waits forever without an error register.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let required = command::MEMORY_SPACE | command::BUS_MASTER;",
        replace: "    let required = command::MEMORY_SPACE;",
        breaks: "bus master enable preserves and verifies every required command bit",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::bus_master_enable_is_verified_and_preserves_the_command",
        },
    },
    // Treat a successful configuration cycle as proof the device accepted the
    // command bits, skipping the readback which distinguishes the two claims.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "            Some(bits) if bits as u16 & wanted == wanted => return Ok(CommandState(original)),",
        replace: "            Some(_) => return Ok(CommandState(original)),",
        breaks: "a bus master command write is read back before success",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_refused_bus_master_enable_restores_the_exact_command",
        },
    },
    // Read the captured command word back without first restoring it.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(CommandAccess::Write(state.0));
    match access(CommandAccess::Read) {",
        replace: "    let _ = &mut access;
    match access(CommandAccess::Read) {",
        breaks: "a successful pci lease restores its captured command word",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_bus_master_lease_restores_the_exact_command_or_reports_uncertainty",
        },
    },
    // Return the initiating access error without restoring a command write
    // which may already have enabled only part of the device.
    Mutation {
        file: "libs/pci-config/src/lib.rs",
        find: "    let _ = access(CommandAccess::Write(original));\n    match access(CommandAccess::Read) {",
        replace: "    return Err(failure);\n    #[expect(unreachable_code)]\n    match access(CommandAccess::Read) {",
        breaks: "every failed bus master access restores or reports uncertainty",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::every_failed_bus_master_access_restores_or_reports_uncertainty",
        },
    },
    // Preserve the warning but continue into BAR admission after the command
    // transaction failed, which was the original PW-206 defect.
    Mutation {
        file: "kernel/src/main.rs",
        find: "    if let Err(reason) = device.enable_bus_master() {\n        warn!(\"nvme: memory space and bus master refused: {reason}\");\n        return None;\n    }",
        replace: "    if let Err(reason) = device.enable_bus_master() {\n        warn!(\"nvme: memory space and bus master refused: {reason}\");\n    }",
        breaks: "failed bus master enable stops before nvme mapping",
        reddens: Caught::Check("a failed bus master enable stops before nvme mapping"),
    },
    // Admit an empty read to the zero-based NLB encoder. In release that
    // underflows to the largest command rather than panicking.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        breaks: "an empty transfer has no nlb encoding",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::an_empty_transfer_has_no_nlb_encoding",
        },
    },
    // Admit the first count whose zero-based encoding no longer fits the
    // command's sixteen-bit NLB field.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        replace: "        if false && blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        breaks: "the nlb field has one exact maximum block count",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_nonempty_nlb_count_has_one_exact_boundary",
        },
    },
    // Wrap the LBA end back into the namespace instead of refusing the
    // overflow which would make a controller reach an unrelated block.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        let Some(end) = lba.checked_add(blocks as u64) else {\n            return Err(TransferError::OutsideNamespace);\n        };",
        replace: "        let end = lba.wrapping_add(blocks as u64);",
        breaks: "a transfer extent cannot wrap into its namespace",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_transfer_never_crosses_or_wraps_its_namespace",
        },
    },
    // Let the device write one byte past the allocation that proves the DMA
    // extent. Exact-size and short buffers differ only at this comparison.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        replace: "        if false && bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        breaks: "a transfer fits wholly inside its dma allocation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::transfer_bytes_must_fit_the_dma_buffer",
        },
    },
    // Replace the controller-bound validation with one known-good request.
    // The self-test submission interceptor catches every malformed caller at
    // the final seam without ringing the real device's doorbell.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        let request = self.validate_read(lba, blocks, buffer_bytes)?;",
        replace: "        let request = self\n            .validate_read(lba, blocks, buffer_bytes)\n            .or_else(|_| {\n                ReadTransfer::new(\n                    0,\n                    1,\n                    self.blocks,\n                    u64::from(self.block_bytes),\n                    u64::MAX,\n                )\n                .map_err(BringUpError::InvalidTransfer)\n            })?;",
        breaks: "raw nvme reads cannot bypass controller-bound validation",
        reddens: Caught::Check("invalid nvme reads stop before submission"),
    },
    // Treat a zeroed Identify response as a usable namespace. There is no LBA
    // zero when NSZE is zero, however plausible the remaining format looks.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        breaks: "a namespace has at least one logical block",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_namespace_has_at_least_one_logical_block",
        },
    },
    // Accept the extended format table even though this host never enabled
    // LBAFEE and copied only the sixteen-entry prefix it negotiated.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        replace: "        if false && last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        breaks: "only negotiated lba formats are accepted",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Ignore FLBAS's reserved bit, making a malformed selector indistinguishable
    // from a format this host knows how to interpret.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        replace: "        if false && formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        breaks: "reserved formatted lba selector bits are refused",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Select a packed table entry beyond zero-based NLBAF. Its bytes are not a
    // supported format even when the fixed Identify buffer happens to hold it.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        replace: "        if false && selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        breaks: "the formatted lba index is inside the reported table",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Interpret reserved LBAF control bits as relative-performance bits. The
    // low two are descriptive; every higher bit is an unknown layout contract.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        replace: "        if false && control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        breaks: "every lba format control bit has an explicit policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lba_format_control_value_has_one_explicit_policy",
        },
    },
    // Drop per-LBA metadata from the transfer length and pointer contract.
    // Even one byte makes the controller's data shape differ from the host's.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if false && metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "selected lba metadata is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Confuse support capabilities with active metadata settings. A device may
    // advertise both transfer forms while the selected LBAF uses no metadata.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if metadata != 0 || identify[27] != 0 || identify[28] != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "metadata capabilities do not enable metadata settings",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Read protected blocks without providing or checking their protection
    // information, turning integrity errors into ordinary data.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        replace: "        if false && protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        breaks: "enabled protection information is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Admit every exponent at or above 512 bytes, restoring the old trust in a
    // byte which may be 32 and which the only storage caller cannot consume.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        if block_shift != SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        replace: "        if block_shift < SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        breaks: "every lbads byte has one explicit supported policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lbads_value_has_one_explicit_policy",
        },
    },
    // Replace refused device data with a known-good synthetic namespace. The
    // self-test queue interceptor catches the bypass before allocation or MMIO.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        let namespace = Namespace::parse(identify).map_err(BringUpError::InvalidNamespace)?;",
        replace: "        let namespace = Namespace::parse(identify)\n            .or_else(|_| {\n                let mut accepted = [0u8; NAMESPACE_IDENTIFY_BYTES];\n                accepted[0] = 1;\n                accepted[128 + 2] = 9;\n                Namespace::parse(&accepted)\n            })\n            .map_err(BringUpError::InvalidNamespace)?;",
        breaks: "invalid namespace data cannot bypass io queue admission",
        reddens: Caught::Check("invalid nvme namespace stops before io queues"),
    },
    // Slice 5. The logical block address dropped, so every read answers with
    // block zero. Invisible against a disk of identical blocks, which is why
    // the namespace carries a different signature in each (D-169).
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            .dword(10, lba as u32)
            .dword(11, (lba >> 32) as u32)",
        replace: "            .dword(10, 0)
            .dword(11, 0)",
        breaks: "a read returns the block that was asked for",
        reddens: Caught::Check("a block read returns the block that was asked for"),
    },
    // The block count sent unadjusted, so one block becomes two. The first
    // block is still exactly right; only the bytes past it say so.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            .dword(12, u32::from(request.zero_based_blocks()));",
        replace: "            .dword(12, u32::from(request.zero_based_blocks()) + 1);",
        breaks: "every count in this specification is zero-based",
        reddens: Caught::Check("a block read returns the block that was asked for"),
    },
    // The phase tag never flips. Works perfectly until the queue wraps, and
    // then sees either every entry or none — so the check has to do more
    // commands than the queue has entries to reach it at all.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "                    if head == 0 {
                        queue.phase = !queue.phase;
                    }",
        replace: "                    if false {
                        queue.phase = !queue.phase;
                    }",
        breaks: "a completion queue wraps and stays readable",
        reddens: Caught::Check("the queues wrap and the phase tag still works"),
    },
    // A register file mapped write-back, which an emulator forgives and
    // hardware does not (`012` §2).
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.map_device_with(physical, len, Flags::NO_CACHE)",
        replace: "        self.map_device_with(physical, len, Flags::NONE)",
        breaks: "a device window is uncacheable",
        reddens: Caught::Check("the controller's registers are uncacheable"),
    },
    // Slice 6. The cluster bias forgotten, so every file is read two clusters
    // late — with no error anywhere, because the sectors are real and the
    // bytes are somebody else's (D-170).
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let index = (cluster - FIRST_DATA_CLUSTER) as u64;",
        replace: "        let index = cluster as u64;",
        breaks: "a cluster number is biased by two",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // A read bounded by the cluster rather than by the file, which hands back
    // the last cluster's tail: its previous occupant's data.
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let wanted = into.len().min((entry.size - offset) as usize);",
        replace: "        let wanted = into.len();",
        breaks: "a read stops at the end of the file",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The deleted marker taken for the end of the directory, which loses every
    // file below it. Only reachable because the corpus deletes one on purpose.
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }",
        replace: "    if slot[0] == 0xE5 {
        return Slot::End;
    }",
        breaks: "a deleted entry is not the end of a directory",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The volume mounted at sector zero, which is where a reader that could
    // not be given an offset would look.
    Mutation {
        file: "kernel/src/fs.rs",
        find: "pub(crate) const VOLUME_SECTOR: u64 = 2048;",
        replace: "pub(crate) const VOLUME_SECTOR: u64 = 0;",
        breaks: "the volume is where the partition puts it",
        reddens: Caught::Check("the volume mounts and says what a fat32 volume says"),
    },
    // Slice 7. `..` passed through as an ordinary name, so the resolver is
    // asked to walk upward — which is the one thing `012` §5 says it must not
    // be *able* to do (D-171).
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\"..\" {",
        replace: "        if false {",
        breaks: "a resolver cannot be asked to ascend",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // Ascending past the start clamped rather than refused, which silently
    // answers a different question than the caller asked.
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "an escape is refused rather than clamped",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // A component that is not a directory walked through, so a file's bytes
    // are read as directory entries — with the caller choosing the bytes.
    Mutation {
        file: "kernel/src/vfs.rs",
        find: "                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }",
        replace: "                if false {
                    return Err(FatError::WrongKind);
                }",
        breaks: "a file cannot be walked through",
        reddens: Caught::Check("a file cannot be walked through"),
    },
    // Slice 8. A tmpfs lookup that ignores the parent, so every directory
    // holds every file. Invisible in a tree whose names are unique, which is
    // why this one holds `MOTD` twice (D-172).
    Mutation {
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
        file: "kernel/src/tmpfs.rs",
        find: "    let wanted = into.len().min(len - offset);",
        replace: "    let wanted = into.len();",
        breaks: "a read stops at the end of the file, in memory too",
        reddens: Caught::Check("a second backend answers the same three operations"),
    },
    // Slice 9. An open that ignores the directory's traverse right, so a
    // handle held only to *list* a directory can walk it (D-173).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if !held.contains(Rights::TRAVERSE) {
            return refuse(token, STATUS_DENIED);
        }",
        replace: "        if false {
            return refuse(token, STATUS_DENIED);
        }",
        breaks: "an open needs the right to traverse",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // A file handle granted WRITE, which nothing can honour: there is no write
    // path, and a right that grants nothing is a syscall name in disguise.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ),
            ),",
        replace: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ).union(Rights::WRITE),
            ),",
        breaks: "an open grants only what it can honour",
        reddens: Caught::Check("a completion carries a file handle"),
    },
    // A file opened through an attenuated directory regains the universal
    // rights its parent gave up. TRAVERSE authorizes READ, not re-delegation.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                held.intersection(Rights::UNIVERSAL).union(Rights::READ),
            ),",
        replace: "                Rights::UNIVERSAL.union(Rights::READ),
            ),",
        breaks: "an open never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // The same widening defect on the other result branch. A directory is a
    // copy value now, but copying its identity must not copy rights its parent
    // did not hold.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            crate::vfs::Node::Directory(directory) => (crate::object::directory(directory), held),",
        replace: "            crate::vfs::Node::Directory(directory) => (
                crate::object::directory(directory),
                Rights::UNIVERSAL.union(held),
            ),",
        breaks: "an opened directory never restores a universal right",
        reddens: Caught::Check("opening a node does not restore universal rights"),
    },
    // A malformed OpenAt path is inspected before its result slot is promised.
    // With a full table the operation must report quota first, just as
    // Duplicate does; otherwise handle-producing opcodes have different
    // admission contracts.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };
        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return refuse(token, STATUS_BAD_BUFFER);
        };",
        replace: "        let mut path = [0u8; MAX_PATH];
        let Some(len) = self.read_data(submission.argument, submission.length, &mut path) else {
            return refuse(token, STATUS_BAD_BUFFER);
        };
        let Ok(reservation) = table.reserve_slot() else {
            return refuse(token, STATUS_NO_HANDLE_SLOT);
        };",
        breaks: "every handle-producing operation reserves its result before other admission work",
        reddens: Caught::Check("a full table refuses a handle operation at submission"),
    },
    // Duplicate's rights validation pulled ahead of the result promise. A
    // full table must report quota even when the submitted rights are
    // malformed, or Duplicate and OpenAt expose different admission orders.
    Mutation {
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
    // A read that ignores its offset, so every read answers from the start.
    // Offsets are per operation and nothing holds a cursor, so this is the
    // only thing that says where the bytes came from.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        match crate::vfs::read(request.file, request.offset, &mut scratch[..request.wanted]) {",
        replace: "        match crate::vfs::read(request.file, 0, &mut scratch[..request.wanted]) {",
        breaks: "a read starts where it was told to",
        reddens: Caught::Check("a read is a submission with its own offset"),
    },
    // The buffer bound moved after the read. **Changes no output** — the
    // shared page bounds the copy anyway — so only the driver's command
    // counter can tell the two kernels apart, which is why it exists (D-173).
    Mutation {
        file: "kernel/src/queue.rs",
        find: "    (end <= DATA_BYTES).then_some((at, len))",
        replace: "    let _ = end;
    Some((at, len))",
        breaks: "a buffer is checked before the device is asked",
        reddens: Caught::Check("a ring operation cannot reach past its rights or its pages"),
    },
    // The phase-close audit's finding, one defect in three places (D-174).
    // A rights word read as its low half, so every undefined bit a program
    // sets is dropped rather than refused.
    Mutation {
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
        file: "kernel/src/queue.rs",
        find: "        let offset = u32::try_from(submission.offset).unwrap_or(u32::MAX);",
        replace: "        let offset = submission.offset as u32;",
        breaks: "an offset that does not fit is past the end",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // A cancellation token truncated, so a program ends an operation it did
    // not name.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        let Ok(target) = u32::try_from(submission.argument) else {
            return refuse(submission.token, STATUS_NOT_FOUND);
        };",
        replace: "        let target = submission.argument as u32;",
        breaks: "a token that does not fit names no operation",
        reddens: Caught::Check("a word that does not fit is refused rather than truncated"),
    },
    // D-133's exemption, put back. The woken waiter still runs on the slice
    // already armed for the program, so only the counter can distinguish the
    // defect from a working syscall wake.
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if woken > 0 {
        if interrupt::in_trap() {",
        replace: "    if woken > 0 && !interrupt::in_trap() {
        if interrupt::in_trap() {",
        breaks: "a syscall wake arms the timer for what it woke",
        reddens: Caught::Check("a syscall wake arms the timer for what it woke"),
    },
    // A discriminant and its `match` arm drifting apart, which is how a
    // diagnosis comes to name the wrong guard (D-176).
    Mutation {
        file: "kernel/src/time.rs",
        find: "            6 => Self::Disagreed,",
        replace: "            6 => Self::NotBelievable,",
        breaks: "the clock's refusal decodes to the one that was recorded",
        reddens: Caught::Check("every refusal the clock can record reads back as itself"),
    },
    // A run offered from a region that is described but is not memory, which
    // is the hole the four probes fell into on VMware (D-177).
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "            .filter(|region| region.kind.is_direct_mapped())",
        replace: "            .filter(|region| !region.kind.is_direct_mapped())",
        breaks: "a probed run is memory, not merely described",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::every_accepted_region_informs_memory_policy",
        },
    },
    // Launch the shipped ring-3 program on its Echo route. The image and all
    // completion validation remain intact; only the machine check's handle
    // count proves that this launch really exercised Duplicate through the
    // trap. The target is kernel source because the running mutation driver
    // cannot rebuild itself before `program::build` writes the image.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-duplicate\", 6, DUPLICATE)",
        replace: "    spawn(\"user-duplicate\", 6, BEHAVE)",
        breaks: "the shipped program uses its bootstrap authority through Duplicate",
        reddens: Caught::Check("a program duplicates its bootstrap handle"),
    },
    // The identity read as a translating controller would rewrite it, which is
    // the state the check exists to catch (D-178).
    Mutation {
        file: "kernel/src/selftest.rs",
        find: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x83);",
        replace: "const UNTRANSLATED_MF2: (u8, u8) = (0xAB, 0x41);",
        breaks: "the keyboard's identity is the untranslated one",
        reddens: Caught::Check("the keyboard identifies as one the decoder knows"),
    },
    // Set 1's code for the key the translated round presses. Green with this
    // applied until the round stopped pressing the same key as the first one
    // (D-155, D-179).
    Mutation {
        file: "libs/ps2-keys/src/set1.rs",
        find: "        0x30 => Key::B,",
        replace: "        0x30 => Key::Pause,",
        breaks: "set 1's numbering is the specification's",
        reddens: Caught::Check("a translated keystroke decodes to the same key"),
    },
    // The decoder left behind when the controller's numbering changes under
    // it. Taking the decoder from the *bit* instead of the identity was the
    // first draft of this mutation and could not be caught: on QEMU the bit
    // and the identity agree, so only a machine that lies about one would tell
    // the two apart, and no gate boot does (D-175, D-179).
    Mutation {
        file: "kernel/src/input.rs",
        find: "    keyboard.decoder = decoder_for(&mut keyboard.controller);",
        replace: "    let _ = &mut keyboard.controller;",
        breaks: "the decoder is re-derived when translation changes",
        reddens: Caught::Check("a translated keystroke decodes to the same key"),
    },
    // Treat an unverified stop as a verified one. A fake device completes
    // later, after the allocator would already have been allowed to reuse the
    // command's memory.
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = if verified {\n            DmaOwnership::Quiesced\n        } else {\n            DmaOwnership::Retained\n        };",
        replace: "        self.state = DmaOwnership::Quiesced;\n        let _ = verified;",
        breaks: "an unverified controller stop retains its dma memory",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::delayed_completion_cannot_recycle_unquiesced_dma_memory",
        },
    },
    // Return a retained DMA segment to the frame allocator. Safe access is
    // still gone, so only the allocator count can distinguish this defect.
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        if let Some(segment) = self.segment.take() {\n            core::mem::forget(segment);\n        }",
        replace: "        if let Some(segment) = self.segment.take() {\n            drop(segment);\n        }",
        breaks: "retained dma memory never returns to the allocator",
        reddens: Caught::Check("uncertain dma memory is never recycled"),
    },
    // The original defect: report timeout directly while the controller still
    // has the command's physical addresses. The machine check repairs the
    // deliberately outstanding command only after recording the red result.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "            core::hint::spin_loop();\n        }\n        self.fail_submitted(BringUpError::NotReady, data)",
        replace: "            core::hint::spin_loop();\n        }\n        Err(BringUpError::NotReady)",
        breaks: "a command timeout settles dma before returning",
        reddens: Caught::Check("an nvme timeout quiesces before dma memory returns"),
    },
    // Re-export the private closure that can hand any safe kernel module the
    // running `AddressSpace`, including its reclaim and arbitrary-unmap doors.
    Mutation {
        file: "kernel/src/mem.rs",
        find: "fn with_active_space<T>(",
        replace: "pub(crate) fn with_active_space<T>(",
        breaks: "the running address space has no broad safe borrow",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::the_running_space_has_no_broad_safe_borrow",
        },
    },
    // Forget every ownership entry after D-117's old fixed ceiling. The table
    // frames remain live and reachable, but reclamation silently loses them.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "for frame in self.tables.drain(..) {",
        replace: "for frame in self.tables.drain(..).take(32) {",
        breaks: "a large address space returns every table frame",
        reddens: Caught::Check("a large space returns every table frame"),
    },
    // Turn the transaction's terminal table release into a refusal. Every
    // earlier resource still drops, so the complete ledger is what notices.
    Mutation {
        file: "kernel/src/user.rs",
        find: "        let table_result = space.reclaim(released);",
        replace: "        let table_result = Err(ReclaimError::Reclaimed);",
        breaks: "a refused process construction returns its page tables",
        reddens: Caught::Check("every refused spawn returns every resource"),
    },
    // Admit an entry outside every executable segment. The image is otherwise
    // valid, so this reaches mapping unless the entry participates in preflight.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        replace: "    if false && !entry_is_executable {\n        return Err(LoadError::InvalidEntry(entry));\n    }",
        breaks: "an ELF entry must be inside executable program memory",
        reddens: Caught::Check("the loader refuses what it may not map"),
    },
    // The original failure found by driving Fabricated twice: retiring the
    // first attempt zeroed the arena stack address reused by the second.
    Mutation {
        file: "wost/src/arch/x86_64/task.rs",
        find: "        let stack_top = table.slots[slot].stack_top;\n        table.slots[slot] = Slot {\n            stack_top,\n            ..Slot::VACANT\n        };",
        replace: "        table.slots[slot] = Slot::VACANT;",
        breaks: "a retired construction preserves its reusable kernel stack",
        reddens: Caught::Panic {
            marker: "address 0xffffffffffffffe8",
            why: "the second fabrication writes through the zeroed stack top before a verdict",
        },
    },
    // Leave the scheduler reservation behind after its launch records refuse
    // publication. `wost` and the process transaction both clean up, so the
    // complete ledger has to notice the otherwise quiet slot loss.
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if !arrange(slot) {\n        let _ = task::retire(slot);\n        THREADS.lock_leaf().release(slot);\n        return Err(SpawnError::Arrangement);\n    }",
        replace: "    if !arrange(slot) {\n        let _ = task::retire(slot);\n        return Err(SpawnError::Arrangement);\n    }",
        breaks: "a refused thread arrangement releases its scheduler slot",
        reddens: Caught::Check("every refused spawn returns every resource"),
    },
    // Consume a pending operation when its reserved terminal post refuses.
    // The accounting still says one completion is owed, so only a witness
    // that inspects ownership before retrying can distinguish the loss.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "            pending.slots[index] = Some(Pending {\n                settling: false,\n                ..entry\n            });\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        replace: "            pending.slots[index] = None;\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        breaks: "a refused terminal post retains its pending operation",
        reddens: Caught::Check("a refused settlement retains every owner"),
    },
    // Continue process construction after the boot stack guard was refused.
    // All later construction remains valid, so only the explicit boundary
    // state distinguishes this from a normal launch.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        replace: "    if false && !crate::user_launch_is_safe() {\n        return Err(SpawnError::UnguardedBootStack);\n    }",
        breaks: "an unguarded boot stack cannot open the user boundary",
        reddens: Caught::Check("an unguarded boot stack closes the user boundary"),
    },
    // Erase the state transition which makes deliberate forgotten ownership
    // observably different from an ordinary holder. The lock still excludes,
    // so the witness must inspect the ownership reason rather than availability.
    Mutation {
        file: "wost/src/sync.rs",
        find: "        self.lock.forgotten_for_test.store(true, Ordering::Release);",
        replace: "        self.lock.forgotten_for_test.store(false, Ordering::Release);",
        breaks: "deliberately forgotten ownership is distinct from ordinary contention",
        reddens: Caught::Check("forgotten lock ownership is distinct from contention"),
    },
    Mutation {
        file: "kernel/src/main.rs",
        find: "    survey_the_bus();\n    survey_the_filesystems();\n    start_interrupts();",
        replace: "    survey_the_bus();\n    if wost::arch::current::pci::is_available() {\n        survey_the_filesystems();\n    }\n    start_interrupts();",
        breaks: "hardware-free namespaces do not depend on PCI availability",
        reddens: Caught::BootMarker(qemu::DEGRADED_FILESYSTEM_MARKER),
    },
    // Leave the failed controller's physical-ownership record behind after
    // removing access. The page tables can still map its sibling, so only the
    // selection ledger says the candidate was not completely retired.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        self.devices[mapping.device_slot] = None;",
        replace: "        self.devices[mapping.device_slot] = Some(expected_extent);",
        breaks: "a failed nvme candidate returns its device-mapping ownership",
        reddens: Caught::Check("a later nvme controller survives an earlier failure"),
    },
    // Remove the mapping but strand its virtual-window allocation. One retry
    // still fits, so the baseline must name the allocator cursor explicitly.
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if mapping
            .virtual_base
            .checked_add(mapping.pages)
            .is_some_and(|end| end == self.device_next)
        {
            self.device_next = mapping.virtual_base;
        }",
        replace: "        let _ = self.device_next;",
        breaks: "a failed nvme candidate returns its device-window address space",
        reddens: Caught::Check("a later nvme controller survives an earlier failure"),
    },
    // Restore everything but stop enumeration anyway, reproducing the
    // first-candidate-only selector without conflating it with cleanup.
    Mutation {
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
    // Publish the queue addresses before moving them into the cleanup owner.
    // The later ordinary enable remains, so the defect compiles and may look
    // harmless on QEMU while an error at this first write loses ownership.
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "    // Cleanup owns every published queue address before the enable write.
    let mut controller = Controller {",
        replace: "    let _ = write(&mut registers, register::CC, cc);
    let mut controller = Controller {",
        breaks: "nvme cleanup owns every published queue before controller enable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::nvme_cleanup_ownership_precedes_the_enable_write",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(_) => Self::Attempts(fallback_attempts),",
        breaks: "a calibrated nvme poll is bounded by real time rather than iteration rate",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_monotonic_poll_limit_expires_by_time_not_iteration_rate",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "            Self::Attempts(remaining) if *remaining > 0 => {",
        replace: "            Self::Attempts(remaining) if *remaining > 1 => {",
        breaks: "a clockless nvme poll spends exactly its declared fallback attempts",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_clockless_poll_limit_permits_its_exact_fallback_count",
        },
    },
    Mutation {
        file: "libs/nvme/src/lib.rs",
        find: "            Some(now) => Self::Deadline(now.saturating_add(timeout_ms.saturating_mul(1_000_000))),",
        replace: "            Some(now) => Self::Deadline(now.wrapping_add(timeout_ms.saturating_mul(1_000_000))),",
        breaks: "an nvme deadline saturates instead of wrapping into the past",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_deadline_saturates_instead_of_wrapping_into_the_past",
        },
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "const WORK_DEPTH: usize = 8;",
        replace: "const WORK_DEPTH: usize = 7;",
        breaks: "file-work admission accepts its exact documented finite capacity",
        reddens: Caught::Check("file work pressure is bounded at admission"),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        matches!(
            self,
            Self::Timer { .. }
                | Self::Read {
                    state: ReadState::Queued,
                    ..
                }
        )",
        replace: "        matches!(self, Self::Timer { .. } | Self::Read { .. })",
        breaks: "a running file read remains owned through teardown until its worker settles",
        reddens: Caught::Check("a delayed read preserves interrupts and exact teardown"),
    },
    Mutation {
        file: "kernel/src/wait.rs",
        find: "        let switched = registered && thread::wait_from_trap(frame, until);",
        replace: "        let switched = registered && false && thread::wait_from_trap(frame, until);",
        breaks: "RingWait hands its caller away through the live trap frame",
        reddens: Caught::Check("a program waits through its trap frame"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        replace: "    if false && !no_execute_is_enabled() {\n        return Err(SpawnError::NoExecuteProtection);\n    }",
        breaks: "ring three requires enforced non-executable mappings",
        reddens: Caught::Check("no-execute protection closes the user boundary"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        replace: "    if false && !crate::time::is_calibrated() {\n        return Err(SpawnError::NoClock);\n    }",
        breaks: "an uncalibrated boot never launches an unpreemptible program",
        reddens: Caught::BootMarker(qemu::DEGRADED_USER_MARKER),
    },
    Mutation {
        file: "kernel/src/queue.rs",
        find: "                if !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        replace: "                if false && !crate::time::is_calibrated() {\n                    return Some(refuse(token, STATUS_UNAVAILABLE));\n                }",
        breaks: "a clockless Timer is refused before it becomes pending",
        reddens: Caught::Check("an uncalibrated clock admits no user or timer"),
    },
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_UNSTARTED, Ordering::Release);\n        return;\n    }",
        replace: "    if spawned.is_err() {\n        IDLE_STATE.store(IDLE_STARTING, Ordering::Release);\n        return;\n    }",
        breaks: "a refused idle spawn leaves its reservation permanently burned",
        reddens: Caught::Check("a failed idle start can be retried exactly once"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        replace: "    if false && !crate::queue::worker_is_ready() {\n        return Err(SpawnError::NoServiceWorker);\n    }",
        breaks: "ring three opens without a thread able to reclaim it",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    if stored {\n        crate::queue::request_reap();\n    }",
        replace: "    let _ = stored;",
        breaks: "retirement leaves the service worker asleep beside a zombie",
        reddens: Caught::Check("a busy machine reaps a program without another spawn"),
    },
    Mutation {
        file: "libs/handle-space/src/lib.rs",
        find: "    identity.checked_add(1)",
        replace: "    Some(identity.wrapping_add(1))",
        breaks: "table identity arithmetic wraps its exhaustion sentinel to zero",
        reddens: Caught::HostTest {
            package: "handle-space",
            name: "tests::table_identity_exhaustion_never_wraps",
        },
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        let successor = handle_space::next_table_identity(self.next_table)\n            .ok_or(HandleError::IdentityExhausted)?;",
        replace: "        let successor = self.next_table.wrapping_add(1);",
        breaks: "handle-table construction crosses the exhausted identity boundary",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        self.next_table = successor;",
        replace: "        if successor != u64::MAX {\n            self.next_table = successor;\n        }",
        breaks: "two simultaneously live handle tables receive one ownership identity",
        reddens: Caught::Check("table identities never wrap or repeat"),
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    model.io_apic_count += 1;",
        replace: "                    model.io_apic_count = 1;",
        breaks: "the MADT inventory discards every I/O APIC after the first",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::a_bounded_interrupt_inventory_keeps_every_controller",
        },
    },
    Mutation {
        file: "libs/acpi-tables/src/lib.rs",
        find: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        return Err(InterruptError::TooManyIoApics { limit: IO_APICS });\n                    };",
        replace: "                    let Some(slot) = model.io_apics.get_mut(model.io_apic_count) else {\n                        break;\n                    };",
        breaks: "an over-capacity MADT is silently truncated into a usable inventory",
        reddens: Caught::HostTest {
            package: "acpi-tables",
            name: "tests::an_interrupt_inventory_refuses_overflow_instead_of_truncating",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "        let Some(distance) = entries.checked_sub(1) else {\n            return None;\n        };",
        replace: "        let distance = entries.saturating_sub(1);",
        breaks: "an I/O APIC reporting zero entries claims its base GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_range_is_non_empty_and_stays_inside_the_gsi_namespace",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "    Ok(ranges.iter().position(|range| range.contains(gsi)))",
        replace: "    Ok(ranges\n        .first()\n        .filter(|range| range.contains(gsi))\n        .map(|_| 0))",
        breaks: "GSI ownership search stops at the first I/O APIC",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::a_gsi_selects_its_later_controller_across_a_gap",
        },
    },
    Mutation {
        file: "libs/x86-apic/src/io.rs",
        find: "            if ranges[first].overlaps(ranges[second]) {",
        replace: "            if ranges[first].overlaps(ranges[second]) && first == second {",
        breaks: "two I/O APICs may ambiguously own the same GSI",
        reddens: Caught::HostTest {
            package: "x86-apic",
            name: "io::tests::any_overlap_refuses_the_complete_inventory",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "a hostile-input parser regains expect, unwrap and panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/port.rs",
        find: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}",
        replace: "pub struct PortSpan {\n    base: u16,\n    len: usize,\n}\n\nimpl PortSpan {\n    pub const fn base(&self) -> u16 {\n        self.base\n    }\n}",
        breaks: "the dynamic port ownership lease regains an uncalled register API",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if first_data_sector >= total_sectors as u64 {",
        replace: "        if false && first_data_sector >= total_sectors as u64 {",
        breaks: "a FAT table extent at or beyond the volume is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::impossible_volume_geometry_is_refused_before_arithmetic",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        if (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        replace: "        if false && (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        breaks: "a FAT root directory may begin outside the complete data clusters",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_root_cluster_must_lie_inside_the_data_area",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "        let skip_clusters = offset / cluster_bytes;\n        for _ in 0..skip_clusters {\n            match self.next_cluster(current)? {\n                Next::Cluster(next) => current = next,\n                Next::End => return Ok(0),\n                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),\n            }\n        }\n\n        let mut done = 0;\n        let mut within = (offset % cluster_bytes) as usize;",
        replace: "        let mut skipped = 0;\n        while skipped + cluster_bytes <= offset {\n            match self.next_cluster(current)? {\n                Next::Cluster(next) => current = next,\n                Next::End => return Ok(0),\n                Next::Bad | Next::Free => return Err(FatError::BadCluster(current)),\n            }\n            skipped += cluster_bytes;\n            if skipped > entry.size {\n                return Ok(0);\n            }\n        }\n\n        let mut done = 0;\n        let mut within = (offset - skipped) as usize;",
        breaks: "a maximum FAT file offset overflows accumulated chain bytes",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_maximum_file_offset_does_not_overflow_chain_progress",
        },
    },
    Mutation {
        file: "tools/fat32-corpus/foreign-fat32.sparse",
        find: "sector 7 ",
        replace: "sector 1 ",
        breaks: "the foreign FAT envelope records one sector twice and omits another",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::foreign_envelope_is_exact_and_hostile_records_are_refused",
        },
    },
    Mutation {
        file: "libs/fat32/src/lib.rs",
        find: "                Next::Cluster(next) => current = next,\n                Next::End => return Ok(()),",
        replace: "                Next::Cluster(_) => return Ok(()),\n                Next::End => return Ok(()),",
        breaks: "a FAT directory walk silently stops after its first cluster",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::both_readers_agree_on_the_adversarial_foreign_layout",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "            workspace: Paths::new(workspace_root, Some(&build)),",
        replace: "            workspace: Paths::new(paths.root().to_path_buf(), Some(&build)),",
        breaks: "mutation builds run from the user checkout instead of the tracked copy",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_session_builds_only_from_its_tracked_copy",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    file.try_lock().with_context(|| {",
        replace: "    std::io::Result::<()>::Ok(()).with_context(|| {",
        breaks: "two mutation sweeps can use one sandbox concurrently",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: "    std::fs::copy(&source, &destination).with_context(|| {\n        format!(\n            \"restoring isolated mutation {} from {}\",\n            destination.display(),\n            source.display()\n        )\n    })?;",
        replace: "    let _ = (&source, &destination);",
        breaks: "resume leaves the interrupted mutation applied in its sandbox",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::a_killed_build_or_boot_resumes_outside_the_user_tree",
        },
    },
    Mutation {
        file: "xtask/src/mutation_workspace.rs",
        find: ".max_by_key(|journal| journal.generation)",
        replace: ".min_by_key(|journal| journal.generation)",
        breaks: "resume chooses stale progress over the newest complete journal generation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutation_workspace::tests::journal_uses_the_newest_complete_slot_and_falls_back_from_a_torn_write",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: "        .env(\"CARGO_TARGET_DIR\", paths.target_dir());",
        replace: "        .env_remove(\"CARGO_TARGET_DIR\");",
        breaks: "isolated cargo builds write outside their private target directory",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::a_cargo_command_uses_the_paths_private_target",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "            marker: \"faulting address 0x0000600000000000\",",
        replace: "            marker: \"\",",
        breaks: "a terminal mutation can name no exact panic cause",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "    result.with_context(|| format!(\"pristine {label} mutation baseline is not green\"))",
        replace: "    let _ = (label, result);\n    Ok(())",
        breaks: "a red pristine baseline is allowed to precede mutation credit",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_red_baseline_stops_before_any_mutation",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed))\n            if failed.iter().any(|actual| actual == name) =>\n        {\n            Ok(true)\n        }",
        replace: "        (Caught::Check(_), qemu::SelftestOutcome::Failed(failed))\n            if !failed.is_empty() =>\n        {\n            Ok(true)\n        }",
        breaks: "an unrelated failed check is credited to the intended mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_wrong_machine_failure_is_inconclusive_not_caught",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked(line))\n            if line.contains(marker) =>\n        {\n            Ok(true)\n        }",
        replace: "        (Caught::Panic { .. }, qemu::SelftestOutcome::Panicked(line))\n            if !line.is_empty() =>\n        {\n            Ok(true)\n        }",
        breaks: "an unrelated kernel panic is credited to a terminal mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::a_panic_requires_its_exact_marker",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        panic.push_str(line.trim_end());",
        replace: "        panic.push_str(PANIC_MARKER);",
        breaks: "a panic continuation is discarded before mutation attribution",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_panic_outcome_carries_its_exact_serial_transcript",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if observation.failed.len() != verdict.failed {",
        replace: "    if false && observation.failed.len() != verdict.failed {",
        breaks: "a failed verdict may omit the names of its red checks",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_failed_verdict_must_name_every_failed_check",
        },
    },
    Mutation {
        file: "libs/eevdf/src/lib.rs",
        find: "        if self.slice_is_spent() {\n            return 0;\n        }",
        replace: "        if false && self.slice_is_spent() {\n            return 0;\n        }",
        breaks: "a thread past its deadline wraps into an enormous remaining slice",
        reddens: Caught::HostTest {
            package: "eevdf",
            name: "tests::a_thread_past_its_deadline_has_no_slice_left",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "        match self.offset.checked_add(self.file_size) {\n            Some(end) => end <= bytes as u64,\n            None => false,\n        }",
        replace: "        self.offset.wrapping_add(self.file_size) <= bytes as u64",
        breaks: "a segment extent wraps around into the elf image",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::a_loader_can_ask_whether_the_contents_fit",
        },
    },
    Mutation {
        file: "libs/frame-space/src/lib.rs",
        find: "        for frame in first..end {\n            self.check_releasable(frame)?;\n        }",
        replace: "        self.check_releasable(first)?;",
        breaks: "a multi-frame free mutates before its complete run is valid",
        reddens: Caught::HostTest {
            package: "frame-space",
            name: "tests::freeing_a_run_is_all_or_nothing",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "        if end > region_len {",
        replace: "        if end > region_len.saturating_add(1) {",
        breaks: "an mmio access may end one byte beyond its claimed region",
        reddens: Caught::HostTest {
            package: "mmio-range",
            name: "tests::a_span_one_byte_past_the_region_is_refused",
        },
    },
    Mutation {
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "ascending above a directory handle is clamped to its root",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::ascending_past_the_start_is_refused_not_clamped",
        },
    },
    Mutation {
        file: "libs/pixel/src/lib.rs",
        find: "        let Some(span) = pitch.checked_mul(height) else {",
        replace: "        let Some(span) = row_bytes.checked_mul(height) else {",
        breaks: "a padded framebuffer mapping omits every row's padding",
        reddens: Caught::HostTest {
            package: "pixel",
            name: "tests::a_padded_framebuffer_spans_its_pitch",
        },
    },
    Mutation {
        file: "libs/ps2-keys/src/decode.rs",
        find: "            Set::One => protocol::is_reply(byte) && byte != protocol::SELF_TEST_PASSED,",
        replace: "            Set::One => protocol::is_reply(byte),",
        breaks: "set one swallows a left-shift release as a controller reply",
        reddens: Caught::HostTest {
            package: "ps2-keys",
            name: "decode::tests::set_1_reads_aa_as_a_shift_release_not_a_self_test_result",
        },
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "        if self.reserved + self.completion.occupied() >= self.completion.capacity().entries() {",
        replace: "        if self.reserved >= self.completion.capacity().entries() {",
        breaks: "ring admission ignores completions already occupying their queue",
        reddens: Caught::HostTest {
            package: "ring-proto",
            name: "tests::the_reservation_invariant_holds_under_interleaving",
        },
    },
    Mutation {
        file: "libs/slab-layout/src/lib.rs",
        find: "        Ok(None) if size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        replace: "        Ok(None) if false && size == 0 => Ok(Placement::Frames {\n            count: 1,\n            align: align.div_ceil(FRAME_SIZE),\n        }),",
        breaks: "an over-aligned zero-size allocation asks for zero frames",
        reddens: Caught::HostTest {
            package: "slab-layout",
            name: "tests::a_zero_sized_request_too_aligned_for_a_class_still_gets_a_frame",
        },
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "            stack_base: guard_base + self.guard_len,",
        replace: "            stack_base: guard_base,",
        breaks: "a stack begins on the page reserved as its guard",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "tests::every_stack_has_its_guard_immediately_below",
        },
    },
    Mutation {
        file: "libs/timebase/src/lib.rs",
        find: "        let carried = advanced - (whole << self.shift);",
        replace: "        let carried = 0;",
        breaks: "each clock update discards its fractional nanosecond carry",
        reddens: Caught::HostTest {
            package: "timebase",
            name: "published_tests::a_thousand_updates_do_not_drift_from_one_conversion",
        },
    },
    Mutation {
        file: "libs/trap-cause/src/lib.rs",
        find: "        self.0 & (1 << 1) != 0",
        replace: "        self.0 & (1 << 2) != 0",
        breaks: "a page fault reports user access as a write",
        reddens: Caught::HostTest {
            package: "trap-cause",
            name: "tests::every_page_fault_bit_is_read_on_its_own",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/xstate.rs",
        find: "            let Ok(narrowed) = u32::try_from(cursor) else {\n                overflowed = true;\n                break;\n            };",
        replace: "            let narrowed = cursor as u32;",
        breaks: "a compacted xstate offset truncates through the u32 boundary",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "xstate::tests::a_layout_that_overflows_a_u32_offset_is_not_consistent",
        },
    },
    Mutation {
        file: "libs/x86-paging/src/lib.rs",
        find: "        Self(self.0 & other.0)",
        replace: "        Self(self.0 | other.0)",
        breaks: "a page walk unions permissions that every level must grant",
        reddens: Caught::HostTest {
            package: "x86-paging",
            name: "tests::intersection_keeps_only_what_both_hold",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "        Some(product) => product.checked_add(denominator / 2),",
        replace: "        Some(product) => product.checked_add(denominator & 0),",
        breaks: "pit calibration floors instead of rounding to nearest",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::rounding_is_to_nearest_not_floor",
        },
    },
    Mutation {
        file: "libs/x86-tables/src/idt.rs",
        find: "        gate.attributes |= 0b11 << 5;",
        replace: "        gate.attributes |= 0b11 << 4;",
        breaks: "a user-callable interrupt gate writes the wrong privilege bits",
        reddens: Caught::HostTest {
            package: "x86-tables",
            name: "idt::tests::a_user_callable_gate_differs_only_in_privilege",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            Self::SelfTest => &[paths::SELFTEST_FEATURE],",
        replace: "            Self::SelfTest => &[],",
        breaks: "the self-test image selector compiles the shipped kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::selectors_build_only_their_named_kernel",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            .open_file(paths::KERNEL_ON_ESP)",
        replace: "            .open_file(paths::PROGRAM_ON_ESP)",
        breaks: "image verification reads another payload instead of the kernel",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::both_kernel_kinds_are_read_back_and_identified",
        },
    },
    Mutation {
        file: "xtask/src/gate.rs",
        find: "    CargoPass {\n        name: \"kernel refuse-pci\",\n        packages: paths::KERNEL_PACKAGES,\n        target: Some(paths::KERNEL_TARGET),\n        features: Some(paths::REFUSE_PCI_FEATURE_SPEC),\n    },\n",
        replace: "",
        breaks: "a kernel feature graph leaves every Cargo-driven gate stage",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_ci_registry_drives_help_and_documentation",
        },
    },
    Mutation {
        file: "xtask/src/gate.rs",
        find: "    Step::ShippedBoot,\n    Step::DegradedBoot,",
        replace: "    Step::ShippedBoot,\n    Step::ShippedBoot,\n    Step::DegradedBoot,",
        breaks: "the local gate silently gains a duplicate shipped-image boot",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_ci_registry_drives_help_and_documentation",
        },
    },
    Mutation {
        file: "libs/elf64/src/lib.rs",
        find: "            offset: read_u64(header, 40)?,",
        replace: "            offset: read_u64(header, 32)?,",
        breaks: "the section table offset is read from the program table field",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::the_section_table_fields_are_where_the_file_header_puts_them",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "            if inventory.is_reexport(file, at) {\n                continue;\n            }",
        replace: "            if false && inventory.is_reexport(file, at) {\n                continue;\n            }",
        breaks: "a public re-export is mistaken for an executable caller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    public_name();\n    api::nested::same_name();",
        replace: "    public_name();\n    api::documented();\n    api::nested::same_name();",
        breaks: "the fixture's documentation-only API secretly gains a code caller",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "xtask/src/api.rs",
        find: "    api::nested::same_name();",
        replace: "    api::same_name();",
        breaks: "the same-name fixture calls the other declaration",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "api::tests::reexports_docs_and_same_names_are_resolved_by_the_compiler",
        },
    },
    Mutation {
        file: "libs/mmio-range/src/lib.rs",
        find: "    pub const fn len(self) -> usize {\n        self.len\n    }\n\n    /// Exclusive end byte.",
        replace: "    pub const fn len(self) -> usize {\n        self.len\n    }\n\n    /// Whether this access range covers no bytes.\n    pub const fn is_empty(self) -> bool {\n        self.len == 0\n    }\n\n    /// Exclusive end byte.",
        breaks: "lint appeasement recreates a public API with no caller",
        reddens: Caught::ApiAudit("method impl::is_empty"),
    },
    Mutation {
        file: "libs/stack-layout/src/lib.rs",
        find: "    left < right_end && right < left_end",
        replace: "    left <= right_end && right < left_end",
        breaks: "a range ending beside a stack is mistaken for an overlap",
        reddens: Caught::HostTest {
            package: "stack-layout",
            name: "tests::stack_overlap_is_half_open_and_fails_closed",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if !is_active() || active_root() != self.root {\n            return Err(MapError::Inactive(self.root));\n        }",
        replace: "        if false && (!is_active() || active_root() != self.root) {\n            return Err(MapError::Inactive(self.root));\n        }",
        breaks: "a recycled inactive root can mint hardware stack authority",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/paging.rs",
        find: "        if pinned != 0\n            && PINNED_STACK_ROOT.load(Ordering::Relaxed) == self.root\n            && ranges_overlap(\n                pinned,\n                PINNED_STACK_LEN.load(Ordering::Relaxed),\n                virtual_base,\n                len,\n            )\n        {",
        replace: "        if pinned != 0\n            && PINNED_STACK_ROOT.load(Ordering::Relaxed) == self.root\n            && false\n            && ranges_overlap(\n                pinned,\n                PINNED_STACK_LEN.load(Ordering::Relaxed),\n                virtual_base,\n                len,\n            )\n        {",
        breaks: "the stack retained by the task-state segment can be unmapped",
        reddens: Caught::Check("the double-fault stack cannot be unmapped"),
    },
    Mutation {
        file: "xtask/src/unsafety.rs",
        find: "        self.lines += span.end().line.saturating_sub(span.start().line) + 1;",
        replace: "        self.lines += 0;",
        breaks: "the unsafe budget counter ignores every operation's span",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::a_block_is_counted_and_a_declaration_is_not",
        },
    },
    Mutation {
        file: "wost/src/mmio.rs",
        find: "    pub const unsafe fn new(base: *mut u8, len: usize) -> Self {",
        replace: "    pub const fn new(base: *mut u8, len: usize) -> Self {",
        breaks: "the raw MMIO constructor loses its caller safety contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "unsafety::tests::every_unsafe_source_has_one_reviewed_fingerprint",
        },
    },
    Mutation {
        file: "README.md",
        find: "See [docs/002-architecture.md](docs/002-architecture.md).",
        replace: "See [docs/002-architecture.md](docs/002-architectur.md).",
        breaks: "a local Markdown link names a path the repository does not hold",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_local_markdown_link_and_anchor_resolves",
        },
    },
    Mutation {
        file: "AGENTS.md",
        find: "[Development](README.md#development)",
        replace: "[Development](README.md#developmnt)",
        breaks: "a local Markdown fragment names no heading in its target",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_local_markdown_link_and_anchor_resolves",
        },
    },
    Mutation {
        file: "AGENTS.md",
        find: "`kernel/src/selftest.rs` and runs under `cargo xtask test-boot`.",
        replace: "`kernel/src/selftest.rs` and runs under `cargo xtask test-bot`.",
        breaks: "an operational document offers an xtask command the parser rejects",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        file: "docs/014-storage.md",
        find: "| Machine checks | 215 |",
        replace: "| Machine checks | 214 |",
        breaks: "the current documentation count disagrees with the machine oracle",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::the_phase_close_says_what_the_source_says",
        },
    },
    Mutation {
        file: "docs/000-vision.md",
        find: "| Boot | UEFI + Limine (vendored); an own bootloader is not planned without a concrete unmet handoff requirement (D-219) |",
        replace: "| Boot | UEFI + Limine (vendored); own bootloader is a future side-quest |",
        breaks: "the current roadmap reschedules the declined own-bootloader project",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::current_facing_foundation_prose_names_the_later_resolutions",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "    if success {\n        let passed = format!(\"test {name} ... ok\");\n        if stdout.contains(&passed) || stderr.contains(&passed) {\n            return Ok(false);\n        }\n        bail!(\"host mutation selected no exact test `{name}`\");\n    }",
        replace: "    if success {\n        return Ok(false);\n    }",
        breaks: "a successful zero-test command is accepted as an exact host witness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_exact_host_witness_must_run",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "            package: \"eevdf\",",
        replace: "            package: \"xtask\",",
        breaks: "a pure host crate can lose its only persistent source mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::every_host_library_has_a_persistent_source_mutation",
        },
    },
    Mutation {
        file: "xtask/src/cargo.rs",
        find: "    let mut args = vec![\"build\".to_string(), \"--locked\".to_string()];",
        replace: "    let mut args = vec![\"build\".to_string()];",
        breaks: "a release build may resolve dependencies outside Cargo.lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "cargo::tests::kernel_builds_are_locked_to_the_recorded_dependencies",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    current == expected",
        replace: "    current.len() == expected.len()",
        breaks: "a stale same-sized namespace becomes an unrecorded boot input",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_same_sized_namespace_with_different_bytes_is_not_reused",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            self.fields.insert(key.clone(), value).is_none(),",
        replace: "            self.fields.insert(key.clone(), value).is_none() || true,",
        breaks: "a later evidence field silently replaces an earlier claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_manifest_rejects_duplicate_keys",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        actual == *expected,",
        replace: "        actual != *expected,",
        breaks: "missing and unknown evidence fields are accepted",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_manifest_schema_is_exact",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    first.bytes == second.bytes && first.sha256 == second.sha256",
        replace: "    first.bytes == second.bytes || first.sha256 == second.sha256",
        breaks: "equal-sized release artifacts are mistaken for equal bytes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::equal_lengths_do_not_make_artifacts_identical",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            record.get(key)? == PASS,",
        replace: "            record.get(key)? == PASS || true,",
        breaks: "a failed qualification result verifies as a pass",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::every_recorded_result_must_pass",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    status.trim().is_empty()",
        replace: "    true",
        breaks: "modified and untracked source can claim a clean identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::a_dirty_source_cannot_be_called_clean",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "        for (key, value) in &self.fields {",
        replace: "        for (key, value) in self.fields.iter().rev() {",
        breaks: "the same evidence fields render in a different byte order",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::rendering_is_sorted_and_round_trips",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "            record.get(&format!(\"{}.sha256\", spec.prefix))? == digest.sha256,",
        replace: "            record.get(&format!(\"{}.sha256\", spec.prefix))? == digest.sha256 || true,",
        breaks: "bundled bytes can differ from the digest the manifest records",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::artifact_bytes_must_match_the_manifest",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "pub(crate) const MACHINE_TYPE: &str = \"pc-q35-9.2\";",
        replace: "pub(crate) const MACHINE_TYPE: &str = \"q35\";",
        breaks: "QEMU's moving q35 alias changes the qualified hardware model",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_machine_type_is_a_versioned_q35_not_an_alias",
        },
    },
    Mutation {
        file: "xtask/src/qualification.rs",
        find: "    Ok(version.to_string())",
        replace: "    Ok(report.trim().to_string())",
        breaks: "vmrun usage text is recorded as part of its version identity",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qualification::tests::vmrun_version_is_separated_from_its_usage_text",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    record.insert(format!(\"{prefix}.version\"), version)?;",
        replace: "    record.insert(format!(\"{prefix}.reported-version\"), version)?;",
        breaks: "an unavailable optional tool changes the evidence schema",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::optional_tools_never_change_the_manifest_schema",
        },
    },
    Mutation {
        file: "xtask/src/evidence.rs",
        find: "    for prefix in QUALIFICATION_TOOLS {",
        replace: "    for prefix in QUALIFICATION_TOOLS.iter().take(0) {",
        breaks: "an unavailable qualification dependency is reported as an unrelated byte mismatch",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "evidence::tests::an_unavailable_qualification_dependency_is_named_exactly",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|anyhow|1.0.104|development|330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470|MIT|selected from MIT OR Apache-2.0",
        replace: "crate|anyhow|1.0.104|development|0000000000000000000000000000000000000000000000000000000000000000|MIT|selected from MIT OR Apache-2.0",
        breaks: "a reviewed dependency checksum can differ from Cargo.lock",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_locked_registry_package_has_one_exact_inventory_row",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|noto-sans-mono-bitmap|0.3.2|runtime|f102c477fd4ec864c149803bf606ecc23bc9329602ec0f99073a728abf454abc|MIT AND OFL-1.1|MIT crate code and OFL-1.1 Noto glyph data",
        replace: "crate|noto-sans-mono-bitmap|0.3.2|runtime|f102c477fd4ec864c149803bf606ecc23bc9329602ec0f99073a728abf454abc|GPL-3.0-only|MIT crate code and OFL-1.1 Noto glyph data",
        breaks: "an unreviewed license conclusion enters the dependency record",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_license_conclusion_is_from_the_reviewed_policy",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "artifact|boot/limine/BOOTX64.EFI|v10.x-binary-2026-07-18|runtime|91ef81c1f95c4ac8bf77c4393a0b2fcd2628550827d7abf89a43194400e17a85|BSD-2-Clause|Limine UEFI binary",
        replace: "artifact|boot/limine/BOOTX64.EFI|v10.x-binary-2026-07-18|runtime|0000000000000000000000000000000000000000000000000000000000000000|BSD-2-Clause|Limine UEFI binary",
        breaks: "a vendored runtime artifact can differ from the inventory digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_vendored_digest_is_in_the_inventory_and_matches_the_bytes",
        },
    },
    Mutation {
        file: "supply-chain/inventory.txt",
        find: "crate|bitflags|2.13.1|runtime|b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da|MIT|selected from MIT OR Apache-2.0",
        replace: "crate|bitflags|2.13.1|development|b588b76d00fde79687d7646a9b5bdf3cc0f655e0bbd080335a95d7e96f3587da|MIT|selected from MIT OR Apache-2.0",
        breaks: "a crate linked into the kernel is labelled development-only",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::runtime_scope_is_the_kernel_dependency_closure",
        },
    },
    Mutation {
        file: "supply-chain/foundation.spdx",
        find: "DocumentName: the-wizards-os-foundation",
        replace: "DocumentName: unrelated-foundation",
        breaks: "the committed SBOM differs from its deterministic source facts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::the_committed_spdx_is_the_deterministic_inventory_rendering",
        },
    },
    Mutation {
        file: "supply-chain/THIRD-PARTY-NOTICES.md",
        find: "SIL Open Font License 1.1 (`OFL-1.1`), copyright 2022 The Noto Project",
        replace: "SIL Open Font License 1.1 (`OFL 1.1`), copyright 2022 The Noto Project",
        breaks: "runtime notices omit the exact font license identifier",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::every_runtime_notice_and_required_license_is_present",
        },
    },
    Mutation {
        file: "xtask/src/supply_chain.rs",
        find: "        matches.len() == 1,",
        replace: "        !matches.is_empty(),",
        breaks: "an ambiguous locked dependency silently selects one version",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "supply_chain::tests::a_dependency_with_two_locked_versions_must_name_which_one",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        word.store(value.to_le(), Ordering::Release);",
        replace: "        word.store(value.to_le(), Ordering::Relaxed);",
        breaks: "a published queue index can become visible before its payload",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::release_publication_excludes_an_old_payload",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "        Some(u32::from_le(word.load(Ordering::Acquire)))",
        replace: "        Some(u32::from_le(word.load(Ordering::Relaxed)))",
        breaks: "a shared payload read can move before its published index",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::acquire_consumption_excludes_an_early_payload_read",
        },
    },
    Mutation {
        file: "wost/src/mm/mod.rs",
        find: "            unsafe { destination.write_volatile(byte) };",
        replace: "            unsafe { destination.write(byte) };",
        breaks: "the compiler can merge or elide shared payload writes",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::every_shared_payload_copy_is_volatile",
        },
    },
    Mutation {
        file: "xtask/src/image/program.rs",
        find: "    const COMPLETION_TAIL_AT: u8 = 28;",
        replace: "    const COMPLETION_TAIL_AT: u8 = 24;",
        breaks: "the real ring client acquires its own head instead of the completion tail",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::program::tests::the_ring_client_orders_shared_memory",
        },
    },
    Mutation {
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
        file: "kernel/src/handle.rs",
        find: "        if entry.owner != self.id {\n            return Err(HandleError::Stale);\n        }",
        replace: "",
        breaks: "a resolved handle cannot cross its ownership table",
        reddens: Caught::Check("a handle reveals no foreign occupancy"),
    },
    Mutation {
        file: "kernel/src/handle.rs",
        find: "                Some(entry) if entry.owner != self.id => return Err(HandleError::Stale),\n",
        replace: "",
        breaks: "closing a foreign handle cannot remove another table's entry",
        reddens: Caught::Check("a handle reveals no foreign occupancy"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/interrupt.rs",
        find: "pub struct InterruptState {
    was_enabled: bool,
    stays_here: PhantomData<*mut ()>,
}",
        replace: "pub struct InterruptState {
    was_enabled: bool,
    stays_here: PhantomData<()>,
}",
        breaks: "a spin-lock guard cannot restore another CPU's interrupt flag",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        file: "wost/src/sync.rs",
        find: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<*const ()>,
}",
        replace: "pub struct ThreadLockGuard<'a, T> {
    lock: &'a ThreadLock<T>,
    stays_here: PhantomData<()>,
}",
        breaks: "a thread-lock guard cannot be released on another CPU",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::guard_types_do_not_cross_cpus",
        },
    },
    Mutation {
        file: "libs/memory-map/src/lib.rs",
        find: "            } else {
                overflow = true;
            }
            reported = reported.saturating_add(1);",
        replace: "            } else {
                continue;
            }
            reported = reported.saturating_add(1);",
        breaks: "an oversized boot map cannot become an accepted prefix",
        reddens: Caught::HostTest {
            package: "memory-map",
            name: "tests::the_region_limit_is_all_or_nothing",
        },
    },
    Mutation {
        file: "libs/x86-cpuid/src/lib.rs",
        find: "                let product = (crystal_hz as u64).checked_mul(numerator as u64);",
        replace: "                let product = (crystal_hz as u64).checked_mul(denominator as u64);",
        breaks: "the CPUID TSC ratio cannot exchange its numerator and denominator",
        reddens: Caught::HostTest {
            package: "x86-cpuid",
            name: "tests::frequency_leaves_keep_exact_and_nominal_evidence_distinct",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if observed_hz.abs_diff(expected_tsc_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        replace: "    if observed_hz.abs_diff(observed_hz) <= expected_tsc_hz / AGREEMENT_DIVISOR {",
        breaks: "a stopped TSC cannot agree with an advancing PIT",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        file: "libs/x86-pit/src/calibrate.rs",
        find: "    if apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        replace: "    if false && apic_remaining != 0 || expected_pit_ticks == 0 || expected_tsc_hz == 0 {",
        breaks: "an older pending APIC interrupt cannot judge a newly armed timer",
        reddens: Caught::HostTest {
            package: "x86-pit",
            name: "calibrate::tests::the_watchdog_distinguishes_disagreement_from_delay",
        },
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "            if failures >= WATCHDOG_FAILURES {",
        replace: "            if failures > WATCHDOG_FAILURES {",
        breaks: "the required watchdog failures revoke the live timing source",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    DESIRED_MODE.store(mode, Ordering::Release);",
        replace: "    DESIRED_MODE.store(MODE_COUNTER, Ordering::Release);",
        breaks: "clock revocation survives the time page's temporary mapping absence",
        reddens: Caught::Check("revoking the clock sends readers to ring Uptime"),
    },
    Mutation {
        file: "kernel/src/time.rs",
        find: "        Some(now.saturating_add(elapsed))",
        replace: "        Some(now.saturating_add(elapsed.saturating_sub(elapsed)))",
        breaks: "the retained PIT advances scheduler time after public-clock revocation",
        reddens: Caught::Check(
            "a clock disagreement revokes time without stranding timers",
        ),
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v1\";",
        replace: "pub const FORMAT: &str = \"wizard-wizzy-handoff-v2\";",
        breaks: "the pinned handoff example uses the one supported envelope version",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::a_canonical_record_is_accepted",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    if lines.next() != Some(FORMAT) {",
        replace: "    if lines.next().is_none() {",
        breaks: "an unknown handoff envelope version cannot be parsed as version one",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::an_unknown_envelope_version_is_refused",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    extra.is_none()",
        replace: "    extra.is_none() || true",
        breaks: "an unknown handoff field cannot be silently ignored",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::unknown_or_noncanonical_fields_are_refused",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    token(record.artifact_abi)",
        replace: "    true || token(record.artifact_abi)",
        breaks: "handoff values cannot use ambiguous encodings or ambient provenance",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::every_value_encoding_is_canonical",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.target_id == TARGET_ID",
        replace: "    true || record.target_id == TARGET_ID",
        breaks: "every handoff target and ABI version must match exactly",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::every_version_must_match_exactly",
        },
    },
    Mutation {
        file: "libs/wizzy-handoff/src/lib.rs",
        find: "    record.artifact_bytes == artifact.bytes && record.artifact_sha256 == artifact.sha256",
        replace: "    record.artifact_bytes == artifact.bytes || record.artifact_sha256 == artifact.sha256",
        breaks: "artifact length and digest must both match the consumed bytes",
        reddens: Caught::HostTest {
            package: "wizzy-handoff",
            name: "tests::artifact_length_and_digest_identify_the_same_bytes",
        },
    },
    Mutation {
        file: "handoff/conformance-v1.artifact",
        find: "This payload is opaque to the handoff envelope.",
        replace: "This payload is altered for the handoff envelope.",
        breaks: "the committed handoff payload cannot differ from its pinned digest",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "handoff::the_committed_handoff_fixture_is_self_contained",
        },
    },
    Mutation {
        file: "userland/abi/wizard_abi.h",
        find: "#define WIZ_TIME_VERSION 1u",
        replace: "#define WIZ_TIME_VERSION 2u",
        breaks: "platform ABI v1 cannot silently contain a version-two wire region",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "handoff::platform_v1_is_entry_ring_and_time_v1",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    let escaped = path.to_string_lossy().replace(',', \",,\");",
        replace: "    let escaped = path.to_string_lossy().into_owned();",
        breaks: "a comma in a disk path remains QEMU drive syntax",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::drive_paths_escape_commas_without_quoting_spaces",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    drive_file(&format!(\"{options},snapshot=on\"), path)",
        replace: "    drive_file(&format!(\"{options},snapshot=off\"), path)",
        breaks: "two QEMU runs may write the same shared build media",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::shared_media_writes_use_a_private_snapshot",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        let monitor = control_listener(\"monitor\")?;",
        replace: "        let monitor = qmp.try_clone().context(\"cloning the QMP listener\")?;",
        breaks: "QMP and the human monitor contend for one control port",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::simultaneous_control_sets_reserve_four_distinct_ports",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "const MONITOR_CLIENT_SUFFIX: &str = \",server=off\";",
        replace: "const MONITOR_CLIENT_SUFFIX: &str = \",server=on\";",
        breaks: "QEMU tries to bind the monitor port already held by the harness",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_monitor_dials_the_reserved_listener",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "            let sequence = NEXT_RUN.fetch_add(1, Ordering::Relaxed);",
        replace: "            let sequence = NEXT_RUN.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU runs reuse one firmware-store name",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::simultaneous_runs_get_distinct_writable_firmware",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "                if cancelled.is_some_and(|flag| flag.load(Ordering::Acquire)) {",
        replace: "                if cancelled.is_some_and(|_| false) {",
        breaks: "a cancelled QEMU control accept waits until its old deadline",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::a_cancelled_control_wait_returns_without_a_connection",
        },
    },
    Mutation {
        file: "xtask/src/image/mod.rs",
        find: "            let sequence = NEXT_RUN_IMAGE.fetch_add(1, Ordering::Relaxed);",
        replace: "            let sequence = NEXT_RUN_IMAGE.load(Ordering::Relaxed);",
        breaks: "concurrent QEMU builds reserve one boot-image path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::tests::two_qemu_images_are_private_and_removed",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        match file.try_lock() {",
        replace: "        match std::result::Result::<(), TryLockError>::Ok(()) {",
        breaks: "concurrent runners can rewrite the NVMe corpus together",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::storage_construction_is_serialized_across_runners",
        },
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::X87 as u32);",
        breaks: "x87 state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::SSE as u32);",
        breaks: "SSE state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "wost/src/arch/x86_64/xstate.rs",
        find: "const OPERATION_MASK: u32 = u32::MAX;",
        replace: "const OPERATION_MASK: u32 = u32::MAX & !(xstate::AVX as u32);",
        breaks: "AVX state follows the thread",
        reddens: Caught::Check("every enabled state component survives a switch"),
    },
    Mutation {
        file: "libs/ring-proto/src/lib.rs",
        find: "                self.reserved += 1;",
        replace: "                self.reserved += 0;",
        breaks: "an admitted operation has no completion reservation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "publication::tests::admission_reserves_before_the_shared_submission_is_consumed",
        },
    },
    Mutation {
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
        file: "models/RingMemory.tla",
        find: "        \\/ BeginRetire(op)\n",
        replace: "",
        breaks: "retirement is absent from the terminal-ownership state graph",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::every_terminal_cause_is_a_reachable_model_action",
        },
    },
    Mutation {
        file: "xtask/src/model.rs",
        find: "{} is missing; see tools/tla/README.md for where it comes from",
        replace: "{} is missing; see models/README.md for where it comes from",
        breaks: "a missing model checker points to a nonexistent recovery document",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "model::tests::a_missing_model_checker_points_to_its_vendoring_instructions",
        },
    },
    Mutation {
        file: "kernel/src/user.rs",
        find: "    spawn(\"user-baseline\", 8, MEASURE)",
        replace: "    spawn(\"user-baseline\", 8, 0)",
        breaks: "the ring boundary baseline never enables its timestamp witness",
        reddens: Caught::Check("a ring enter round trip is measurable"),
    },
    Mutation {
        file: "kernel/src/nvme.rs",
        find: "        self.read_blocks(0, blocks, into).ok()?;",
        replace: "        self.read_blocks(self.blocks, blocks, into).ok()?;",
        breaks: "the storage baseline never submits inside the namespace",
        reddens: Caught::Check("a full-buffer nvme read is measurable"),
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "        if fields.next().is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        replace: "        if false && fields.next().is_some() {\n            bail!(\"trailing fields in foundation baseline\");\n        }",
        breaks: "a performance record can append unversioned fields",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::baseline_record_is_versioned_ordered_and_unique",
        },
    },
    Mutation {
        file: "xtask/src/baseline.rs",
        find: "        if value >= self.minimum && value <= self.maximum {",
        replace: "        if value >= self.minimum || value <= self.maximum {",
        breaks: "a performance threshold has no lower or upper side",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "baseline::tests::baseline_math_and_bands_are_two_sided",
        },
    },
    Mutation {
        file: "xtask/src/cli.rs",
        find: "        \"baseline\" => Ok(Task::Baseline),",
        replace: "        \"baseline-disabled\" => Ok(Task::Baseline),",
        breaks: "the documented foundation baseline command is not runnable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        file: "xtask/src/image/program.rs",
        find: "    code.extend_from_slice(&[0x41, 0xB8, 1, 0, 0, 0]); // mov r8d, 1",
        replace: "    code.extend_from_slice(&[0x41, 0xB8, 0, 0, 0, 0]); // mov r8d, 0",
        breaks: "the measured ring behaviour never arms its timestamp path",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_c_header_and_the_kernel_agree_about_the_abi",
        },
    },
    Mutation {
        file: "xtask/src/qemu.rs",
        find: "    if port == 0 {",
        replace: "    if false && port == 0 {",
        breaks: "the debugger accepts QEMU's unallocated port-zero placeholder",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "qemu::tests::the_debug_endpoint_is_allocated_and_loopback_only",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "    if identity != GDB_IDENTITY {",
        replace: "    if false && identity != GDB_IDENTITY {",
        breaks: "an unreviewed debugger identity becomes part of the supported setup",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::only_the_reviewed_native_gdb_identity_is_accepted",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "        format!(\"hbreak {ENTRY_SYMBOL}\"),",
        replace: "        format!(\"break {ENTRY_SYMBOL}\"),",
        breaks: "the debugger tries to write a software breakpoint before the kernel is mapped",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::the_debugger_commands_are_entry_first_and_the_proof_is_bounded",
        },
    },
    Mutation {
        file: "xtask/src/debugger.rs",
        find: "    if !frame.contains(\"wizard_kernel::kmain\") || !frame.contains(\"wost/src/boot/mod.rs\") {",
        replace: "    if !frame.contains(\"wizard_kernel::kmain\") && !frame.contains(\"wost/src/boot/mod.rs\") {",
        breaks: "a backtrace may omit either the entry symbol or its source location",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "debugger::tests::the_debugger_proof_requires_the_entry_marker_symbol_and_source",
        },
    },
    Mutation {
        file: "xtask/src/cli.rs",
        find: "        \"debug\" => Ok(Task::Debug(if check {",
        replace: "        \"debug-disabled\" => Ok(Task::Debug(if check {",
        breaks: "the documented native debugger command is not runnable",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "prose::tests::every_documented_xtask_invocation_parses",
        },
    },
    Mutation {
        file: "xtask/src/mutate.rs",
        find: "const MACHINE_OBSERVATIONS: usize = 2;\n\nfn qualify_machine_mutation",
        replace: "const MACHINE_OBSERVATIONS: usize = 1;\n\nfn qualify_machine_mutation",
        breaks: "one incidental machine failure can invalidate an otherwise exact mutation",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "mutate::tests::an_inconclusive_machine_observation_gets_one_bounded_retry",
        },
    },
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Baselines {
    host: bool,
    api: bool,
    selftest: bool,
    degraded: bool,
}

fn baselines_for(mutations: &[Mutation]) -> Baselines {
    let mut baselines = Baselines::default();
    for mutation in mutations {
        match mutation.reddens {
            Caught::HostTest { .. } => baselines.host = true,
            Caught::ApiAudit(_) => baselines.api = true,
            Caught::Check(_) | Caught::Panic { .. } => baselines.selftest = true,
            Caught::BootMarker(_) => baselines.degraded = true,
        }
    }
    baselines
}

fn require_green(label: &str, result: Result<()>) -> Result<()> {
    result.with_context(|| format!("pristine {label} mutation baseline is not green"))
}

fn prove_green_baselines(paths: &Paths, mutations: &[Mutation]) -> Result<()> {
    let baselines = baselines_for(mutations);
    if baselines.host {
        println!("[xtask] proving the pristine host mutation baseline");
        require_green("host", tasks::test(paths))?;
    }
    if baselines.api {
        println!("[xtask] proving the pristine API-audit mutation baseline");
        require_green("API audit", crate::api::check(paths))?;
    }
    if baselines.selftest {
        println!("[xtask] proving the pristine self-test mutation baseline");
        require_green(
            "self-test",
            tasks::test_boot(paths, crate::cargo::Profile::Release),
        )?;
    }
    if baselines.degraded {
        println!("[xtask] proving the pristine degraded mutation baseline");
        require_green(
            "degraded",
            tasks::degraded_check(paths, crate::cargo::Profile::Release),
        )?;
    }
    Ok(())
}

/// Apply each mutation, run its exact machine or host witness, and require red.
///
/// # Errors
///
/// If a pattern no longer matches, if the tree cannot be restored, or if any
/// mutation **survives** — which is the interesting failure and the reason
/// this exists.
pub fn run(paths: &Paths) -> Result<()> {
    let registry = registry_fingerprint();
    let mut session = Session::open(paths, &registry, MUTATIONS.len())?;
    if let Some(active) = session.active() {
        let mutation = MUTATIONS
            .get(active)
            .with_context(|| format!("journal names absent mutation {active}"))?;
        println!(
            "[xtask] restoring interrupted mutation {}/{}: {}",
            active + 1,
            MUTATIONS.len(),
            mutation.breaks
        );
        session.recover(active, mutation.file)?;
    }

    let first = session.completed().len();
    if first != 0 {
        println!(
            "[xtask] resuming after {first}/{} durable mutation outcomes",
            MUTATIONS.len()
        );
    }
    prove_green_baselines(session.paths(), &MUTATIONS[first..])?;
    for (index, mutation) in MUTATIONS.iter().enumerate().skip(first) {
        ensure_target_is_rebuilt(mutation)?;
        let original = session.original(mutation.file)?;
        let mutated = apply(&original, mutation)?;

        println!(
            "[xtask] mutate {}/{}: {}",
            index + 1,
            MUTATIONS.len(),
            mutation.breaks
        );
        session.begin(index)?;
        session.write(mutation.file, &mutated)?;
        let outcome = match mutation_reports(session.paths(), mutation.reddens) {
            Ok(true) => {
                println!("[xtask]   caught");
                Outcome::Caught
            }
            Ok(false) => {
                println!("[xtask]   SURVIVED");
                Outcome::Survived
            }
            Err(error) => {
                // A harness failure or a different red verdict is not a
                // survivor: this mutation's intended question was not
                // answered, so crediting it would be the lie this task exists
                // to find.
                //
                // **Kept apart from the survivors**, because the two want
                // different work. A survivor is a property nothing checks; a
                // mutation with no valid causal verdict is a mutation or
                // harness defect, and reading it as the other sends the reader
                // looking for a missing check that is not missing (D-175,
                // D-209).
                println!("[xtask]   inconclusive: {error}");
                Outcome::Inconclusive(format!("{error:#}"))
            }
        };
        session.finish(index, mutation.file, outcome)?;
    }

    let outcomes = session.complete(MUTATIONS.len())?;
    let mut survivors = Vec::new();
    let mut inconclusive = Vec::new();
    for (mutation, outcome) in MUTATIONS.iter().zip(&outcomes) {
        match outcome {
            Outcome::Caught => {}
            Outcome::Survived => survivors.push(mutation),
            Outcome::Inconclusive(error) => inconclusive.push((mutation, error)),
        }
    }
    if survivors.is_empty() && inconclusive.is_empty() {
        println!("[xtask] every mutation was caught by the check it names");
        return Ok(());
    }
    for survivor in &survivors {
        println!("[xtask] survived: {} ({})", survivor.breaks, survivor.file);
    }
    for (mutation, error) in &inconclusive {
        println!(
            "[xtask] inconclusive: {} ({}) — {error}",
            mutation.breaks, mutation.file
        );
    }

    match (survivors.len(), inconclusive.len()) {
        (0, count) => bail!(
            "{count} of {} mutations were inconclusive; each needs a valid build and its \
             exact expected verdict",
            MUTATIONS.len()
        ),
        (count, 0) => bail!(
            "{count} of {} mutations survived; each is a property nothing checks",
            MUTATIONS.len()
        ),
        (survived, inconclusive) => bail!(
            "{survived} of {} mutations survived and {inconclusive} were inconclusive",
            MUTATIONS.len()
        ),
    }
}

fn registry_fingerprint() -> String {
    fn field(material: &mut Vec<u8>, value: &str) {
        material.extend_from_slice(&(value.len() as u64).to_le_bytes());
        material.extend_from_slice(value.as_bytes());
    }

    let mut material = Vec::new();
    for mutation in MUTATIONS {
        field(&mut material, mutation.file);
        field(&mut material, mutation.find);
        field(&mut material, mutation.replace);
        field(&mut material, mutation.breaks);
        match mutation.reddens {
            Caught::Check(name) => {
                field(&mut material, "check");
                field(&mut material, name);
            }
            Caught::HostTest { package, name } => {
                field(&mut material, "host");
                field(&mut material, package);
                field(&mut material, name);
            }
            Caught::ApiAudit(item) => {
                field(&mut material, "api-audit");
                field(&mut material, item);
            }
            Caught::BootMarker(marker) => {
                field(&mut material, "boot-marker");
                field(&mut material, marker);
            }
            Caught::Panic { marker, why } => {
                field(&mut material, "panic");
                field(&mut material, marker);
                field(&mut material, why);
            }
        }
    }
    sha256::hex(&sha256::digest(&material))
}

/// Refuse source the already-running mutation driver would consume itself.
fn ensure_target_is_rebuilt(mutation: &Mutation) -> Result<()> {
    let rebuilt_witness = matches!(
        mutation.reddens,
        Caught::HostTest {
            package: "xtask",
            ..
        } | Caught::ApiAudit(_)
    );
    if mutation.file.starts_with("xtask/") && !rebuilt_witness {
        bail!(
            "{}: `{}` belongs to the running mutation driver and would not be rebuilt by its witness",
            mutation.file,
            mutation.breaks
        );
    }
    Ok(())
}

/// The mutated text, or an error naming why the pattern is no longer usable.
fn apply(original: &str, mutation: &Mutation) -> Result<String> {
    match original.matches(mutation.find).count() {
        1 => Ok(original.replacen(mutation.find, mutation.replace, 1)),
        0 => bail!(
            "{}: the pattern for `{}` no longer occurs; the mutation tests nothing",
            mutation.file,
            mutation.breaks
        ),
        many => bail!(
            "{}: the pattern for `{}` occurs {many} times; which one it breaks is not decided",
            mutation.file,
            mutation.breaks
        ),
    }
}

/// Run the named host or machine witness and report whether **this** mutation
/// made its exact check go red.
///
/// Not merely whether the boot did, which is what this asked until D-134 —
/// and the difference is not theoretical. Mutation 4 reddens mutation 3's
/// check as well as its own, so deleting 4's target would have left it
/// printing "caught" for ever, off the back of another entry's check.
/// Mutations 2 and 10 overlap on ten checks between them. "Some check failed"
/// is a description of the boot; the claim the table makes is about a
/// property.
///
/// A refused *build* is an error rather than a red, because a mutation that
/// does not compile has not been tested by anything.
fn mutation_reports(paths: &Paths, wanted: Caught) -> Result<bool> {
    if let Caught::HostTest { package, name } = wanted {
        return host_test_reports(paths, package, name);
    }
    if let Caught::BootMarker(marker) = wanted {
        return match tasks::degraded_check(paths, crate::cargo::Profile::Release) {
            Ok(()) => Ok(false),
            Err(error) => {
                let expected = format!("without reporting {marker:?}");
                if format!("{error:#}").contains(&expected) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    if let Caught::ApiAudit(item) = wanted {
        return match crate::api::check(paths) {
            Ok(()) => Ok(false),
            Err(error) => {
                let report = format!("{error:#}");
                if report.contains("public API item(s) have no caller") && report.contains(item) {
                    Ok(true)
                } else {
                    Err(error)
                }
            }
        };
    }
    qualify_machine_mutation(wanted, || {
        tasks::observe_test_boot(paths, crate::cargo::Profile::Release)
    })
}

const MACHINE_OBSERVATIONS: usize = 2;

fn qualify_machine_mutation(
    wanted: Caught,
    mut observe: impl FnMut() -> Result<qemu::SelftestOutcome>,
) -> Result<bool> {
    let mut first_error = None;
    for attempt in 1..=MACHINE_OBSERVATIONS {
        match observe().and_then(|observed| assess_selftest(wanted, observed)) {
            Ok(caught) => return Ok(caught),
            Err(error) if attempt < MACHINE_OBSERVATIONS => {
                let report = format!("{error:#}");
                eprintln!("[xtask]   machine observation inconclusive; retrying once: {report}");
                first_error = Some(report);
            }
            Err(error) => {
                if let Some(first) = first_error {
                    bail!(
                        "machine mutation remained inconclusive after {MACHINE_OBSERVATIONS} observations; first: {first}; retry: {error:#}"
                    );
                }
                return Err(error);
            }
        }
    }
    unreachable!("the bounded observation loop returns on every outcome")
}

fn assess_selftest(wanted: Caught, observed: qemu::SelftestOutcome) -> Result<bool> {
    match (wanted, observed) {
        (_, qemu::SelftestOutcome::Passed) => Ok(false),
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed))
            if failed.iter().any(|actual| actual == name) =>
        {
            Ok(true)
        }
        (Caught::Check(name), qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected check {name:?}, but different checks failed: {}",
            failed.join("; ")
        ),
        (Caught::Check(name), qemu::SelftestOutcome::Panicked(line)) => {
            bail!("mutation expected check {name:?}, but the kernel panicked: {line}")
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked(line))
            if line.contains(marker) =>
        {
            Ok(true)
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Panicked(line)) => {
            bail!("mutation expected panic marker {marker:?}, but observed: {line}")
        }
        (Caught::Panic { marker, .. }, qemu::SelftestOutcome::Failed(failed)) => bail!(
            "mutation expected panic marker {marker:?}, but checks failed: {}",
            failed.join("; ")
        ),
        (Caught::HostTest { .. } | Caught::ApiAudit(_) | Caught::BootMarker(_), _) => {
            unreachable!("other witnesses return before self-test assessment")
        }
    }
}

fn host_test_reports(paths: &Paths, package: &str, name: &str) -> Result<bool> {
    let output = Command::new(env!("CARGO"))
        .current_dir(paths.root())
        .env("CARGO_TARGET_DIR", paths.target_dir())
        .args(["test", "--package", package, name, "--", "--exact"])
        .output()
        .with_context(|| format!("could not spawn host mutation test {package}::{name}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assess_host_test(
        name,
        output.status.success(),
        &stdout,
        &stderr,
        &output.status.to_string(),
    )
}

fn assess_host_test(
    name: &str,
    success: bool,
    stdout: &str,
    stderr: &str,
    status: &str,
) -> Result<bool> {
    let verdict = format!("test {name} ... FAILED");
    if stdout.contains(&verdict) || stderr.contains(&verdict) {
        return Ok(true);
    }
    if success {
        let passed = format!("test {name} ... ok");
        if stdout.contains(&passed) || stderr.contains(&passed) {
            return Ok(false);
        }
        bail!("host mutation selected no exact test `{name}`");
    }
    bail!("host mutation did not reach `{name}`; cargo exited {status}\n{stdout}{stderr}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rust_sources_contain(directory: &std::path::Path, needle: &str) -> bool {
        std::fs::read_dir(directory)
            .unwrap_or_else(|_| panic!("reading {}", directory.display()))
            .flatten()
            .any(|entry| {
                let path = entry.path();
                if path.is_dir() {
                    rust_sources_contain(&path, needle)
                } else if path.extension().is_some_and(|extension| extension == "rs") {
                    std::fs::read_to_string(&path)
                        .unwrap_or_else(|_| panic!("reading {}", path.display()))
                        .contains(needle)
                } else {
                    false
                }
            })
    }

    /// Every mutation still names something that exists, exactly once.
    ///
    /// The cheap tier, and the one that matters more: a pattern that has
    /// stopped matching is a mutation that silently stopped testing anything —
    /// which is the failure mode this whole file exists to catch, one level up.
    /// Milliseconds, in `cargo test`, so it is in the gate where the boot
    /// itself cannot be.
    #[test]
    fn every_mutation_still_applies() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");

        for mutation in MUTATIONS {
            let path = root.join(mutation.file);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("reading {}", path.display()));
            let found = text.matches(mutation.find).count();
            assert_eq!(
                found, 1,
                "{}: the pattern for `{}` occurs {found} times, not once",
                mutation.file, mutation.breaks
            );
            // And it changes something: a mutation whose replacement is its
            // own pattern is a boot that proves nothing.
            assert_ne!(mutation.find, mutation.replace, "{}", mutation.breaks);
        }
    }

    /// Mutating the driver cannot change the binary already building images.
    #[test]
    fn a_mutation_cannot_target_the_running_driver() {
        for mutation in MUTATIONS {
            ensure_target_is_rebuilt(mutation).unwrap_or_else(|error| panic!("{error}"));
        }

        let mut impossible = MUTATIONS[0];
        impossible.file = "xtask/src/image/program.rs";
        let error =
            ensure_target_is_rebuilt(&impossible).expect_err("the driver is already running");
        assert!(error.to_string().contains("would not be rebuilt"));
    }

    /// Every named machine check or host test function exists.
    ///
    /// The cheap half of D-134, and the one that runs at every commit. A
    /// mutation whose target check was renamed or deleted still boots, still
    /// reddens *something* on the way past, and still prints "caught" — so the
    /// expensive tier cannot see this, and the sweep costs a boot apiece.
    /// Host module qualification is proved when the exact test command prints
    /// that test's own verdict; this cheap source check catches a renamed or
    /// deleted function without recursively invoking Cargo.
    #[test]
    fn every_named_check_exists() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest = std::fs::read_to_string(root.join("kernel/src/selftest.rs"))
            .expect("reading selftest.rs");

        for mutation in MUTATIONS {
            match mutation.reddens {
                Caught::Check(name) => {
                    // The literal as `report.check` is called with it.
                    assert!(
                        selftest.contains(&format!("\"{name}\"")),
                        "`{}` names the check `{name}`, which `selftest.rs` does not have",
                        mutation.breaks
                    );
                }
                Caught::HostTest { package, name } => {
                    assert!(
                        crate::paths::HOST_PACKAGES.contains(&package),
                        "`{}` names non-host package `{package}`",
                        mutation.breaks
                    );
                    let function = name.rsplit("::").next().expect("a test name");
                    let package = if package == "xtask" {
                        root.join(package)
                    } else {
                        root.join("libs").join(package)
                    };
                    assert!(
                        rust_sources_contain(&package, &format!("fn {function}(")),
                        "`{}` names host test `{name}`, which package `{}` does not have",
                        mutation.breaks,
                        package.display()
                    );
                }
                Caught::ApiAudit(item) => {
                    assert!(!item.is_empty(), "API-audit mutations name an exact item");
                }
                Caught::BootMarker(marker) => {
                    let qemu = std::fs::read_to_string(root.join("xtask/src/qemu.rs"))
                        .expect("reading qemu.rs");
                    assert!(
                        qemu.contains(marker),
                        "`{}` names boot marker `{marker}`, which qemu.rs does not have",
                        mutation.breaks
                    );
                }
                Caught::Panic { .. } => {}
            }
        }
    }

    #[test]
    fn every_host_library_has_a_persistent_source_mutation() {
        for &package in crate::paths::HOST_PACKAGES {
            if package == "xtask" {
                continue;
            }
            let source = format!("libs/{package}/src/");
            assert!(
                MUTATIONS.iter().any(|mutation| {
                    mutation.file.starts_with(&source)
                        && matches!(
                            mutation.reddens,
                            Caught::HostTest {
                                package: actual,
                                ..
                            } if actual == package
                        )
                }),
                "{package} has no persistent production-source mutation"
            );
        }
    }

    #[test]
    fn an_exact_host_witness_must_run() {
        let name = "tests::the_exact_test";
        assert!(assess_host_test(name, true, "running 0 tests", "", "0").is_err());
        assert!(
            assess_host_test(name, true, "test tests::the_exact_test ... ok", "", "0")
                .is_ok_and(|caught| !caught)
        );
        assert!(
            assess_host_test(
                name,
                false,
                "test tests::the_exact_test ... FAILED",
                "",
                "1"
            )
            .is_ok_and(|caught| caught)
        );
        assert!(assess_host_test(name, false, "", "compiler error", "1").is_err());
    }

    /// Every mutation says what catching it looks like.
    ///
    /// A generic panic would be the way to credit an unrelated crash, and a
    /// panic with no reason would be the way to dodge naming a check.
    #[test]
    fn a_panicking_mutation_names_an_exact_marker_and_why_it_is_terminal() {
        for mutation in MUTATIONS {
            if let Caught::Panic { marker, why } = mutation.reddens {
                assert!(
                    marker.len() > 12,
                    "`{}` has no exact panic marker",
                    mutation.breaks
                );
                assert!(
                    why.len() > 40,
                    "`{}` claims nothing names it and does not say why",
                    mutation.breaks
                );
            }
        }
    }

    #[test]
    fn a_red_baseline_stops_before_any_mutation() {
        let error = require_green("self-test", Err(anyhow::anyhow!("already red")))
            .expect_err("a red baseline cannot be accepted");
        assert!(format!("{error:#}").contains("pristine self-test mutation baseline"));
    }

    #[test]
    fn a_wrong_machine_failure_is_inconclusive_not_caught() {
        let wanted = Caught::Check("the intended check");
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Failed(vec!["another check".to_string()])
            )
            .is_err()
        );
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked("unrelated panic".to_string())
            )
            .is_err()
        );
        assert!(assess_selftest(wanted, qemu::SelftestOutcome::Passed).is_ok_and(|caught| !caught));
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
            )
            .is_ok_and(|caught| caught)
        );
    }

    #[test]
    fn a_panic_requires_its_exact_marker() {
        let wanted = Caught::Panic {
            marker: "the exact cause",
            why: "this fixture explains why no ordinary check can report the terminal defect",
        };
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked("a different panic".to_string())
            )
            .is_err()
        );
        assert!(
            assess_selftest(
                wanted,
                qemu::SelftestOutcome::Panicked("panic: the exact cause".to_string())
            )
            .is_ok_and(|caught| caught)
        );
    }

    #[test]
    fn an_inconclusive_machine_observation_gets_one_bounded_retry() {
        let wanted = Caught::Check("the intended check");
        let mut observations = 0;
        let caught = qualify_machine_mutation(wanted, || {
            observations += 1;
            Ok(if observations == 1 {
                qemu::SelftestOutcome::Panicked("incidental panic".to_string())
            } else {
                qemu::SelftestOutcome::Failed(vec!["the intended check".to_string()])
            })
        })
        .expect("the exact retry is causal");
        assert!(caught);
        assert_eq!(observations, 2);

        observations = 0;
        let error = qualify_machine_mutation(wanted, || {
            observations += 1;
            Ok(qemu::SelftestOutcome::Panicked(
                "incidental panic".to_string(),
            ))
        })
        .expect_err("repeated inconclusive observations still fail qualification");
        assert_eq!(observations, 2);
        assert!(error.to_string().contains("after 2 observations"));
    }

    /// No two mutations break the same property.
    ///
    /// A duplicate is a witness run spent twice on one question; machine
    /// entries make that cost a complete boot.
    #[test]
    fn no_two_mutations_name_the_same_property() {
        for (index, mutation) in MUTATIONS.iter().enumerate() {
            for earlier in &MUTATIONS[..index] {
                assert_ne!(earlier.breaks, mutation.breaks, "{}", mutation.breaks);
            }
        }
    }

    #[test]
    fn a_pattern_that_matches_once_is_applied_and_others_are_refused() {
        let mutation = Mutation {
            file: "x",
            find: "keep",
            replace: "drop",
            breaks: "a thing",
            reddens: Caught::Check("a thing goes red"),
        };
        assert_eq!(apply("a keep b", &mutation).expect("once"), "a drop b");
        assert!(apply("nothing here", &mutation).is_err());
        assert!(apply("keep and keep", &mutation).is_err());
    }
}
