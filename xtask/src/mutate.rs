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
//! removed, a flag not stripped, a guard inverted. Applying one and booting
//! must turn some check **red**. A mutation that survives is a hole, and the
//! report names it (D-119).
//!
//! **Two tiers, and the cheap one matters more.** `cargo test` checks that
//! every mutation's pattern still occurs exactly once in its file — no boot, no
//! build, milliseconds. A pattern that has stopped matching is a mutation that
//! silently stopped testing anything, which is precisely the failure mode this
//! whole file exists to catch, one level up. The expensive tier — build, boot,
//! require red, restore — is `cargo xtask mutate`, and it is not in `ci`
//! because it is one QEMU boot per mutation.
//!
//! **Restoring is not optional and not best-effort.** Every mutation is
//! applied to a file that is restored from an in-memory copy of the original
//! before the next one, and on every error path. A crash between the two would
//! leave the tree modified, which is why the mutation is written back before
//! anything is allowed to fail.

use anyhow::{Context, Result, bail};

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
        find: "        if self.bound && released.is_none_or(|released| released.root() != self.root) {",
        replace: "        if false && self.bound && released.is_none() {",
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
    // The arena's move, made to lose what it was moving. Aimed at D-128's own
    // check rather than at the lock order it also fixed: the leaf-nesting
    // property already has a mutation, and one defect per property is what
    // makes a survivor mean something.
    //
    // Nothing is lost on the first growth, because the arena is empty then.
    // The second is the one that drops live handles on the floor, and reaching
    // a second growth is the whole reason that check holds forty at once.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "            grown.append(&mut arena.slots);",
        replace: "            grown.clear();",
        breaks: "the handle arena grows more than once",
        reddens: Caught::Check("the handle arena grows more than once"),
    },
    // Slice 1's four properties, one defect each (D-119).
    //
    // The ring became a delegation path in Phase 6, and a delegation path that
    // does not narrow is a widening path. This is the whole of it: `install`
    // hands out what was offered instead of the narrower of offered and
    // declared.
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
        find: "                    if slot.is_some_and(|entry| entry.deadline_ns <= now_ns) {",
        replace: "                    if slot.is_some() {",
        breaks: "an operation is not due before its deadline",
        reddens: Caught::Check("an operation completes after its deadline and only once"),
    },
    // The cancellation stops posting the target's completion and reports only
    // its own — the double-completion class from `012` §8 with the sign
    // flipped, and the reason cancellation is an operation rather than a
    // substitute.
    Mutation {
        file: "kernel/src/queue.rs",
        find: "        if !self.post(Completion {
            token: entry.token,
            status: STATUS_CANCELLED,
            value: 0,
        }) {",
        replace: "        if false {",
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
        breaks: "the page and the call tell the same time",
        reddens: Caught::Check("the time page and the call tell the same time"),
    },
    // The window left open: the counter goes odd and never comes back, so
    // every correct reader spins for ever and gives up.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::end(Seq::begin(seq)));",
        replace: "    write_u32(page, CLOCK_AT + clock::SEQ, Seq::begin(seq));",
        breaks: "a published clock closes its update window",
        reddens: Caught::Check("the time page and the call tell the same time"),
    },
    // The revocation gate ignored. A kernel that has stopped trusting its
    // counter goes on telling every program the counter is good, which is the
    // one failure the gate exists to prevent.
    Mutation {
        file: "kernel/src/timepage.rs",
        find: "    write_u32(page, CLOCK_AT + clock::MODE, mode);",
        replace: "    write_u32(page, CLOCK_AT + clock::MODE, MODE_COUNTER);",
        breaks: "a revoked clock reads as revoked",
        reddens: Caught::Check("revoking the clock sends readers back to the call"),
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
        find: "            if !self.post(Completion {
                token: entry.token,
                status: STATUS_CANCELLED,
                value: 0,
            }) {
                debug_assert!(false, \"an abandoned operation had no reserved slot\");
                break;
            }
            ended += 1;",
        replace: "            ended += 1;",
        breaks: "a dead program's work is finished, not forgotten",
        reddens: Caught::Check("work in flight is ended before the ring is released"),
    },
    // The promise given a table of its own, so the one it was made against
    // dies under it. This is the use-after-free D-163's `Arc` exists to
    // prevent, and it reddens four checks at once.
    Mutation {
        file: "kernel/src/handle.rs",
        find: "        Ok(SlotReservation {
            table: Arc::clone(self),
            spent: false,
        })",
        replace: "        Ok(SlotReservation {
            table: Arc::new(HandleTable::new(self.quota)),
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
    // Two drivers for one device, which is the whole of D-77's entitlement
    // complaint arriving as a defect rather than as an argument.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "        if claimed
            .iter()
            .flatten()
            .any(|held| held.bits() == at.bits())
        {",
        replace: "        if false
            && claimed
                .iter()
                .flatten()
                .any(|held| held.bits() == at.bits())
        {",
        breaks: "a device is claimed once",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
    },
    // The ports a device decodes left free for anyone to claim, which is the
    // arrangement D-77 said proves the wrong thing.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "            match wost::arch::current::port::claim_span(port, size as usize) {",
        replace: "            match wost::arch::current::port::claim_span(port, 0) {",
        breaks: "a device claim covers the ports it decodes",
        reddens: Caught::Check("a claimed device's io window is not claimable bare"),
    },
    // A bridge's bus numbers decoded as a base address and then probed —
    // all ones written into the register that says which bus is behind it.
    Mutation {
        file: "kernel/src/pci.rs",
        find: "    let registers = if layout.is_bridge() {
        BRIDGE_BARS
    } else {
        BARS
    };",
        replace: "    let registers = BARS;
    let _ = layout;",
        breaks: "a bridge has two base address registers, not six",
        reddens: Caught::Check("a device is claimed once and a refusal strands nothing"),
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
];

/// Apply each mutation, boot, and require something to go red.
///
/// # Errors
///
/// If a pattern no longer matches, if the tree cannot be restored, or if any
/// mutation **survives** — which is the interesting failure and the reason
/// this exists.
pub fn run(paths: &Paths) -> Result<()> {
    let mut survivors = Vec::new();

    for (index, mutation) in MUTATIONS.iter().enumerate() {
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

        let noticed = boot_reports(paths, mutation.reddens);

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
                println!("[xtask]   inconclusive: {error}");
                survivors.push(mutation);
            }
        }
    }

    if survivors.is_empty() {
        println!("[xtask] every mutation was caught by the check it names");
        return Ok(());
    }
    for survivor in &survivors {
        println!("[xtask] survived: {} ({})", survivor.breaks, survivor.file);
    }
    bail!(
        "{} of {} mutations survived; each is a property nothing checks",
        survivors.len(),
        MUTATIONS.len()
    );
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

/// Build and boot the self-test kernel, and report whether **this** mutation's
/// check went red.
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
fn boot_reports(paths: &Paths, wanted: Caught) -> Result<bool> {
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Every check a mutation names is a check that exists.
    ///
    /// The cheap half of D-134, and the one that runs at every commit. A
    /// mutation whose target check was renamed or deleted still boots, still
    /// reddens *something* on the way past, and still prints "caught" — so the
    /// expensive tier cannot see this, and the sweep costs a boot apiece.
    /// Matching the name against `selftest.rs`'s own `report.check` calls
    /// costs milliseconds and catches it the moment the rename lands.
    #[test]
    fn every_named_check_exists() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask sits below the workspace root");
        let selftest = std::fs::read_to_string(root.join("kernel/src/selftest.rs"))
            .expect("reading selftest.rs");

        for mutation in MUTATIONS {
            let Caught::Check(name) = mutation.reddens else {
                continue;
            };
            // The literal as `report.check` is called with it. Searching for
            // the quoted name rather than the whole call handles both the
            // one-line and the wrapped forms rustfmt produces.
            assert!(
                selftest.contains(&format!("\"{name}\"")),
                "`{}` names the check `{name}`, which `selftest.rs` does not have",
                mutation.breaks
            );
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
    /// A duplicate is a boot spent twice on one question, and this task's cost
    /// is one boot per entry.
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
