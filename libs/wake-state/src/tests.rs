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
fn a_claim_after_registration_never_leaves_the_thread_blocked() {
    for schedule in 0u32..16 {
        let mut word = step(State::Running.word(), Event::Park).unwrap().word();
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
                    Waker::Claim => match step(word, Event::Claim) {
                        Some(State::Woken) => {
                            word = State::Woken.word();
                            waker = Waker::Done;
                        }
                        Some(State::Waking) => {
                            word = State::Waking.word();
                            waker = Waker::Place;
                        }
                        other => {
                            panic!("schedule {schedule:#06b}: claim from {word} gave {other:?}")
                        }
                    },
                    Waker::Place => {
                        assert!(off_cpu, "schedule {schedule:#06b}: placed while on a CPU");
                        word = step(word, Event::Place).unwrap().word();
                        placed = true;
                        waker = Waker::Done;
                    }
                    Waker::Done => unreachable!("a finished waker takes no turn"),
                }
            } else {
                match waiter {
                    Waiter::Block => match step(word, Event::Block) {
                        Some(blocked) => {
                            word = blocked.word();
                            off_cpu = true;
                            waiter = Waiter::Done;
                        }
                        None => waiter = Waiter::Resume,
                    },
                    Waiter::Resume => {
                        word = step(word, Event::Resume).unwrap().word();
                        resumed = true;
                        waiter = Waiter::Done;
                    }
                    Waiter::Done => unreachable!("a finished waiter takes no turn"),
                }
            }
        }
        assert_eq!(
            State::from_word(word),
            Some(State::Running),
            "schedule {schedule:#06b} leaves the thread runnable"
        );
        assert!(
            resumed != placed,
            "schedule {schedule:#06b}: the wait completes exactly once"
        );
    }
}
