//! Path splitting whose output can never name an ancestor.
//!
//! > No output of [`split`] contains `..`, and no output is longer than the
//! > input's component count. A walker fed these can only descend.
//!
//! # Examples
//!
//! ```
//! use path_walk::{split, PathError};
//!
//! let mut components = [b"".as_slice(); 8];
//!
//! // `.` disappears and `..` cancels the component before it.
//! let count = split(b"a/./b/../c", &mut components).expect("a valid path");
//! assert_eq!(&components[..count], &[b"a".as_slice(), b"c".as_slice()]);
//!
//! // Ascending past the handle is refused, not clamped.
//! assert_eq!(split(b"a/../..", &mut components), Err(PathError::Escapes));
//!
//! // And there is no root to be absolute against.
//! assert_eq!(split(b"/etc", &mut components), Err(PathError::Absolute));
//! ```

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

/// The separator, and the only one.
pub const SEPARATOR: u8 = b'/';

/// The longest a single component may be.
pub const MAX_NAME: usize = 255;

/// Why a path could not be split.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PathError {
    /// The path is empty, which names nothing.
    Empty,
    /// The path begins with a separator.
    Absolute,
    /// A `..` that would leave the directory it started from.
    Escapes,
    /// A component longer than [`MAX_NAME`].
    NameTooLong,
    /// A component containing a byte no name may hold.
    BadName,
    /// The path went deeper than the caller left room for.
    TooDeep,
}

impl core::fmt::Display for PathError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Write::write_str(
            f,
            match self {
                Self::Empty => "the path is empty",
                Self::Absolute => "the path is absolute",
                Self::Escapes => "the path escapes its starting directory",
                Self::NameTooLong => "a path component is too long",
                Self::BadName => "a path component contains an invalid byte",
                Self::TooDeep => "the path exceeds the component capacity",
            },
        )
    }
}

impl core::error::Error for PathError {}

/// Split `path` into components, resolving `.` and `..` as it goes.
///
/// # Errors
///
/// See [`PathError`]. Every one of them is a refusal — nothing is clamped,
/// truncated or silently dropped, because each of those turns a path the caller
/// wrote into a different path that resolves.
pub fn split<'a>(path: &'a [u8], into: &mut [&'a [u8]]) -> Result<usize, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path[0] == SEPARATOR {
        return Err(PathError::Absolute);
    }

    let mut depth: usize = 0;
    let mut start = 0;

    // One past the end on purpose, so the last component is closed by the loop
    // rather than by a copy of the body after it.
    for index in 0..=path.len() {
        let at_end = index == path.len();
        if !at_end && path[index] != SEPARATOR {
            continue;
        }

        let component = &path[start..index];
        start = index + 1;

        // `a//b` is `a/b`. Empty components are how a trailing separator and a
        // doubled one both arrive, and neither names anything.
        if component.is_empty() {
            continue;
        }
        if component == b"." {
            continue;
        }
        if component == b".." {
            // Popping here means the output never contains `..`; underflow is
            // a refusal, because clamping would answer a different question.
            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;
            continue;
        }

        if component.len() > MAX_NAME {
            return Err(PathError::NameTooLong);
        }
        if !is_a_name(component) {
            return Err(PathError::BadName);
        }
        if depth >= into.len() {
            return Err(PathError::TooDeep);
        }
        into[depth] = component;
        depth += 1;
    }

    Ok(depth)
}

/// Whether every byte of `component` may appear in a name.
fn is_a_name(component: &[u8]) -> bool {
    for &byte in component {
        if byte == 0 || byte == SEPARATOR {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
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
}
