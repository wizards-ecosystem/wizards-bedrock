#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]

use core::fmt;

// Bumped with any change to the grammar, the kinds, their fields or the fact names.
pub const VERSION: u32 = 4;

// The kernel writes each record as one `info` line, and nothing else it logs opens this way.
const LEVEL: &str = "[info ] ";

const OPEN: &str = "record ";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Record {
    Verdict(Verdict),
    Fact(Fact),
    Protections(Protections),
    Memory(Memory),
    Processors(Processors),
    Exit(Exit),
    Fault(Fault),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Verdict {
    pub passed: usize,
    pub failed: usize,
    pub expected: usize,
}

impl Verdict {
    pub const fn is_clean_pass(self) -> bool {
        self.failed == 0 && self.expected != 0 && self.passed == self.expected
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Protections {
    pub write_protect: bool,
    pub smep: bool,
    pub smap: bool,
    pub umip: bool,
    pub nx: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Memory {
    pub colours: usize,
}

// Processors brought online at boot, the boot processor included.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Processors {
    pub online: usize,
}

// How a program ended, written when it is collected.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Exit {
    pub status: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fault {
    pub vector: u64,
    pub error: u64,
    pub address: u64,
    pub rip: u64,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fact {
    DmaFenceStrict,
    IsolationShared,
    IsolationPairedForced,
    IommuTranslating,
    NvmeCandidateRetired,
    NvmeUp,
    NvmeNone,
    StorageUnisolated,
    Fat32Mounted,
    TmpfsListed,
    DevfsResolved,
    KeyboardLinesMasked,
    KeyboardPolled,
    X2apicAbsent,
    ExtendedStateAbsent,
    RingThreeClosedToInterrupts,
    Settled,
    KeyConsumed,
}

impl Fact {
    const ALL: [Self; 18] = [
        Self::DmaFenceStrict,
        Self::IsolationShared,
        Self::IsolationPairedForced,
        Self::IommuTranslating,
        Self::NvmeCandidateRetired,
        Self::NvmeUp,
        Self::NvmeNone,
        Self::StorageUnisolated,
        Self::Fat32Mounted,
        Self::TmpfsListed,
        Self::DevfsResolved,
        Self::KeyboardLinesMasked,
        Self::KeyboardPolled,
        Self::X2apicAbsent,
        Self::ExtendedStateAbsent,
        Self::RingThreeClosedToInterrupts,
        Self::Settled,
        Self::KeyConsumed,
    ];

    const fn name(self) -> &'static str {
        match self {
            Self::DmaFenceStrict => "dma-fence-strict",
            Self::IsolationShared => "isolation-shared",
            Self::IsolationPairedForced => "isolation-paired-forced",
            Self::IommuTranslating => "iommu-translating",
            Self::NvmeCandidateRetired => "nvme-candidate-retired",
            Self::NvmeUp => "nvme-up",
            Self::NvmeNone => "nvme-none",
            Self::StorageUnisolated => "storage-unisolated",
            Self::Fat32Mounted => "fat32-mounted",
            Self::TmpfsListed => "tmpfs-listed",
            Self::DevfsResolved => "devfs-resolved",
            Self::KeyboardLinesMasked => "keyboard-lines-masked",
            Self::KeyboardPolled => "keyboard-polled",
            Self::X2apicAbsent => "x2apic-absent",
            Self::ExtendedStateAbsent => "extended-state-absent",
            Self::RingThreeClosedToInterrupts => "ring3-closed-to-interrupts",
            Self::Settled => "settled",
            Self::KeyConsumed => "key-consumed",
        }
    }

    fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|fact| fact.name() == name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    Malformed,
    UnknownVersion,
    UnknownKind,
    UnknownFact,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Malformed => "the record does not follow its grammar",
            Self::UnknownVersion => "the record has a version this reader does not know",
            Self::UnknownKind => "the record has a kind this reader does not know",
            Self::UnknownFact => "the record names a fact this reader does not know",
        })
    }
}

impl core::error::Error for Error {}

impl Record {
    // `None` for a line that is not a record; a record line that does not parse is an error.
    pub fn parse(line: &str) -> Option<Result<Self, Error>> {
        Some(Self::parse_body(
            line.strip_prefix(LEVEL)?.strip_prefix(OPEN)?,
        ))
    }

    fn parse_body(body: &str) -> Result<Self, Error> {
        let mut words = Words(body.split(' '));
        let version: u32 = words
            .word()?
            .strip_prefix('v')
            .and_then(decimal)
            .ok_or(Error::Malformed)?;
        if version != VERSION {
            return Err(Error::UnknownVersion);
        }
        let record = match words.word()? {
            "verdict" => Self::Verdict(Verdict {
                passed: words.count("passed")?,
                failed: words.count("failed")?,
                expected: words.count("expected")?,
            }),
            "fact" => Self::Fact(Fact::named(words.value("name")?).ok_or(Error::UnknownFact)?),
            "protections" => Self::Protections(Protections {
                write_protect: words.flag("wp")?,
                smep: words.flag("smep")?,
                smap: words.flag("smap")?,
                umip: words.flag("umip")?,
                nx: words.flag("nx")?,
            }),
            "memory" => Self::Memory(Memory {
                colours: words.count("colours")?,
            }),
            "processors" => Self::Processors(Processors {
                online: words.count("online")?,
            }),
            "exit" => Self::Exit(Exit {
                status: words.count("status")?,
            }),
            "fault" => Self::Fault(Fault {
                vector: words.count("vector")?,
                error: words.count("error")?,
                address: words.count("address")?,
                rip: words.count("rip")?,
            }),
            _ => return Err(Error::UnknownKind),
        };
        words.end()?;
        Ok(record)
    }
}

impl fmt::Display for Record {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{OPEN}v{VERSION} ")?;
        match *self {
            Self::Verdict(Verdict {
                passed,
                failed,
                expected,
            }) => write!(
                formatter,
                "verdict passed={passed} failed={failed} expected={expected}"
            ),
            Self::Fact(fact) => write!(formatter, "fact name={}", fact.name()),
            Self::Protections(protections) => write!(
                formatter,
                "protections wp={} smep={} smap={} umip={} nx={}",
                u8::from(protections.write_protect),
                u8::from(protections.smep),
                u8::from(protections.smap),
                u8::from(protections.umip),
                u8::from(protections.nx)
            ),
            Self::Memory(Memory { colours }) => write!(formatter, "memory colours={colours}"),
            Self::Processors(Processors { online }) => {
                write!(formatter, "processors online={online}")
            }
            Self::Exit(Exit { status }) => write!(formatter, "exit status={status}"),
            Self::Fault(Fault {
                vector,
                error,
                address,
                rip,
            }) => write!(
                formatter,
                "fault vector={vector} error={error} address={address} rip={rip}"
            ),
        }
    }
}

struct Words<'a>(core::str::Split<'a, char>);

impl<'a> Words<'a> {
    fn word(&mut self) -> Result<&'a str, Error> {
        self.0
            .next()
            .filter(|word| !word.is_empty())
            .ok_or(Error::Malformed)
    }

    fn value(&mut self, key: &str) -> Result<&'a str, Error> {
        match self.word()?.split_once('=') {
            Some((named, value)) if named == key && !value.is_empty() => Ok(value),
            _ => Err(Error::Malformed),
        }
    }

    fn count<T: core::str::FromStr>(&mut self, key: &str) -> Result<T, Error> {
        decimal(self.value(key)?).ok_or(Error::Malformed)
    }

    fn flag(&mut self, key: &str) -> Result<bool, Error> {
        match self.value(key)? {
            "0" => Ok(false),
            "1" => Ok(true),
            _ => Err(Error::Malformed),
        }
    }

    fn end(mut self) -> Result<(), Error> {
        match self.0.next() {
            Some(_) => Err(Error::Malformed),
            None => Ok(()),
        }
    }
}

// Canonical digits only, so one value has one spelling.
fn decimal<T: core::str::FromStr>(digits: &str) -> Option<T> {
    let canonical = !digits.is_empty()
        && digits.bytes().all(|byte| byte.is_ascii_digit())
        && (digits == "0" || !digits.starts_with('0'));
    if !canonical {
        return None;
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests;
