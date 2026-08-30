//! Property tests over the crate's stated whole property.
//!
//! The module doc puts it in a blockquote, and it is what the crate is for:
//!
//! > No output of `split` contains `..`, and no output is longer than the
//! > input's component count. A walker fed these can only descend.
//!
//! The unit tests pin the paths a reader can check by hand. This asks the same
//! claim of paths nobody would write, because the resolver above holds no
//! parent pointer and D-171 rests the whole no-ascent argument on this
//! arithmetic being right.
//!
//! What is *not* here: anything about what a resolver does with these
//! components. Whether a walker honours the descent-only guarantee is a
//! property of the walker, and four machine checks in `selftest.rs` carry it.

use path_walk::{MAX_NAME, SEPARATOR, is_a_name, split};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, TestRunner};

/// Fragments carrying the syntax that matters. Uniform bytes almost never
/// produce a separator or a `..`, so a raw-byte strategy alone would exercise
/// the first guard and nothing behind it.
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

/// A path built by joining fragments with the one separator.
fn built_path() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(fragment(), 1..8).prop_map(|parts| parts.join(&SEPARATOR))
}

/// Either a built path or raw bytes; the second arm keeps the first honest.
fn path() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![built_path(), prop::collection::vec(any::<u8>(), 0..24)]
}

/// Separator-delimited, non-empty segments in the input.
fn component_count(path: &[u8]) -> usize {
    path.split(|byte| *byte == SEPARATOR)
        .filter(|segment| !segment.is_empty())
        .count()
}

proptest! {
    /// No output names an ancestor, and no output is longer than the input.
    ///
    /// The blockquote, mechanically. A `..` surviving into the output is the
    /// one defect that turns a descend-only resolver into one that ascends,
    /// and it would not show on any path a person thought to write down.
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

    /// Every component handed back is one the crate itself calls a name.
    ///
    /// `is_a_name` and `split` are separate public answers about the same
    /// bytes; a component the first rejects and the second emits is the two
    /// disagreeing, which no single-answer test can see.
    #[test]
    fn every_returned_component_is_a_name(path in path(), room in 1usize..12) {
        let mut into = vec![b"".as_slice(); room];
        let Ok(count) = split(&path, &mut into) else {
            return Ok(());
        };
        for component in &into[..count] {
            prop_assert!(component.len() <= MAX_NAME, "a component exceeded MAX_NAME");
            prop_assert!(
                is_a_name(component),
                "split emitted {:?}, which is_a_name rejects",
                component
            );
        }
    }

    /// More room never makes a path less acceptable.
    ///
    /// `TooDeep` is documented as a property of "the deepest point reached, not
    /// the result", so widening the caller's buffer can refuse fewer paths but
    /// must never change the components of one it already accepted.
    #[test]
    fn more_room_never_makes_a_path_less_acceptable(path in path(), room in 1usize..8) {
        let mut small = vec![b"".as_slice(); room];
        let mut large = vec![b"".as_slice(); room + 4];
        let tight = split(&path, &mut small);
        let loose = split(&path, &mut large);
        if let Ok(count) = tight {
            prop_assert_eq!(
                loose.as_ref().ok().copied(),
                Some(count),
                "a wider buffer changed the answer"
            );
            prop_assert_eq!(&small[..count], &large[..count], "a wider buffer changed the components");
        }
    }
}

/// How often the properties above get past `split`'s guards, stated rather
/// than assumed — D-242's obligation on a generated-input check.
#[test]
fn the_properties_reach_accepted_paths_and_every_refusal() {
    const SAMPLES: usize = 4096;
    let mut runner = TestRunner::new(Config {
        cases: SAMPLES as u32,
        ..Config::default()
    });
    let mut accepted = 0usize;
    let mut with_components = 0usize;
    let mut refused = 0usize;

    for _ in 0..SAMPLES {
        let value = path().new_tree(&mut runner).expect("a path").current();
        let mut into = [b"".as_slice(); 6];
        match split(&value, &mut into) {
            Ok(count) => {
                accepted += 1;
                if count > 0 {
                    with_components += 1;
                }
            }
            Err(_) => refused += 1,
        }
    }

    println!(
        "path-walk: {accepted}/{SAMPLES} accepted, {with_components} with components, {refused} refused"
    );
    assert!(
        with_components > SAMPLES / 8,
        "only {with_components} of {SAMPLES} paths produced components; the properties are near-vacuous"
    );
    assert!(
        refused > SAMPLES / 32,
        "only {refused} of {SAMPLES} paths were refused; the guards are never exercised"
    );
}
