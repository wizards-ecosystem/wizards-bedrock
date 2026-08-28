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
//! **Restoring is not optional and not best-effort.** Every mutation is
//! applied to a file that is restored from an in-memory copy of the original
//! before the next one, and on every error path. A crash between the two would
//! leave the tree modified, which is why the mutation is written back before
//! anything is allowed to fail.

use anyhow::{Context, Result, bail};
use std::process::Command;

use crate::paths::Paths;
use crate::{qemu, tasks};

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
    /// A shipped degraded boot must emit one exact serial marker.
    BootMarker(&'static str),
    /// Nothing names it: the defect kills the machine before the verdict.
    ///
    /// Carries *why*, because a bare variant would be a hole anything could be
    /// driven through. A reason someone can check by reading is what keeps
    /// this from becoming the way to dodge naming a check (D-134).
    Panic(&'static str),
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
        reddens: Caught::Check("a vector register survives a switch"),
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
        reddens: Caught::Panic(
            "without the fixup the fault is terminal, so the kernel dies before it \
             reaches a verdict at all",
        ),
    },
    // Its failure mode is a panic: the assert in `ThreadLock` fires, which is
    // the defect being a defect. A program reaching this halted the machine
    // (D-122).
    Mutation {
        file: "kernel/src/user.rs",
        find: "            REFUSED_CALLS.fetch_add(1, Ordering::Relaxed);",
        replace: "            crate::warn!(\"refused\");",
        breaks: "an unserved call does not log from trap context",
        reddens: Caught::Panic(
            "logging from trap context trips `ThreadLock`'s assert, which is the \
             defect being a defect",
        ),
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
    // D-126's detector, exercised by putting a violation back. `bootstrap_of`
    // is the smallest of the four that were fixed: holding the table's guard
    // across `kind_of` takes the handle arena's under it.
    Mutation {
        file: "kernel/src/user.rs",
        find: "    let (table, handle) = {
        let guard = BOOTSTRAPS.lock_leaf();
        let bootstrap = guard.get(slot)?.as_ref()?;
        (alloc::sync::Arc::clone(&bootstrap.table), bootstrap.handle)
    };",
        replace: "    let guard = BOOTSTRAPS.lock_leaf();
    let bootstrap = guard.get(slot)?.as_ref()?;
    let table = alloc::sync::Arc::clone(&bootstrap.table);
    let handle = bootstrap.handle;",
        breaks: "no lock is taken while a leaf guard is held",
        reddens: Caught::Check("no lock is taken while a leaf guard is held"),
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
        find: "        commit_quota(quota)?;",
        replace: "        commit_quota(quota.saturating_sub(1))?;",
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
        find: "self.begin_settlement(|entry| entry.deadline_ns <= now_ns)",
        replace: "self.begin_settlement(|_| true)",
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
            self.pending.lock_leaf()[index] = None;
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
        find: "            room[index] = Some(span);",
        replace: "            for port in span.clone() {
                self.reserved[port / PORTS_PER_WORD] |= bit(port);
            }
            room[index] = Some(span);",
        breaks: "a refused multi-span reservation records no earlier span",
        reddens: Caught::HostTest {
            package: "port-space",
            name: "tests::a_refused_multi_span_reservation_takes_nothing",
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
        find: "            Some(bits) if bits as u16 & wanted == wanted => return Ok(()),",
        replace: "            Some(_) => return Ok(()),",
        breaks: "a bus master command write is read back before success",
        reddens: Caught::HostTest {
            package: "pci-config",
            name: "tests::a_refused_bus_master_enable_restores_the_exact_command",
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
        find: "        let read = match crate::vfs::read(file, offset, &mut scratch[..wanted]) {",
        replace: "        let read = match crate::vfs::read(file, 0, &mut scratch[..wanted]) {",
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
    // D-133's exemption, put back. The inner test is left in place so the
    // counter still cannot move, which is the point: the woken thread runs
    // either way, because the keyboard arm reschedules a moment later, so only
    // a check that reads the arming can tell the two boots apart.
    Mutation {
        file: "kernel/src/thread.rs",
        find: "    if woken > 0 {
        if interrupt::in_trap() {",
        replace: "    if woken > 0 && !interrupt::in_trap() {
        if interrupt::in_trap() {",
        breaks: "a wake from a handler arms the timer for what it woke",
        reddens: Caught::Check("a wake from a handler arms the timer for what it woke"),
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
        file: "wost/src/boot/mod.rs",
        find: "            .filter(|region| region.kind.is_direct_mapped())",
        replace: "            .filter(|region| !region.kind.is_direct_mapped())",
        breaks: "a probed run is memory, not merely described",
        reddens: Caught::Check("the map offers a run to probe"),
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
        reddens: Caught::Panic(
            "the second fabrication writes through the zeroed stack top before a verdict",
        ),
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
        find: "            pending[index] = Some(Pending {\n                settling: false,\n                ..entry\n            });\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
        replace: "            pending[index] = None;\n            self.settlement_refusals.fetch_add(1, Ordering::AcqRel);",
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
];

/// Apply each mutation, run its exact machine or host witness, and require red.
///
/// # Errors
///
/// If a pattern no longer matches, if the tree cannot be restored, or if any
/// mutation **survives** — which is the interesting failure and the reason
/// this exists.
pub fn run(paths: &Paths) -> Result<()> {
    let mut survivors = Vec::new();
    let mut inconclusive = Vec::new();

    for (index, mutation) in MUTATIONS.iter().enumerate() {
        ensure_target_is_rebuilt(mutation)?;
        let path = paths.root().join(mutation.file);
        let original = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mutated = apply(&original, mutation)?;

        println!(
            "[xtask] mutate {}/{}: {}",
            index + 1,
            MUTATIONS.len(),
            mutation.breaks
        );
        std::fs::write(&path, &mutated).with_context(|| format!("writing {}", path.display()))?;

        let noticed = mutation_reports(paths, mutation.reddens);

        // Before anything is allowed to fail, including the check below.
        std::fs::write(&path, &original)
            .with_context(|| format!("restoring {}", path.display()))?;

        match noticed {
            Ok(true) => println!("[xtask]   caught"),
            Ok(false) => {
                println!("[xtask]   SURVIVED");
                survivors.push(mutation);
            }
            Err(error) => {
                // A build failure is not a survivor: the mutation did not run,
                // so it proved nothing either way, and reporting it as caught
                // would be the same lie this task exists to find.
                //
                // **Kept apart from the survivors**, because the two want
                // different work. A survivor is a property nothing checks; a
                // mutation that will not build is a *defect in the mutation*,
                // and reading one as the other sends the reader looking for a
                // missing check that is not missing. The summary used to call
                // both "survived" and did exactly that (D-175).
                println!("[xtask]   inconclusive: {error}");
                inconclusive.push((mutation, error));
            }
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
            "[xtask] did not build: {} ({}) — {error}",
            mutation.breaks, mutation.file
        );
    }

    match (survivors.len(), inconclusive.len()) {
        (0, count) => bail!(
            "{count} of {} mutations would not build; each is a defect in the mutation, \
             not a missing check",
            MUTATIONS.len()
        ),
        (count, 0) => bail!(
            "{count} of {} mutations survived; each is a property nothing checks",
            MUTATIONS.len()
        ),
        (survived, unbuilt) => bail!(
            "{survived} of {} mutations survived and {unbuilt} would not build",
            MUTATIONS.len()
        ),
    }
}

/// Refuse source the already-running mutation driver would consume itself.
fn ensure_target_is_rebuilt(mutation: &Mutation) -> Result<()> {
    if mutation.file.starts_with("xtask/") {
        bail!(
            "{}: `{}` belongs to the running mutation driver and would not be rebuilt",
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

/// The check names in a self-test failure, one element at a time.
///
/// `correlate_selftest` joins them with `"; "` after the constant, so the list
/// is everything past it.
fn failed_checks(text: &str) -> impl Iterator<Item = &str> {
    text.split_once(qemu::SELFTEST_FAILURE)
        .map(|(_, rest)| rest.trim_start_matches(": "))
        .unwrap_or("")
        // The error may carry context after the list; a name never contains a
        // newline, so the first line is the list.
        .lines()
        .next()
        .unwrap_or("")
        .split("; ")
        .map(str::trim)
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
    match tasks::test_boot(paths, crate::cargo::Profile::Release) {
        // The kernel booted and every check passed: the mutation survived.
        Ok(()) => Ok(false),
        Err(error) => {
            let text = format!("{error:#}");
            match wanted {
                // The name comes from the machine's own report, forwarded by
                // `qemu::correlate_selftest`, not from anything inferred here.
                Caught::Check(name) => {
                    if text.contains(qemu::SELFTEST_FAILURE) {
                        // A **whole** element of the `"; "`-joined list, not a
                        // substring of it. A future check whose name contains
                        // another's would otherwise satisfy that other one —
                        // D-134's own defect, re-entered through the substring
                        // door (D-147).
                        Ok(failed_checks(&text).any(|failed| failed == name))
                    } else if text.contains(qemu::PANIC_FAILURE) {
                        // A panic where a check was named is not this check
                        // failing; it is the kernel dying before it could say.
                        Ok(false)
                    } else {
                        Err(error)
                    }
                }
                Caught::Panic(_) => {
                    if text.contains(qemu::PANIC_FAILURE) {
                        Ok(true)
                    } else if text.contains(qemu::SELFTEST_FAILURE) {
                        // Something went red where a panic was expected. The
                        // entry says nothing names this defect, and something
                        // now does — which is a better world and a wrong table.
                        Ok(false)
                    } else {
                        Err(error)
                    }
                }
                Caught::HostTest { .. } | Caught::BootMarker(_) => {
                    unreachable!("other witnesses return before self-test booting")
                }
            }
        }
    }
}

fn host_test_reports(paths: &Paths, package: &str, name: &str) -> Result<bool> {
    let output = Command::new(env!("CARGO"))
        .current_dir(paths.root())
        .args(["test", "--package", package, name, "--", "--exact"])
        .output()
        .with_context(|| format!("could not spawn host mutation test {package}::{name}"))?;
    if output.status.success() {
        return Ok(false);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let verdict = format!("test {name} ... FAILED");
    if stdout.contains(&verdict) || stderr.contains(&verdict) {
        Ok(true)
    } else {
        bail!(
            "host mutation did not reach `{name}`; cargo exited {}\n{stdout}{stderr}",
            output.status
        )
    }
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

    /// Every exact machine or host check a mutation names exists.
    ///
    /// The cheap half of D-134, and the one that runs at every commit. A
    /// mutation whose target check was renamed or deleted still boots, still
    /// reddens *something* on the way past, and still prints "caught" — so the
    /// expensive tier cannot see this, and the sweep costs a boot apiece.
    /// Matching the name against `selftest.rs` or the named host package's
    /// Rust sources costs milliseconds and catches it when the rename lands.
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
                Caught::BootMarker(marker) => {
                    let qemu = std::fs::read_to_string(root.join("xtask/src/qemu.rs"))
                        .expect("reading qemu.rs");
                    assert!(
                        qemu.contains(marker),
                        "`{}` names boot marker `{marker}`, which qemu.rs does not have",
                        mutation.breaks
                    );
                }
                Caught::Panic(_) => {}
            }
        }
    }

    /// Every mutation says what catching it looks like.
    ///
    /// A `Panic` with no reason would be the way to dodge naming a check, so
    /// the reason has to be long enough to be one.
    #[test]
    fn a_panicking_mutation_says_why_nothing_names_it() {
        for mutation in MUTATIONS {
            if let Caught::Panic(why) = mutation.reddens {
                assert!(
                    why.len() > 40,
                    "`{}` claims nothing names it and does not say why",
                    mutation.breaks
                );
            }
        }
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
