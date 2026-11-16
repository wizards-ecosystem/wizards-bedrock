use super::*;

#[test]
fn slots_retire_rather_than_repeat_a_generation_and_stale_references_never_resolve() {
    let Some(mut slots) = Slots::<2>::new(u32::MAX - 3) else {
        panic!("a two-slot page is valid");
    };
    let mut removed: Vec<Issued> = Vec::new();
    while let Ok(issued) = slots.insert() {
        for stale in &removed {
            assert!(
                !slots.resolves(stale.slot, stale.generation),
                "{stale:?} resolved"
            );
            assert_eq!(
                slots.acquire(stale.slot, stale.generation),
                Err(Refusal::Stale)
            );
            assert_eq!(
                slots.release(stale.slot, stale.generation),
                Err(Refusal::Stale)
            );
            assert_eq!(
                slots.remove(stale.slot, stale.generation),
                Err(Refusal::Stale)
            );
        }
        assert!(!removed.contains(&issued), "{issued:?} was issued twice");
        assert!(
            slots.resolves(issued.slot, issued.generation),
            "{issued:?} did not resolve"
        );
        assert_eq!(
            slots.remove(issued.slot, issued.generation),
            Err(Refusal::Held)
        );
        assert_eq!(
            slots.release(issued.slot, issued.generation),
            Ok(Released::NowZero)
        );
        let expected = if issued.generation == u32::MAX {
            Removed::Retired
        } else {
            Removed::Freed
        };
        assert_eq!(slots.remove(issued.slot, issued.generation), Ok(expected));
        removed.push(issued);
        assert!(
            removed.len() <= 8,
            "a slot issued more than its four generations"
        );
    }
    assert_eq!(
        removed.len(),
        8,
        "a slot retired before its last generation"
    );
    assert_eq!(slots.insert(), Err(Refusal::Full));
    assert_eq!(slots.floor_after(0), None);
}
