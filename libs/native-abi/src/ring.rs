use crate::PAGE_BYTES;

// Bumped whenever a reader-visible layout changes.
pub const VERSION: u32 = 1;

pub const BYTES: usize = 3 * PAGE_BYTES;

pub const ENTRIES_AT: usize = PAGE_BYTES;

pub const COMPLETIONS_AT: usize = ENTRIES_AT + SUBMISSIONS as usize * SUBMISSION_BYTES;

pub const DATA_AT: usize = 2 * PAGE_BYTES;

pub const DATA_BYTES: usize = PAGE_BYTES;

pub const SUBMISSION_BYTES: usize = 32;

pub const COMPLETION_BYTES: usize = 16;

pub const SUBMISSIONS: u32 = 8;

pub const COMPLETIONS: u32 = 16;

pub mod header {
    pub const VERSION: usize = 0;
    pub const SUBMISSIONS: usize = 4;
    pub const COMPLETIONS: usize = 8;
    pub const SUBMISSION_HEAD: usize = 16;
    pub const SUBMISSION_TAIL: usize = 20;
    pub const COMPLETION_HEAD: usize = 24;
    pub const COMPLETION_TAIL: usize = 28;
}

mod submission {
    pub(super) const OPCODE: usize = 0;
    pub(super) const TOKEN: usize = 4;
    pub(super) const HANDLE: usize = 8;
    pub(super) const LENGTH: usize = 12;
    pub(super) const OFFSET: usize = 16;
    pub(super) const ARGUMENT: usize = 24;
}

mod completion {
    pub(super) const TOKEN: usize = 0;
    pub(super) const STATUS: usize = 4;
    pub(super) const VALUE: usize = 8;
}

pub mod status {
    pub const OK: u32 = 0;
    pub const UNKNOWN_OPCODE: u32 = 1;
    pub const NO_HANDLE_SLOT: u32 = 2;
    pub const BAD_HANDLE: u32 = 3;
    pub const DENIED: u32 = 4;
    pub const CANCELLED: u32 = 5;
    pub const NOT_FOUND: u32 = 6;
    pub const NO_ROOM: u32 = 7;
    pub const BAD_BUFFER: u32 = 8;
    pub const NO_ENTRY: u32 = 9;
    pub const BAD_PATH: u32 = 10;
    pub const UNAVAILABLE: u32 = 11;
    pub const IO: u32 = 12;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opcode {
    Echo,
    Uptime,
    Duplicate,
    Timer,
    Cancel,
    OpenAt,
    Read,
    Unknown(u32),
}

impl Opcode {
    const fn decode(value: u32) -> Self {
        match value {
            1 => Self::Echo,
            2 => Self::Uptime,
            3 => Self::Duplicate,
            4 => Self::Timer,
            5 => Self::Cancel,
            6 => Self::OpenAt,
            7 => Self::Read,
            other => Self::Unknown(other),
        }
    }

    const fn number(self) -> u32 {
        match self {
            Self::Echo => 1,
            Self::Uptime => 2,
            Self::Duplicate => 3,
            Self::Timer => 4,
            Self::Cancel => 5,
            Self::OpenAt => 6,
            Self::Read => 7,
            Self::Unknown(other) => other,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Submission {
    pub opcode: Opcode,
    pub token: u32,
    pub handle: u32,
    pub length: u32,
    pub offset: u64,
    pub argument: u64,
}

impl Submission {
    pub const fn about(opcode: Opcode, token: u32, handle: u32, argument: u64) -> Self {
        Self {
            opcode,
            token,
            handle,
            length: 0,
            offset: 0,
            argument,
        }
    }

    pub const fn simple(opcode: Opcode, token: u32, argument: u64) -> Self {
        Self::about(opcode, token, 0, argument)
    }

    #[must_use]
    pub const fn length(self, length: u32) -> Self {
        Self { length, ..self }
    }

    #[must_use]
    pub const fn offset(self, offset: u64) -> Self {
        Self { offset, ..self }
    }

    pub const fn decode(bytes: &[u8; SUBMISSION_BYTES]) -> Self {
        Self {
            opcode: Opcode::decode(read_u32(bytes, submission::OPCODE)),
            token: read_u32(bytes, submission::TOKEN),
            handle: read_u32(bytes, submission::HANDLE),
            length: read_u32(bytes, submission::LENGTH),
            offset: read_u64(bytes, submission::OFFSET),
            argument: read_u64(bytes, submission::ARGUMENT),
        }
    }

    pub fn encode(self) -> [u8; SUBMISSION_BYTES] {
        let mut bytes = [0u8; SUBMISSION_BYTES];
        write(
            &mut bytes,
            submission::OPCODE,
            &self.opcode.number().to_le_bytes(),
        );
        write(&mut bytes, submission::TOKEN, &self.token.to_le_bytes());
        write(&mut bytes, submission::HANDLE, &self.handle.to_le_bytes());
        write(&mut bytes, submission::LENGTH, &self.length.to_le_bytes());
        write(&mut bytes, submission::OFFSET, &self.offset.to_le_bytes());
        write(
            &mut bytes,
            submission::ARGUMENT,
            &self.argument.to_le_bytes(),
        );
        bytes
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Completion {
    pub token: u32,
    pub status: u32,
    pub value: u64,
}

impl Completion {
    pub const fn decode(bytes: &[u8; COMPLETION_BYTES]) -> Self {
        Self {
            token: read_u32(bytes, completion::TOKEN),
            status: read_u32(bytes, completion::STATUS),
            value: read_u64(bytes, completion::VALUE),
        }
    }

    pub fn encode(self) -> [u8; COMPLETION_BYTES] {
        let mut bytes = [0u8; COMPLETION_BYTES];
        write(&mut bytes, completion::TOKEN, &self.token.to_le_bytes());
        write(&mut bytes, completion::STATUS, &self.status.to_le_bytes());
        write(&mut bytes, completion::VALUE, &self.value.to_le_bytes());
        bytes
    }
}

const fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

const fn read_u64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes([
        bytes[at],
        bytes[at + 1],
        bytes[at + 2],
        bytes[at + 3],
        bytes[at + 4],
        bytes[at + 5],
        bytes[at + 6],
        bytes[at + 7],
    ])
}

fn write(bytes: &mut [u8], at: usize, field: &[u8]) {
    bytes[at..at + field.len()].copy_from_slice(field);
}
