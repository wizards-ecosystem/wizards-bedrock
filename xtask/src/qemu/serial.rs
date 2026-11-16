use anyhow::{Context, Result, ensure};
use boot_record::{Fact, Memory, Protections, Record, Verdict};
use panic_record::OPEN_PREFIX;

// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33.
pub(super) const SELFTEST_PASSED: i32 = 33;

pub(super) const FULL_PROTECTIONS: Record = Record::Protections(Protections {
    write_protect: true,
    smep: true,
    smap: true,
    umip: true,
    nx: true,
});

pub(super) const SELFTEST_REQUIRED: &[Record] = &[
    Record::Fact(Fact::DmaFenceStrict),
    Record::Fact(Fact::IsolationPairedForced),
    FULL_PROTECTIONS,
];

// An unreadable record fails the boot, so no record the gate does not know passes unread.
pub(crate) fn read_record(line: &str) -> Result<Option<Record>> {
    Record::parse(line)
        .transpose()
        .with_context(|| format!("reading the boot record {line:?}"))
}

pub(super) fn require_no_panic(line: &str, kernel: &str) -> Result<()> {
    ensure!(
        !line.starts_with(OPEN_PREFIX),
        "the {kernel} kernel panicked: {line}"
    );
    Ok(())
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Records {
    required: &'static [Record],
    seen: Vec<bool>,
}

impl Records {
    pub(super) fn new(required: &'static [Record]) -> Self {
        Self {
            required,
            seen: vec![false; required.len()],
        }
    }

    pub(super) fn observe(&mut self, record: Record) {
        for (required, seen) in self.required.iter().zip(&mut self.seen) {
            *seen |= *required == record;
        }
    }

    pub(super) fn require_all(&self, kernel: &str) -> Result<()> {
        for (record, seen) in self.required.iter().zip(&self.seen) {
            ensure!(*seen, "the {kernel} kernel never wrote `{record}`");
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Selftest {
    records: Records,
    failed: Vec<String>,
    verdict: Option<Verdict>,
    colours: Option<usize>,
}

impl Default for Selftest {
    fn default() -> Self {
        Self {
            records: Records::new(SELFTEST_REQUIRED),
            failed: Vec::new(),
            verdict: None,
            colours: None,
        }
    }
}

impl Selftest {
    pub(super) fn observe(&mut self, line: &str) -> Result<()> {
        ensure!(
            self.verdict.is_none(),
            "the self-test kernel printed serial data after its final verdict"
        );
        require_no_panic(line, "self-test")?;
        // Names for the message only; the verdict record alone decides.
        if let Some(name) = line
            .strip_prefix("[error] selftest: ")
            .and_then(|message| message.strip_suffix(" FAILED"))
        {
            self.failed.push(name.to_string());
        }
        match read_record(line)? {
            Some(Record::Verdict(verdict)) => self.verdict = Some(verdict),
            Some(Record::Memory(Memory { colours })) => self.colours = Some(colours),
            Some(record) => self.records.observe(record),
            None => {}
        }
        Ok(())
    }

    pub(super) fn conclude(self, exit: Option<i32>) -> Result<()> {
        let verdict = self
            .verdict
            .context("QEMU exited without a self-test verdict record")?;
        ensure!(
            verdict.is_clean_pass(),
            "the self-test verdict passed {} of {} checks with {} failures: {}",
            verdict.passed,
            verdict.expected,
            verdict.failed,
            self.failed.join("; ")
        );
        ensure!(
            exit == Some(SELFTEST_PASSED),
            "the self-test verdict record contradicts its debug-exit status ({exit:?})"
        );
        // Sibling worlds with disjoint colour masks need more than one colour to exist.
        let colours = self
            .colours
            .context("the self-test kernel never wrote its `memory` record")?;
        ensure!(
            colours > 1,
            "the self-test machine has {colours} cache colour, so colour masks cannot differ"
        );
        self.records.require_all("self-test")
    }
}

// Guest bytes are untrusted, so terminal controls never reach the operator's console.
pub(super) fn escaped(line: &[u8]) -> String {
    let visible = line.strip_suffix(b"\n").unwrap_or(line);
    let visible = visible.strip_suffix(b"\r").unwrap_or(visible);
    let mut escaped = String::with_capacity(visible.len());
    for &byte in visible {
        if (b' '..=b'~').contains(&byte) {
            escaped.push(char::from(byte));
        } else {
            escaped.push_str(&format!("\\x{byte:02x}"));
        }
    }
    escaped
}
