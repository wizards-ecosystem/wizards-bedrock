use crate::*;

#[test]
fn attenuation_never_widens_the_held_rights() {
    let held = Rights::READ.union(Rights::WRITE).union(Rights::TRANSFER);

    assert_eq!(held.union(Rights::READ), held, "union removed a held right");
    assert_eq!(held.attenuate(Rights::READ), Some(Rights::READ));
    assert_eq!(held.attenuate(Rights::NONE), Some(Rights::NONE));
    assert_eq!(held.attenuate(held), Some(held));
    assert_eq!(
        held.intersection(Rights::UNIVERSAL),
        Rights::TRANSFER,
        "intersection restored or discarded a held universal right"
    );

    assert_eq!(held.attenuate(Rights::EXECUTE), None, "gained a right");
    assert_eq!(
        held.attenuate(held.union(Rights::MAP)),
        None,
        "gained a right by asking for everything plus one"
    );

    for bits in 0..=Rights::ALL.bits() {
        let Some(wanted) = Rights::from_bits(bits) else {
            continue;
        };
        if let Some(attenuated) = held.attenuate(wanted) {
            assert!(held.contains(attenuated), "attenuation widened {bits:#x}");
        }
    }
}
