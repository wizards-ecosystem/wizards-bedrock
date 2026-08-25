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
    /// Human-facing, and deliberately *not* the check's name: only six of
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
        find: "        let granted = offered.attenuate(declared).ok_or(HandleError::Denied)?;",
        replace: "        let granted = offered.attenuate(declared).unwrap_or(offered);",
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
                        Ok(text.contains(name))
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
