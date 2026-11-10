//! Path splitting whose output can never name an ancestor.
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

use core::fmt;

pub const SEPARATOR: u8 = b'/';

/// The longest a component may be; longer is refused, since a truncated name finds another file.
pub const MAX_NAME: usize = 255;

/// Why a path could not be split.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PathError {
    /// The path is empty, which names nothing.
    Empty,
    /// The path begins with a separator, a malformed argument since there is no root.
    Absolute,
    /// A `..` that would leave the directory it started from.
    Escapes,
    /// A component longer than [`MAX_NAME`].
    NameTooLong,
    /// A component containing a byte no name may hold.
    BadName,
    /// The deepest point the path reached, not its result, exceeded the caller's room.
    TooDeep,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "the path is empty",
            Self::Absolute => "the path is absolute",
            Self::Escapes => "the path escapes its starting directory",
            Self::NameTooLong => "a path component is too long",
            Self::BadName => "a path component contains an invalid byte",
            Self::TooDeep => "the path exceeds the component capacity",
        })
    }
}

impl core::error::Error for PathError {}

/// Split `path` into borrowed components of `into`, resolving `.` and `..`, and return the count.
pub fn split<'path>(path: &'path [u8], into: &mut [&'path [u8]]) -> Result<usize, PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path[0] == SEPARATOR {
        return Err(PathError::Absolute);
    }

    let mut depth: usize = 0;
    let mut start = 0;

    for index in 0..=path.len() {
        let at_end = index == path.len();
        if !at_end && path[index] != SEPARATOR {
            continue;
        }

        let component = &path[start..index];
        start = index + 1;

        if component.is_empty() {
            continue;
        }
        if component == b"." {
            continue;
        }
        if component == b".." {
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

/// Whether `component` is free of separators and of the NUL that truncates a name in C.
fn is_a_name(component: &[u8]) -> bool {
    for &byte in component {
        if byte == 0 || byte == SEPARATOR {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests;
