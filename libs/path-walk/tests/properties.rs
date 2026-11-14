//! Property tests that no output of `split` names an ancestor or outgrows its input.

use path_walk::{SEPARATOR, split};
use proptest::prelude::*;

/// Fragments carrying separators and dots, which uniform bytes almost never produce.
fn fragment() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        Just(b"..".to_vec()),
        Just(b".".to_vec()),
        Just(b"a".to_vec()),
        Just(b"bb".to_vec()),
        Just(Vec::new()),
        prop::collection::vec(any::<u8>(), 0..4),
        prop::collection::vec(Just(b'z').boxed(), 250..260),
    ]
}

fn built_path() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(fragment(), 1..8).prop_map(|parts| parts.join(&SEPARATOR))
}

/// Either a built path or raw bytes; the second arm keeps the first honest.
fn path() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![built_path(), prop::collection::vec(any::<u8>(), 0..24)]
}

fn component_count(path: &[u8]) -> usize {
    path.split(|byte| *byte == SEPARATOR)
        .filter(|segment| !segment.is_empty())
        .count()
}

proptest! {
    #[test]
    fn no_split_output_can_name_an_ancestor(path in path(), room in 1usize..12) {
        let mut into = vec![b"".as_slice(); room];
        let Ok(count) = split(&path, &mut into) else {
            return Ok(());
        };
        prop_assert!(count <= room, "split reported more components than room");
        for component in &into[..count] {
            prop_assert_ne!(*component, b"..".as_slice(), "an output named an ancestor");
            prop_assert_ne!(*component, b".".as_slice(), "a no-op survived as a name");
            prop_assert!(!component.is_empty(), "an empty component was emitted");
        }
        prop_assert!(
            count <= component_count(&path),
            "split returned {} components from an input with {}",
            count,
            component_count(&path)
        );
    }

}
