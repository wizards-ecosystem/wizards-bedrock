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

fn components(path: &[u8]) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, PathError> {
    let mut slots = [b"".as_slice(); 16];
    let count = split(path, &mut slots)?;
    Ok(slots[..count].iter().map(|part| part.to_vec()).collect())
}

extern crate alloc;

#[test]
fn a_plain_path_is_its_components() {
    assert_eq!(components(b"a").unwrap(), vec![b"a".to_vec()]);
    assert_eq!(
        components(b"one/two/three").unwrap(),
        vec![b"one".to_vec(), b"two".to_vec(), b"three".to_vec()]
    );
}

/// Doubled and trailing separators are ordinary in paths a program builds by concatenation.
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

/// A walker fed these can only descend, because nothing in the output means up.
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

/// Clamping to the start would silently answer a different question than the caller asked.
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

    assert_eq!(components(b"a/..").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(components(b"a/b/../..").unwrap(), Vec::<Vec<u8>>::new());
}

/// There is no root, so a permission failure would suggest some other program could.
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

#[test]
fn an_empty_path_names_nothing() {
    assert_eq!(components(b""), Err(PathError::Empty));
}

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

/// The bound is on the deepest point reached, so the work is bounded by the buffer.
#[test]
fn a_path_deeper_than_the_buffer_is_refused() {
    let mut two = [b"".as_slice(); 2];
    assert_eq!(split(b"a/b", &mut two), Ok(2));
    assert_eq!(split(b"a/b/c", &mut two), Err(PathError::TooDeep));

    assert_eq!(split(b"a/b/../c", &mut two), Ok(2));
    assert_eq!(split(b"a/b/c/../../d", &mut two), Err(PathError::TooDeep));

    let mut four = [b"".as_slice(); 4];
    assert_eq!(split(b"a/b/c/../../d", &mut four), Ok(2));
}

/// Exhaustive over short paths built from the pieces that matter.
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
                        assert!(count <= 3, "{path:?} produced {count}");
                    }
                }
            }
        }
    }
}
