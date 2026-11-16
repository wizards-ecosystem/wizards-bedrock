use super::*;

#[derive(Clone, Copy, Debug)]
enum Operation {
    AcquireWriter,
    Seal(Seal),
}

fn apply(bits: u64, operation: Operation) -> Result<u64, StateRefusal> {
    let word = StateWord::from_bits(bits).expect("every word written is well formed");
    let next = match operation {
        Operation::AcquireWriter => word.acquire_writer(),
        Operation::Seal(seal) => word.seal(seal),
    }?;
    Ok(next.bits())
}

#[test]
fn a_seal_succeeds_only_from_writable_with_no_writer_in_either_order() {
    for seal in [Seal::Read, Seal::Execute] {
        let orders = [
            [Operation::AcquireWriter, Operation::Seal(seal)],
            [Operation::Seal(seal), Operation::AcquireWriter],
        ];
        for order in orders {
            let mut bits = StateWord::WRITABLE.bits();
            let mut sealed = false;
            let mut writers = 0;
            for operation in order {
                match (operation, apply(bits, operation)) {
                    (Operation::AcquireWriter, Ok(next)) => {
                        assert!(!sealed, "{order:?}: a writer was acquired after the seal");
                        writers += 1;
                        bits = next;
                    }
                    (Operation::AcquireWriter, Err(refusal)) => {
                        assert!(sealed, "{order:?}: a writable object refused a writer");
                        assert_eq!(refusal, StateRefusal::Sealed);
                    }
                    (Operation::Seal(_), Ok(next)) => {
                        assert_eq!(writers, 0, "{order:?}: sealed with a live writer");
                        sealed = true;
                        bits = next;
                    }
                    (Operation::Seal(_), Err(refusal)) => {
                        assert!(writers > 0, "{order:?}: a seal with no writer was refused");
                        assert_eq!(refusal, StateRefusal::Busy);
                    }
                }
            }
            let word = StateWord::from_bits(bits).expect("the final word is well formed");
            assert_eq!(sealed, word.state() != ObjectState::Writable, "{order:?}");
            assert_eq!(word.writers(), writers, "{order:?}");
            let again = if sealed {
                StateRefusal::Sealed
            } else {
                StateRefusal::Busy
            };
            assert_eq!(
                apply(bits, Operation::Seal(Seal::Read)),
                Err(again),
                "{order:?}"
            );
        }
    }
}
