#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

use native_abi::limits::NAME_BYTES;

pub const SEPARATOR: u8 = b'/';

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathError {
    Empty,
    Absolute,
    Escapes,
    NameTooLong,
    BadName,
    TooDeep,
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
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

        if component.len() > NAME_BYTES {
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

// NUL truncates a name in C.
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
