//! Path splitting whose output can never name an ancestor.
//!
//! **There is no `/`** (`012` §5). An absolute path is a *malformed argument*,
//! not a permission failure, because there is no root to resolve it against: a
//! program holds directory handles and nothing else, and "the top" is not a
//! thing this kernel has a name for.
//!
//! **`..` is resolved lexically, here, before any node is touched.** A
//! component vector that pops on `..` and refuses on underflow means the
//! resolver above never holds a parent pointer — so **there is no operation
//! that can walk upward**. That is stronger than `RESOLVE_BENEATH`, which
//! enforces a predicate on a resolver still capable of ascending; the capacity
//! is what is missing here, not the permission.
//!
//! Fuchsia reached the same answer — *Dot Dot Considered Harmful* — and moved
//! it client-side. This kernel has no trusted client library, so it goes in the
//! resolver, which is to say here (D-171).
//!
//! **The property, and it is what the whole crate is for:**
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
///
/// 255, which is what every filesystem this kernel will read agrees on and
/// what a `u8` length field holds. Longer is refused rather than truncated: a
/// truncated name is a *different* name, and resolving it finds a different
/// file.
pub const MAX_NAME: usize = 255;

/// Why a path could not be split.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[non_exhaustive]
pub enum PathError {
    /// The path is empty, which names nothing.
    Empty,
    /// The path begins with a separator.
    ///
    /// **A malformed argument, not a denial.** There is no root, so there is
    /// nothing for an absolute path to be relative to, and reporting it as a
    /// permission failure would suggest that some other program could.
    Absolute,
    /// A `..` that would leave the directory it started from.
    Escapes,
    /// A component longer than [`MAX_NAME`].
    NameTooLong,
    /// A component containing a byte no name may hold.
    BadName,
    /// The path went deeper than the caller left room for.
    ///
    /// The **deepest point reached**, not the result: `a/b/c/../../d` is two
    /// components long and passes through three, so a buffer of two refuses
    /// it. Bounding the intermediate rather than the answer is what keeps the
    /// work bounded by the caller's buffer and not by the path's length.
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
/// Returns how many of `into` were filled. The components borrow from `path`,
/// so nothing is copied and nothing is allocated.
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
///
/// A separator cannot — the splitter has already removed them — and neither can
/// a NUL, which is how a name crosses into C and back with its tail cut off.
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
