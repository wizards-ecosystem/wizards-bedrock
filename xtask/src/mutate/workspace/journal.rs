use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::sha256;

use super::source::{Baseline, finish_staged_operation};
use super::tree::{
    canonical_existing_real_directory, require_direct_file, require_direct_file_if_present,
};
use super::{Outcome, read_state_text};

const JOURNAL_MAGIC: &str = "wizard-mutation-journal-v1";
const REPORT_MAGIC: &str = "wizard-mutation-report-v2";
const JOURNAL_SLOTS: [&str; 2] = ["journal.0", "journal.1"];
const REPORT_PREFIX: &str = "run.";
const REPORT_SUFFIX: &str = ".report";
pub(super) const LAST_RUN: &str = "last-run";
static NEXT_STORE_TEMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Journal {
    pub(super) generation: u64,
    pub(super) commit: String,
    pub(super) registry: String,
    pub(super) source_digest: String,
    pub(super) index_digest: String,
    pub(super) active: Option<usize>,
    pub(super) completed: Vec<Outcome>,
}

impl Journal {
    pub(super) fn new(baseline: &Baseline, registry: &str) -> Self {
        Self {
            generation: 0,
            commit: baseline.commit.clone(),
            registry: registry.to_string(),
            source_digest: baseline.source_digest.clone(),
            index_digest: baseline.index_digest.clone(),
            active: None,
            completed: Vec::new(),
        }
    }

    pub(super) fn matches(
        &self,
        baseline: &Baseline,
        registry: &str,
        mutation_count: usize,
    ) -> Result<()> {
        ensure!(
            self.commit == baseline.commit,
            "journal belongs to another commit"
        );
        ensure!(
            self.registry == registry,
            "journal belongs to another mutation registry"
        );
        ensure!(
            self.source_digest == baseline.source_digest,
            "source differs from the interrupted sweep"
        );
        ensure!(
            self.index_digest == baseline.index_digest,
            "index differs from the interrupted sweep"
        );
        ensure!(
            self.completed.len() <= mutation_count,
            "journal contains more outcomes than the registry"
        );
        if let Some(active) = self.active {
            ensure!(
                active == self.completed.len() && active < mutation_count,
                "journal's active mutation is not the next mutation"
            );
        }
        Ok(())
    }

    fn body(&self) -> String {
        let mut body = format!(
            "{JOURNAL_MAGIC}\ngeneration {}\ncommit {}\nregistry {}\nsource {}\nindex {}\nactive {}\ncompleted {}\n",
            self.generation,
            self.commit,
            self.registry,
            self.source_digest,
            self.index_digest,
            self.active
                .map_or_else(|| "none".to_string(), |index| index.to_string()),
            self.completed.len()
        );
        for outcome in &self.completed {
            push_outcome(&mut body, outcome);
        }
        body
    }

    pub(super) fn encode_with_checksum(&self) -> String {
        let body = self.body();
        let digest = sha256::hex(&sha256::digest(body.as_bytes()));
        format!("{body}checksum {digest}\n")
    }

    fn parse(text: &str) -> Result<Self> {
        let mut lines = checksummed_lines(text, "journal")?.into_iter();
        ensure!(lines.next() == Some(JOURNAL_MAGIC), "wrong journal version");
        let generation = <u64 as core::str::FromStr>::from_str(field(&mut lines, "generation")?)
            .context("parsing journal generation")?;
        let commit = field(&mut lines, "commit")?.to_string();
        let registry = field(&mut lines, "registry")?.to_string();
        let source_digest = field(&mut lines, "source")?.to_string();
        let index_digest = field(&mut lines, "index")?.to_string();
        let active = match field(&mut lines, "active")? {
            "none" => None,
            index => Some(
                <usize as core::str::FromStr>::from_str(index)
                    .context("parsing active mutation")?,
            ),
        };
        let count = <usize as core::str::FromStr>::from_str(field(&mut lines, "completed")?)
            .context("parsing completed mutation count")?;
        ensure!(
            is_hex(&commit, 40)
                && is_digest(&registry)
                && is_digest(&source_digest)
                && is_digest(&index_digest),
            "journal identity field is malformed"
        );

        let mut completed = Vec::with_capacity(count);
        for _ in 0..count {
            let line = lines.next().context("journal outcome is missing")?;
            completed.push(parse_outcome(line, "journal")?);
        }
        ensure!(lines.next().is_none(), "journal has trailing records");
        Ok(Self {
            generation,
            commit,
            registry,
            source_digest,
            index_digest,
            active,
            completed,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ReportVerdict {
    Pass,
    Fail,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Report {
    pub(super) sequence: u64,
    pub(super) commit: String,
    pub(super) registry: String,
    pub(super) source_digest: String,
    pub(super) index_digest: String,
    pub(super) caught: usize,
    pub(super) survived: usize,
    pub(super) inconclusive: usize,
    pub(super) verdict: ReportVerdict,
    pub(super) ids: Vec<u16>,
    pub(super) outcomes: Vec<Outcome>,
}

impl Report {
    pub(super) fn from(journal: &Journal, ids: &[u16]) -> Self {
        let (caught, survived, inconclusive) = outcome_totals(&journal.completed);
        Self {
            sequence: 0,
            commit: journal.commit.clone(),
            registry: journal.registry.clone(),
            source_digest: journal.source_digest.clone(),
            index_digest: journal.index_digest.clone(),
            caught,
            survived,
            inconclusive,
            verdict: if survived == 0 && inconclusive == 0 {
                ReportVerdict::Pass
            } else {
                ReportVerdict::Fail
            },
            ids: ids.to_vec(),
            outcomes: journal.completed.clone(),
        }
    }

    pub(super) fn body(&self) -> String {
        let verdict = match self.verdict {
            ReportVerdict::Pass => "pass",
            ReportVerdict::Fail => "fail",
        };
        let mut body = format!(
            "{REPORT_MAGIC}\nsequence {}\ncommit {}\nregistry {}\nsource {}\nindex {}\nmutations {}\ncaught {}\nsurvived {}\ninconclusive {}\nverdict {verdict}\n",
            self.sequence,
            self.commit,
            self.registry,
            self.source_digest,
            self.index_digest,
            self.outcomes.len(),
            self.caught,
            self.survived,
            self.inconclusive,
        );
        for (id, outcome) in self.ids.iter().zip(&self.outcomes) {
            body.push_str(&format!("id {id} "));
            push_outcome(&mut body, outcome);
        }
        body
    }

    pub(super) fn encode_with_checksum(&self) -> String {
        let body = self.body();
        let digest = sha256::hex(&sha256::digest(body.as_bytes()));
        format!("{body}checksum {digest}\n")
    }

    pub(super) fn parse(text: &str) -> Result<Self> {
        ensure!(
            text.ends_with('\n') && !text.contains('\r'),
            "mutation report is not canonical LF text"
        );
        let mut lines = checksummed_lines(text, "mutation report")?.into_iter();
        ensure!(
            lines.next() == Some(REPORT_MAGIC),
            "wrong mutation report version"
        );
        let sequence = <u64 as core::str::FromStr>::from_str(field(&mut lines, "sequence")?)
            .context("parsing mutation report sequence")?;
        ensure!(sequence != 0, "mutation report sequence is zero");
        let commit = field(&mut lines, "commit")?.to_string();
        let registry = field(&mut lines, "registry")?.to_string();
        let source_digest = field(&mut lines, "source")?.to_string();
        let index_digest = field(&mut lines, "index")?.to_string();
        let mutation_count =
            <usize as core::str::FromStr>::from_str(field(&mut lines, "mutations")?)
                .context("parsing mutation count")?;
        let caught = <usize as core::str::FromStr>::from_str(field(&mut lines, "caught")?)
            .context("parsing caught count")?;
        let survived = <usize as core::str::FromStr>::from_str(field(&mut lines, "survived")?)
            .context("parsing survived count")?;
        let inconclusive =
            <usize as core::str::FromStr>::from_str(field(&mut lines, "inconclusive")?)
                .context("parsing inconclusive count")?;
        let verdict = match field(&mut lines, "verdict")? {
            "pass" => ReportVerdict::Pass,
            "fail" => ReportVerdict::Fail,
            _ => bail!("mutation report verdict is malformed"),
        };
        ensure!(
            is_hex(&commit, 40)
                && is_digest(&registry)
                && is_digest(&source_digest)
                && is_digest(&index_digest),
            "mutation report identity field is malformed"
        );

        let mut ids = Vec::with_capacity(mutation_count);
        let mut outcomes = Vec::with_capacity(mutation_count);
        for _ in 0..mutation_count {
            let line = lines.next().context("mutation report outcome is missing")?;
            let (id, line) = line
                .strip_prefix("id ")
                .and_then(|line| line.split_once(' '))
                .context("mutation report outcome names no mutation")?;
            ids.push(
                <u16 as core::str::FromStr>::from_str(id)
                    .context("parsing mutation report identity")?,
            );
            outcomes.push(parse_outcome(line, "mutation report")?);
        }
        ensure!(
            lines.next().is_none(),
            "mutation report has trailing records"
        );
        let actual = outcome_totals(&outcomes);
        ensure!(
            actual == (caught, survived, inconclusive),
            "mutation report totals do not match its outcomes"
        );
        let expected_verdict = if survived == 0 && inconclusive == 0 {
            ReportVerdict::Pass
        } else {
            ReportVerdict::Fail
        };
        ensure!(
            verdict == expected_verdict,
            "mutation report verdict contradicts its totals"
        );
        let report = Self {
            sequence,
            commit,
            registry,
            source_digest,
            index_digest,
            caught,
            survived,
            inconclusive,
            verdict,
            ids,
            outcomes,
        };
        ensure!(
            report.encode_with_checksum() == text,
            "mutation report is not in its canonical encoding"
        );
        Ok(report)
    }

    pub(super) fn require_pass(
        &self,
        baseline: &Baseline,
        registry: &str,
        mutation_count: usize,
    ) -> Result<()> {
        ensure!(
            self.commit == baseline.commit
                && self.registry == registry
                && self.source_digest == baseline.source_digest
                && self.index_digest == baseline.index_digest
                && self.outcomes.len() == mutation_count,
            "mutation report is stale or belongs to different inputs"
        );
        ensure!(
            self.verdict == ReportVerdict::Pass
                && self.caught == mutation_count
                && self.survived == 0
                && self.inconclusive == 0,
            "the newest mutation report did not pass"
        );
        Ok(())
    }
}

fn outcome_totals(outcomes: &[Outcome]) -> (usize, usize, usize) {
    let mut caught = 0;
    let mut survived = 0;
    let mut inconclusive = 0;
    for outcome in outcomes {
        match outcome {
            Outcome::Caught => caught += 1,
            Outcome::Survived => survived += 1,
            Outcome::Inconclusive(_) => inconclusive += 1,
        }
    }
    (caught, survived, inconclusive)
}

fn checksummed_lines<'text>(text: &'text str, label: &str) -> Result<Vec<&'text str>> {
    let mut lines: Vec<&str> = text.lines().collect();
    let checksum = lines
        .pop()
        .with_context(|| format!("{label} has no checksum"))?;
    let expected = checksum
        .strip_prefix("checksum ")
        .with_context(|| format!("{label} checksum line is malformed"))?;
    ensure!(is_digest(expected), "{label} checksum is malformed");
    let body = format!("{}\n", lines.join("\n"));
    ensure!(
        sha256::hex(&sha256::digest(body.as_bytes())) == expected,
        "{label} checksum does not match"
    );
    Ok(lines)
}

fn push_outcome(body: &mut String, outcome: &Outcome) {
    match outcome {
        Outcome::Caught => body.push_str("result caught\n"),
        Outcome::Survived => body.push_str("result survived\n"),
        Outcome::Inconclusive(error) => {
            body.push_str("result inconclusive ");
            body.push_str(&hex_encode(error.as_bytes()));
            body.push('\n');
        }
    }
}

fn parse_outcome(line: &str, label: &str) -> Result<Outcome> {
    Ok(match line {
        "result caught" => Outcome::Caught,
        "result survived" => Outcome::Survived,
        _ => {
            let encoded = line
                .strip_prefix("result inconclusive ")
                .with_context(|| format!("{label} outcome is malformed"))?;
            Outcome::Inconclusive(
                String::from_utf8(hex_decode(encoded)?)
                    .with_context(|| format!("{label} error is not UTF-8"))?,
            )
        }
    })
}

fn field<'text>(lines: &mut impl Iterator<Item = &'text str>, name: &str) -> Result<&'text str> {
    let line = lines
        .next()
        .with_context(|| format!("journal field {name} is missing"))?;
    line.strip_prefix(&format!("{name} "))
        .with_context(|| format!("journal field {name} is malformed"))
}

pub(super) struct JournalStore {
    pub(super) directory: PathBuf,
}

impl JournalStore {
    pub(super) fn new(directory: PathBuf) -> Result<Self> {
        Ok(Self {
            directory: canonical_existing_real_directory(&directory, "mutation journal directory")?,
        })
    }

    fn require_directory(&self) -> Result<()> {
        ensure!(
            canonical_existing_real_directory(&self.directory, "mutation journal directory")?
                == self.directory,
            "mutation journal directory changed after it was opened"
        );
        Ok(())
    }

    pub(super) fn slot(&self, generation: u64) -> PathBuf {
        self.directory
            .join(JOURNAL_SLOTS[generation as usize % JOURNAL_SLOTS.len()])
    }

    pub(super) fn load(&self) -> Result<Option<Journal>> {
        self.require_directory()?;
        let mut present = 0;
        let mut valid = Vec::new();
        let mut errors = Vec::new();
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if !require_direct_file_if_present(&self.directory, &path, "mutation journal slot")? {
                continue;
            }
            present += 1;
            match read_state_text(&path, "mutation journal slot")
                .and_then(|text| Journal::parse(&text))
            {
                Ok(journal) => valid.push(journal),
                Err(error) => errors.push(format!("{}: {error:#}", path.display())),
            }
        }
        if let Some(journal) = valid.into_iter().max_by_key(|journal| journal.generation) {
            return Ok(Some(journal));
        }
        if present == 0 {
            Ok(None)
        } else {
            bail!(
                "no complete mutation journal remains:\n{}",
                errors.join("\n")
            )
        }
    }

    pub(super) fn save(&self, journal: &mut Journal) -> Result<()> {
        self.require_directory()?;
        journal.generation = journal
            .generation
            .checked_add(1)
            .context("mutation journal generation exhausted")?;
        let path = self.slot(journal.generation);
        let encoded = journal.encode_with_checksum();
        self.replace_file(&path, encoded.as_bytes(), "mutation journal slot")
    }

    pub(super) fn save_report(&self, mut report: Report) -> Result<Report> {
        self.require_directory()?;
        let next = self.latest_report()?.map_or(Ok(1), |(_, report)| {
            report
                .sequence
                .checked_add(1)
                .context("mutation report sequence exhausted")
        })?;
        report.sequence = next;
        let path = self.directory.join(report_name(next));
        let encoded = report.encode_with_checksum();
        let mut options = OpenOptions::new();
        OpenOptions::create_new(&mut options, true);
        OpenOptions::write(&mut options, true);
        let mut file = OpenOptions::open(&options, &path)
            .with_context(|| format!("creating immutable mutation report {}", path.display()))?;
        file.write_all(encoded.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", path.display()))?;
        require_direct_file(&self.directory, &path, "immutable mutation report")?;
        ensure!(
            Report::parse(&read_state_text(&path, "immutable mutation report")?)? == report,
            "durable mutation report differs from the completed sweep"
        );
        Ok(report)
    }

    pub(super) fn latest_report(&self) -> Result<Option<(PathBuf, Report)>> {
        self.require_directory()?;
        let mut reports = Vec::new();
        for entry in std::fs::read_dir(&self.directory)
            .with_context(|| format!("reading {}", self.directory.display()))?
        {
            let entry = entry
                .with_context(|| format!("reading an entry in {}", self.directory.display()))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(sequence) = report_sequence(name)? else {
                continue;
            };
            let path = entry.path();
            require_direct_file(&self.directory, &path, "immutable mutation report")?;
            let report = Report::parse(&read_state_text(&path, "immutable mutation report")?)?;
            ensure!(
                report.sequence == sequence,
                "mutation report sequence disagrees with its file name"
            );
            reports.push((path, report));
        }
        Ok(reports
            .into_iter()
            .max_by_key(|(_, report)| report.sequence))
    }

    pub(super) fn write_last_run(&self, report: &Report) -> Result<()> {
        self.require_directory()?;
        let path = self.directory.join(LAST_RUN);
        let encoded = report.encode_with_checksum();
        self.replace_file(&path, encoded.as_bytes(), "last mutation report")
    }

    pub(super) fn clear(&self) -> Result<()> {
        self.require_directory()?;
        for name in JOURNAL_SLOTS {
            let path = self.directory.join(name);
            if require_direct_file_if_present(&self.directory, &path, "mutation journal slot")? {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing completed journal {}", path.display()))?;
            }
        }
        Ok(())
    }

    fn replace_file(&self, path: &Path, bytes: &[u8], label: &str) -> Result<()> {
        ensure!(
            path.parent() == Some(self.directory.as_path()),
            "{label} is not an exact mutation-state child"
        );
        let (temporary, mut file) = self.create_temporary_file(label)?;
        let result = (|| {
            file.write_all(bytes)
                .with_context(|| format!("writing {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("syncing {}", temporary.display()))?;
            drop(file);

            self.require_directory()?;
            if require_direct_file_if_present(&self.directory, path, label)? {
                std::fs::remove_file(path)
                    .with_context(|| format!("removing old {label} {}", path.display()))?;
            }
            std::fs::rename(&temporary, path).with_context(|| {
                format!(
                    "publishing {label} {} as {}",
                    temporary.display(),
                    path.display()
                )
            })?;
            require_direct_file(&self.directory, path, label)
        })();
        finish_staged_operation(&self.directory, &temporary, label, result)
    }

    fn create_temporary_file(&self, label: &str) -> Result<(PathBuf, File)> {
        self.require_directory()?;
        for _ in 0..64 {
            let nonce = NEXT_STORE_TEMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = self.directory.join(format!(
                ".mutation-write-{}-{nonce:016x}",
                std::process::id()
            ));
            let mut options = OpenOptions::new();
            OpenOptions::create_new(&mut options, true);
            OpenOptions::write(&mut options, true);
            match OpenOptions::open(&options, &path) {
                Ok(file) => {
                    require_direct_file(&self.directory, &path, label)?;
                    return Ok((path, file));
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(error)
                        .with_context(|| format!("creating temporary {label} {}", path.display()));
                }
            }
        }
        bail!("could not allocate a unique temporary {label}")
    }
}

pub(super) fn report_name(sequence: u64) -> String {
    format!("{REPORT_PREFIX}{sequence:020}{REPORT_SUFFIX}")
}

fn report_sequence(name: &str) -> Result<Option<u64>> {
    let Some(sequence) = name
        .strip_prefix(REPORT_PREFIX)
        .and_then(|name| name.strip_suffix(REPORT_SUFFIX))
    else {
        return Ok(None);
    };
    ensure!(
        sequence.len() == 20 && sequence.bytes().all(|byte| byte.is_ascii_digit()),
        "malformed mutation report file name `{name}`"
    );
    Ok(Some(
        <u64 as core::str::FromStr>::from_str(sequence)
            .context("parsing mutation report file sequence")?,
    ))
}

pub(super) fn is_digest(text: &str) -> bool {
    is_hex(text, 64)
}

pub(super) fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().fold(
        String::with_capacity(bytes.len() * 2),
        |mut encoded, byte| {
            encoded.push_str(&format!("{byte:02x}"));
            encoded
        },
    )
}

fn hex_decode(encoded: &str) -> Result<Vec<u8>> {
    ensure!(encoded.len().is_multiple_of(2), "hex value has half a byte");
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let high = hex_nibble(pair[0]).context("hex value has a non-hexadecimal byte")?;
            let low = hex_nibble(pair[1]).context("hex value has a non-hexadecimal byte")?;
            Ok(high << 4 | low)
        })
        .collect()
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
