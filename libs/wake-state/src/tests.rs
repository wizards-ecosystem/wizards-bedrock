use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Waiter {
    Block,
    Resume,
    Done,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Waker {
    Claim,
    Place,
    Done,
}

#[test]
fn a_claim_after_registration_never_leaves_the_thread_blocked_and_a_late_claim_does_nothing() {
    for schedule in 0u32..16 {
        let mut word = step(Word::default(), Event::Park).unwrap();
        let claim = Event::Claim {
            sequence: word.sequence(),
        };
        let mut waiter = Waiter::Block;
        let mut waker = Waker::Claim;
        let mut off_cpu = false;
        let mut resumed = false;
        let mut placed = false;
        let mut turn = 0;
        while waiter != Waiter::Done || waker != Waker::Done {
            let waker_turn = schedule & (1 << turn) != 0;
            turn += 1;
            if (waker_turn && waker != Waker::Done) || waiter == Waiter::Done {
                match waker {
                    Waker::Claim => {
                        word = step(word, claim).unwrap();
                        waker = match word.state() {
                            Some(State::Woken) => Waker::Done,
                            Some(State::Waking) => Waker::Place,
                            other => panic!("schedule {schedule:#06b}: claim gave {other:?}"),
                        };
                    }
                    Waker::Place => {
                        assert!(off_cpu, "schedule {schedule:#06b}: placed while on a CPU");
                        word = step(word, Event::Place).unwrap();
                        placed = true;
                        waker = Waker::Done;
                    }
                    Waker::Done => unreachable!("a finished waker takes no turn"),
                }
            } else {
                match waiter {
                    Waiter::Block => match step(word, Event::Block) {
                        Some(blocked) => {
                            word = blocked;
                            off_cpu = true;
                            waiter = Waiter::Done;
                        }
                        None => waiter = Waiter::Resume,
                    },
                    Waiter::Resume => {
                        word = step(word, Event::Resume).unwrap();
                        resumed = true;
                        waiter = Waiter::Done;
                    }
                    Waiter::Done => unreachable!("a finished waiter takes no turn"),
                }
            }
        }
        assert_eq!(
            word.state(),
            Some(State::Running),
            "schedule {schedule:#06b} leaves the thread runnable"
        );
        assert!(
            resumed != placed,
            "schedule {schedule:#06b}: the wait completes exactly once"
        );

        let next = step(word, Event::Park).unwrap();
        assert_eq!(
            step(next, claim),
            None,
            "schedule {schedule:#06b}: a claim on the finished wait reaches the next one"
        );
    }
}
