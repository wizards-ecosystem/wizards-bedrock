//! Concurrency permutation over the shape a ring's counters will sit in.
//!
//! `Ring` and `Queues` hold plain integers and no atomics: the crate takes
//! *copies* of head and tail and never reads shared memory (D-79,
//! CVE-2025-39816 is literally "always use `READ_ONCE()` to read ring
//! pointers"). So what shuttle permutes here is not the crate's arithmetic but
//! the thing the arithmetic will sit behind — two sides advancing a shared
//! counter — and that is the part a single CPU cannot show and a unit test
//! cannot fail on (D-70 rule 6).
//!
//! **What shuttle does not do, established by trying it (D-112).** It explores
//! thread *schedules* under sequential consistency; it does not model store
//! buffering or reordering. A model whose only fault was `Relaxed` where
//! `Release`/`Acquire` belonged passed ten thousand schedules, because under
//! every schedule the program-order store still happened first. Memory
//! ordering is the TLA+ model's and the reviewer's; interleaving is shuttle's.
//! Asking the wrong tool and believing the answer is the failure this comment
//! exists to prevent.
//!
//! **And it only sees its own primitives.** `std::sync::atomic` is invisible
//! to the scheduler — a model built on one has no preemption points at all, so
//! every schedule is the same schedule and everything passes. The imports below
//! are `shuttle::sync`'s deliberately; the first version of this file used
//! `std`'s and proved nothing twice over (D-112).
//!
//! **The last test here must fail if the harness works.** A version with a
//! read-modify-write split into a load and a store is included and asserted to
//! be found broken: a passing wrong version is the only proof that a passing
//! right version means anything (D-84).

use shuttle::sync::Arc;
use shuttle::sync::atomic::{AtomicU32, Ordering};
use shuttle::thread;

/// How many times each side advances the counter.
const ROUNDS: u32 = 4;

/// Two sides, each advancing a shared counter `ROUNDS` times.
///
/// With `atomic` set the advance is one `fetch_add`; without it, the load and
/// the store are separate operations and a schedule can land between them.
fn advance_together(atomic: bool) {
    let counter = Arc::new(AtomicU32::new(0));
    let sides: Vec<_> = (0..2)
        .map(|_| {
            let counter = Arc::clone(&counter);
            thread::spawn(move || {
                for _ in 0..ROUNDS {
                    if atomic {
                        counter.fetch_add(1, Ordering::AcqRel);
                    } else {
                        let seen = counter.load(Ordering::Acquire);
                        counter.store(seen + 1, Ordering::Release);
                    }
                }
            })
        })
        .collect();
    for side in sides {
        side.join().expect("a side finished");
    }
    assert_eq!(
        counter.load(Ordering::Acquire),
        2 * ROUNDS,
        "an advance was lost"
    );
}

/// Two sides advancing one counter atomically never lose an advance.
///
/// The shape a submission tail and a completion head will have: two writers,
/// one number, and no lock between them. What shuttle checks is that no
/// schedule loses one.
#[test]
fn no_schedule_loses_an_advance() {
    shuttle::check_random(|| advance_together(true), 10_000);
}

/// And the harness can tell: the same model with the advance split into a load
/// and a store is found broken.
///
/// Not a test of the crate — a test of the tool. A concurrency harness that
/// passes a version known to be wrong has proved nothing about the version it
/// passed, and this is the cheapest way to keep that honest (D-84).
///
/// `should_panic` rather than an assertion on the outcome, because what
/// shuttle does on finding a violation *is* to panic with the schedule.
#[test]
#[should_panic(expected = "an advance was lost")]
fn the_harness_finds_a_split_advance_broken() {
    shuttle::check_random(|| advance_together(false), 10_000);
}
