use super::*;

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
