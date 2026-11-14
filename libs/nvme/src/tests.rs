use super::*;

/// Exhaustive over every head and tail, because a sample misses the one pair the rule is about.
#[test]
fn a_queue_refuses_the_entry_that_would_make_full_look_empty() {
    let capacity = Capacity::new(4).expect("four entries");

    for head in 0..4 {
        for tail in 0..4 {
            let room = capacity.has_room(head, tail).expect("valid indices");
            let would_wrap_onto_the_head = Capacity::next(capacity, tail) == Some(head);
            assert_eq!(room, !would_wrap_onto_the_head, "head {head}, tail {tail}");
        }
    }

    let mut tail = 0;
    let mut admitted = 0;
    for _ in 0..capacity.entries() {
        if capacity.has_room(0, tail) != Some(true) {
            break;
        }
        tail = Capacity::next(capacity, tail).expect("valid tail");
        admitted += 1;
    }
    assert_eq!(admitted, capacity.entries() - 1);
    assert_eq!(admitted, 3);
}

/// Exhaust every count around the sixteen-bit, zero-based NLB field.
#[test]
fn every_nonempty_nlb_count_has_one_exact_boundary() {
    let namespace = u64::from(MAX_TRANSFER_BLOCKS) + 1;
    for blocks in 1..=MAX_TRANSFER_BLOCKS + 1 {
        let transfer = ReadTransfer::new(0, blocks, namespace, 1, namespace);
        if blocks <= MAX_TRANSFER_BLOCKS {
            let transfer = transfer.expect("the NLB field can encode this count");
            assert_eq!(u32::from(transfer.zero_based_blocks()), blocks - 1);
            assert_eq!(transfer.bytes(), u64::from(blocks));
        } else {
            assert_eq!(transfer, Err(TransferError::TooManyBlocks));
        }
    }
}

/// The namespace end is exclusive and its addition never wraps.
#[test]
fn a_transfer_never_crosses_or_wraps_its_namespace() {
    let last = ReadTransfer::new(9, 1, 10, 512, 512).expect("the final block");
    assert_eq!(last.lba(), 9);
    assert_eq!(last.bytes(), 512);
    assert_eq!(
        ReadTransfer::new(9, 2, 10, 512, 1024),
        Err(TransferError::OutsideNamespace)
    );
    assert_eq!(
        ReadTransfer::new(u64::MAX, 1, u64::MAX, 1, 1),
        Err(TransferError::OutsideNamespace)
    );
    assert_eq!(
        ReadTransfer::new(0, 1, 0, 1, 1),
        Err(TransferError::OutsideNamespace)
    );
}
