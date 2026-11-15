use anyhow::{Context, Result, bail, ensure};
use panic_record::OPEN_PREFIX;

// QEMU exits with `(value << 1) | 1`, so the kernel's 0x10 arrives as 33.
pub(super) const SELFTEST_PASSED: i32 = 33;

pub(super) const STRICT_DMA_HANDOFF: &str = "[info ] iommu: entry DMA fence policy strict";
pub(super) const SHIPPED_ISOLATION: &str =
    "[info ] cpu: user isolation shared kernel mappings: qemu_tcg";
pub(super) const FULL_PROTECTIONS: &str = "[info ] cpu: protections wp smep smap umip nx";
pub(super) const NVME_FALLBACK: &str = "[info ] nvme: candidate 1 retired cleanly; trying next";
pub(super) const SETTLED: &str = "[info ] kernel: reading the keyboard, uptime ";
pub(super) const USER_CALL: &str = "[info ] user: ring 3 call observed";
pub(super) const KEY_CONSUMED: &str = "[info ] input: key consumed";

pub(super) const SELFTEST_REQUIRED: &[&str] = &[
    STRICT_DMA_HANDOFF,
    "[info ] cpu: user isolation paired roots: forced for self-test; shared proof retained: qemu_tcg",
    FULL_PROTECTIONS,
];

const VERDICT_PREFIX: &str = "selftest: verdict ";
const VERDICT_VERSION: &str = "v1";
const NVME_STACK_PREFIX: &str = "[info ] selftest: nvme-stack v2 ";
const NVME_STACK_CAPACITY: u64 = 32 * 1024;
const NVME_STACK_MIN_HEADROOM: u64 = 2 * 4096;

// Each marker opens a keystroke round, answered in order by the matching keys.
pub(super) const ROUNDS: &[(&str, &[&str])] = &[
    ("[info ] selftest: awaiting storage-delay keystroke", &["c"]),
    (
        "[info ] selftest: awaiting injected keystroke",
        &["a", "backspace", "ret"],
    ),
    ("[info ] selftest: awaiting translated keystroke", &["b"]),
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Status {
    Pass,
    Fail,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Verdict {
    pub(crate) status: Status,
    pub(crate) passed: usize,
    pub(crate) failed: usize,
    pub(crate) expected: usize,
}

impl Verdict {
    pub(crate) fn parse(line: &str) -> Option<Result<Self>> {
        let (severity, message) = match line.split_at_checked(8)? {
            ("[info ] ", message) => (Status::Pass, message),
            ("[error] ", message) => (Status::Fail, message),
            _ => return None,
        };
        let fields = message.strip_prefix(VERDICT_PREFIX)?;
        Some(Self::parse_fields(fields, severity))
    }

    fn parse_fields(fields: &str, severity: Status) -> Result<Self> {
        let fields: Vec<&str> = fields.split(' ').collect();
        let [version, status, passed, failed, expected] = fields.as_slice() else {
            bail!("the self-test verdict does not have five fields");
        };
        ensure!(
            *version == VERDICT_VERSION,
            "unsupported self-test verdict version"
        );
        let status = match *status {
            "PASS" => Status::Pass,
            "FAIL" => Status::Fail,
            _ => bail!("invalid self-test verdict status"),
        };
        ensure!(
            status == severity,
            "self-test verdict status has the wrong log severity"
        );
        Ok(Self {
            status,
            passed: count(passed, "passed=")?,
            failed: count(failed, "failed=")?,
            expected: count(expected, "expected=")?,
        })
    }

    pub(crate) fn is_clean_pass(self) -> bool {
        self.status == Status::Pass
            && self.failed == 0
            && self.expected != 0
            && self.passed == self.expected
    }
}

fn count(field: &str, name: &str) -> Result<usize> {
    let value = field
        .strip_prefix(name)
        .with_context(|| format!("the self-test verdict has no {name} count"))?;
    ensure!(
        !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()),
        "the self-test verdict has a malformed {name} count"
    );
    value
        .parse()
        .context("a self-test verdict count overflowed")
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
    required: &'static [&'static str],
    seen: Vec<bool>,
}

impl Records {
    pub(super) fn new(required: &'static [&'static str]) -> Self {
        Self {
            required,
            seen: vec![false; required.len()],
        }
    }

    pub(super) fn observe(&mut self, line: &str) {
        for (record, seen) in self.required.iter().zip(&mut self.seen) {
            *seen |= line.starts_with(record);
        }
    }

    pub(super) fn require_all(&self, kernel: &str) -> Result<()> {
        for (record, seen) in self.required.iter().zip(&self.seen) {
            ensure!(*seen, "the {kernel} kernel never reported {record:?}");
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Selftest {
    records: Records,
    failed: Vec<String>,
    nvme_stack_checked: bool,
    verdict: Option<Verdict>,
}

impl Default for Selftest {
    fn default() -> Self {
        Self {
            records: Records::new(SELFTEST_REQUIRED),
            failed: Vec::new(),
            nvme_stack_checked: false,
            verdict: None,
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
        self.records.observe(line);
        if let Some(name) = line
            .strip_prefix("[error] selftest: ")
            .and_then(|message| message.strip_suffix(" FAILED"))
        {
            self.failed.push(name.to_string());
        } else if let Some(fields) = line.strip_prefix(NVME_STACK_PREFIX) {
            ensure!(!self.nvme_stack_checked, "the NVMe stack record repeated");
            require_nvme_stack(fields)?;
            self.nvme_stack_checked = true;
        } else if let Some(verdict) = Verdict::parse(line) {
            self.verdict = Some(verdict?);
        }
        Ok(())
    }

    pub(super) fn conclude(self, exit: Option<i32>) -> Result<()> {
        let verdict = self
            .verdict
            .context("QEMU exited without a serial self-test verdict")?;
        if verdict.status == Status::Fail || !self.failed.is_empty() {
            bail!(
                "the self-test reported failures: {}",
                self.failed.join("; ")
            );
        }
        ensure!(
            verdict.is_clean_pass(),
            "the self-test verdict passed {} of {} checks with {} failures",
            verdict.passed,
            verdict.expected,
            verdict.failed
        );
        ensure!(
            exit == Some(SELFTEST_PASSED),
            "the self-test serial verdict contradicts its debug-exit status ({exit:?})"
        );
        self.records.require_all("self-test")?;
        ensure!(
            self.nvme_stack_checked,
            "the self-test printed no NVMe stack record"
        );
        Ok(())
    }
}

fn require_nvme_stack(record: &str) -> Result<()> {
    let fields: Vec<&str> = record.split(' ').collect();
    let [used, capacity, headroom, leaf] = fields.as_slice() else {
        bail!("the NVMe stack record does not have four fields");
    };
    let used = stack_value(used, "used=")?;
    let capacity = stack_value(capacity, "capacity=")?;
    let headroom = stack_value(headroom, "headroom=")?;
    let leaf = stack_value(leaf, "leaf=")?;
    ensure!(
        capacity == NVME_STACK_CAPACITY
            && used.checked_add(headroom) == Some(capacity)
            && leaf <= used,
        "the NVMe stack record is inconsistent: {record}"
    );
    ensure!(
        headroom >= NVME_STACK_MIN_HEADROOM,
        "NVMe candidates kept only {headroom} bytes of stack headroom"
    );
    Ok(())
}

fn stack_value(field: &str, name: &str) -> Result<u64> {
    field
        .strip_prefix(name)
        .and_then(|value| value.parse().ok())
        .with_context(|| format!("the NVMe stack record has no {name} value"))
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
