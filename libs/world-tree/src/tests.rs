use super::*;

const SIBLING: u32 = 1000;

fn ancestor(distance: u8) -> NodeRef {
    NodeRef {
        node: 100 + u32::from(distance),
        generation: 7,
    }
}

fn sibling(distance: u8) -> NodeRef {
    NodeRef {
        node: SIBLING + u32::from(distance),
        generation: 3,
    }
}

#[test]
fn a_creation_enters_every_ancestor_set_or_leaves_each_unchanged() {
    let child = NodeRef {
        node: 9,
        generation: 1,
    };
    for depth in 1..=MAX_DEPTH {
        for failing in 0..=depth {
            let mut path = Path::ROOT;
            for distance in (0..depth).rev() {
                path = path
                    .child(ancestor(distance))
                    .expect("depth is within the limit");
            }
            let mut states = vec![WorldState::Live; usize::from(depth)];
            let mut sets: Vec<Descendants<4>> = Vec::new();
            for distance in 0..depth {
                let mut set = Descendants::EMPTY;
                set.insert(WorldState::Live, sibling(distance))
                    .expect("room for a sibling");
                sets.push(set);
            }
            if failing < depth {
                states[usize::from(failing)] = WorldState::Dying;
            }
            let before = sets.clone();

            let mut creation = Creation::new(&path).expect("a child has a parent");
            for _ in 0..4 * usize::from(MAX_DEPTH) + 8 {
                let event = match creation.step() {
                    CreationStep::Insert(distance) => {
                        let node = path
                            .nearest(distance)
                            .expect("an ancestor at that distance");
                        let at = (node.node - 100) as usize;
                        match sets[at].insert(states[at], child) {
                            Ok(()) => CreationEvent::Done,
                            Err(_) => CreationEvent::Halted(CreationRefusal::Dead),
                        }
                    }
                    CreationStep::Remove(distance, _) => {
                        let node = path
                            .nearest(distance)
                            .expect("an ancestor at that distance");
                        sets[(node.node - 100) as usize]
                            .remove(child)
                            .expect("entered before");
                        CreationEvent::Done
                    }
                    CreationStep::Unwound(_) | CreationStep::Published => break,
                    _ => CreationEvent::Done,
                };
                creation = creation.next(event).expect("each step takes its event");
            }

            if failing < depth {
                assert_eq!(
                    creation.step(),
                    CreationStep::Unwound(CreationRefusal::Dead),
                    "depth {depth} failing at {failing}"
                );
                for (set, old) in sets.iter().zip(&before) {
                    assert_eq!(
                        set.count(),
                        old.count(),
                        "depth {depth} failing at {failing}"
                    );
                    for position in 0..old.count() {
                        let entry = old.get(position).expect("within the old count");
                        assert!(set.contains(entry), "depth {depth} failing at {failing}");
                    }
                    assert!(!set.contains(child), "depth {depth} failing at {failing}");
                }
            } else {
                assert_eq!(creation.step(), CreationStep::Published, "depth {depth}");
                for set in &sets {
                    assert!(
                        set.contains(child),
                        "depth {depth}: an ancestor misses the child"
                    );
                }
            }
        }
    }
}
