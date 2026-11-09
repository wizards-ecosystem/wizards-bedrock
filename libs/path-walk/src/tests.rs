use super::*;

#[test]
fn every_path_error_describes_its_cause() {
    fn assert_error<T: core::error::Error>() {}

    assert_error::<PathError>();
    for (error, expected) in [
        (PathError::Empty, "the path is empty"),
        (PathError::Absolute, "the path is absolute"),
        (
            PathError::Escapes,
            "the path escapes its starting directory",
        ),
        (PathError::NameTooLong, "a path component is too long"),
        (
            PathError::BadName,
            "a path component contains an invalid byte",
        ),
        (
            PathError::TooDeep,
            "the path exceeds the component capacity",
        ),
    ] {
        assert_eq!(error.to_string(), expected);
    }
}

/// Split `path` into a fresh buffer, for tests that want the answer.
fn components(path: &[u8]) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, PathError> {
    let mut slots = [b"".as_slice(); 16];
    let count = split(path, &mut slots)?;
    Ok(slots[..count].iter().map(|part| part.to_vec()).collect())
}

extern crate alloc;

/// A plain path is its components, in order.
#[test]
fn a_plain_path_is_its_components() {
    assert_eq!(components(b"a").unwrap(), vec![b"a".to_vec()]);
    assert_eq!(
        components(b"one/two/three").unwrap(),
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
}

/// Empty components and `.` disappear without changing what is named.
///
/// A doubled separator and a trailing one are how an empty component
/// arrives, and both are ordinary in paths a program builds by
/// concatenation.
#[test]
fn empty_components_and_dots_disappear() {
    let plain = components(b"a/b").unwrap();
    for equivalent in [
        b"a//b".as_slice(),
        b"a/b/",
        b"a/./b",
        b"./a/b",
        b"a/b/.",
        b"a/././b//",
    ] {
        assert_eq!(components(equivalent).unwrap(), plain, "{equivalent:?}");
    }
}

/// `..` cancels the component before it, and the output never holds one.
///
/// The property the crate exists for: a walker fed these can only descend,
/// because there is nothing in the output that means "up".
#[test]
fn dot_dot_cancels_and_never_survives() {
    assert_eq!(components(b"a/../b").unwrap(), vec![b"b".to_vec()]);
    assert_eq!(
        components(b"a/b/../c").unwrap(),
        vec![b"a".to_vec(), b"c".to_vec()]
    );
    assert_eq!(components(b"a/b/../..").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(
        components(b"a/b/c/../../d").unwrap(),
        vec![b"a".to_vec(), b"d".to_vec()]
    );

    // And no output anywhere holds one, over every shape above.
    for path in [
        b"a/../b".as_slice(),
        b"a/b/../c",
        b"a/b/c/../../d",
        b"x/y/z/../../../q",
    ] {
        for part in components(path).unwrap() {
            assert_ne!(part, b"..".to_vec(), "{path:?} kept a ..");
            assert_ne!(part, b".".to_vec());
        }
    }
}

/// Ascending past the start is refused, from every direction.
///
/// Not clamped to the start, which is what a resolver that "just stays
/// inside" would do — and which silently answers a different question than
/// the caller asked. `012` §5 is explicit that the *capacity* to ascend is
/// what must be missing.
#[test]
fn ascending_past_the_start_is_refused_not_clamped() {
    for escape in [
        b"..".as_slice(),
        b"../a",
        b"a/../..",
        b"a/b/../../..",
        b"./..",
        b"a/./../..",
        b"a/../b/../..",
    ] {
        assert_eq!(
            components(escape),
            Err(PathError::Escapes),
            "{escape:?} should not resolve"
        );
    }

    // One short of escaping is fine, which is what makes the refusals
    // above about the boundary rather than about `..` at all.
    assert_eq!(components(b"a/..").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(components(b"a/b/../..").unwrap(), Vec::<Vec<u8>>::new());
}

/// An absolute path is malformed, and says so as itself.
///
/// Not `Escapes` and not a denial: there is no root, so there is nothing
/// for it to be relative to, and calling it a permission failure would
/// suggest some other program could.
#[test]
fn an_absolute_path_is_malformed_rather_than_denied() {
    for absolute in [b"/".as_slice(), b"/a", b"//a", b"/../a", b"/etc/passwd"] {
        assert_eq!(
            components(absolute),
            Err(PathError::Absolute),
            "{absolute:?}"
        );
    }
}

/// An empty path names nothing and says so.
#[test]
fn an_empty_path_names_nothing() {
    assert_eq!(components(b""), Err(PathError::Empty));
}

/// A name that is not one is refused rather than passed on.
///
/// A NUL is how a name crosses into C and back with its tail cut off, and a
/// name too long is a *different* name once truncated — which resolves to a
/// different file.
#[test]
fn a_name_that_is_not_one_is_refused() {
    assert_eq!(components(b"a/b\0c"), Err(PathError::BadName));
    assert_eq!(components(b"\0"), Err(PathError::BadName));

    let long = vec![b'x'; MAX_NAME];
    assert_eq!(components(&long).unwrap(), vec![long.clone()]);

    let longer = vec![b'x'; MAX_NAME + 1];
    assert_eq!(components(&longer), Err(PathError::NameTooLong));

    assert!(is_a_name(b"hello.txt"));
    assert!(!is_a_name(b"a/b"));
    assert!(!is_a_name(b"a\0b"));
}

/// A path deeper than the caller's buffer is refused, not truncated.
///
/// Truncation would resolve a *prefix* of what was asked for, which is a
/// directory the caller did not name — and on a walk that ends in an open,
/// opening the wrong thing.
///
/// The bound is on the **deepest point reached**, not on the answer, so a
/// path that goes down three and back to two is refused by a buffer of
/// two. That is deliberate: bounding the intermediate is what keeps the
/// work bounded by the buffer rather than by the path.
#[test]
fn a_path_deeper_than_the_buffer_is_refused() {
    let mut two = [b"".as_slice(); 2];
    assert_eq!(split(b"a/b", &mut two), Ok(2));
    assert_eq!(split(b"a/b/c", &mut two), Err(PathError::TooDeep));

    // `..` gives room back for what follows it...
    assert_eq!(split(b"a/b/../c", &mut two), Ok(2));
    // ...but a path that was three deep on the way is still refused.
    assert_eq!(split(b"a/b/c/../../d", &mut two), Err(PathError::TooDeep));

    // A larger buffer takes both, which is what makes the refusal about
    // the buffer rather than about the path.
    let mut four = [b"".as_slice(); 4];
    assert_eq!(split(b"a/b/c/../../d", &mut four), Ok(2));
}

/// Every path either resolves to a descent or is refused.
///
/// Exhaustive over short paths built from the pieces that matter, because
/// the property is about *every* input and a handful of examples is not.
/// Nothing here may panic, and nothing that resolves may hold a `..`.
#[test]
fn every_short_path_either_descends_or_is_refused() {
    const PIECES: [&[u8]; 5] = [b"a", b".", b"..", b"", b"b"];

    for first in PIECES {
        for second in PIECES {
            for third in PIECES {
                let mut path = alloc::vec::Vec::new();
                path.extend_from_slice(first);
                path.push(SEPARATOR);
                path.extend_from_slice(second);
                path.push(SEPARATOR);
                path.extend_from_slice(third);

                let mut slots = [b"".as_slice(); 8];
                match split(&path, &mut slots) {
                    Err(_) => {}
                    Ok(count) => {
                        for part in &slots[..count] {
                            assert_ne!(*part, b"..".as_slice(), "{path:?}");
                            assert_ne!(*part, b".".as_slice(), "{path:?}");
                            assert!(!part.is_empty(), "{path:?}");
                        }
                        // Never more components than separators plus one,
                        // so the input bounds the walk rather than the buffer.
                        assert!(count <= 3, "{path:?} produced {count}");
                    }
                }
            }
        }
    }
}
