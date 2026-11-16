use super::*;

#[test]
fn catching_up_never_leaves_a_requested_generation_unflushed() {
    for seen in 0u64..6 {
        for latest in 0u64..6 {
            for request in 0u64..6 {
                for extent in [Extent::Range, Extent::Whole] {
                    let case = format!("seen {seen} request {request} latest {latest} {extent:?}");
                    let (action, now) = catch_up(seen, request, latest, extent);
                    assert!(now >= seen && now >= request, "{case}: seen went to {now}");
                    match action {
                        Action::Skip => {
                            assert!(request <= seen, "{case}: an uncovered request skipped");
                            assert_eq!(now, seen, "{case}: a skip moved the generation");
                        }
                        Action::Range => {
                            assert_eq!(extent, Extent::Range, "{case}: a range from a whole");
                            assert_eq!(now, seen + 1, "{case}: a range covered several");
                            assert!(now >= latest, "{case}: a range left a later one");
                        }
                        Action::Whole => assert!(now >= latest, "{case}: whole fell short"),
                    }
                }
            }
        }
    }
}
