mod kernel;
mod libs;
mod repository;
mod wost;
mod xtask_api;
mod xtask_build;
mod xtask_contracts;
mod xtask_gate;
mod xtask_mutation;
mod xtask_qemu;
mod xtask_qualification;
mod xtask_runners;
mod xtask_security_deep;

/// One deliberate defect, and what it is a defect *in*.
#[derive(Debug, Clone, Copy)]
pub struct Mutation {
    /// Permanent identity that documents cite; never renumbered or reused (D-341).
    pub id: u16,
    /// Workspace-relative path.
    pub file: &'static str,
    /// The exact text to replace. Must occur exactly once.
    pub find: &'static str,
    /// What to put there.
    pub replace: &'static str,
    /// The property this removes, in the register a check name is written in.
    ///
    /// Human-facing, and deliberately *not* the check's name: only four of
    /// these are, and the paraphrases read better in a progress line than the
    /// checks they stand for. [`Mutation::reddens`] carries the name.
    pub breaks: &'static str,
    /// The check this must turn red, or why nothing names it.
    pub reddens: Caught,
}

/// What a mutation must produce, and how precisely it is required.
#[derive(Debug, Clone, Copy)]
pub enum Caught {
    /// The exact `report.check("…")` name that must appear as `FAILED`.
    Check(&'static str),
    /// An exact host test whose own FAILED verdict must name the defect.
    HostTest {
        /// Cargo package containing the test.
        package: &'static str,
        /// Fully qualified test name accepted by `--exact`.
        name: &'static str,
    },
    /// The compiler-resolved API audit must name this exact dead item.
    ApiAudit(&'static str),
    /// A shipped degraded boot must emit one exact serial marker.
    BootMarker(&'static str),
    /// A degraded boot must diagnose its absent APIC before this exact panic.
    DegradedPanic {
        /// Exact text which must occur in the serial panic transcript.
        marker: &'static str,
        /// Why a terminal outcome is the defect's expected witness.
        why: &'static str,
    },
    /// The defect kills the machine before the verdict.
    Panic {
        /// Exact text which must occur in the serial panic line.
        marker: &'static str,
        /// Why a terminal outcome is the defect's expected witness.
        why: &'static str,
    },
    /// The defect prevents the machine from reaching any terminal verdict.
    Timeout {
        /// Exact observation deadline whose expiry is the witness.
        seconds: u64,
        /// Why this defect necessarily exhausts that deadline.
        why: &'static str,
    },
}

/// The identity the next registered mutation takes, wherever it is placed.
pub const NEXT_MUTATION_ID: u16 = 2456;

/// Identities of deleted mutations, which no later entry may take.
pub const RETIRED_MUTATION_IDS: &[u16] = &[
    116, 514, 515, 517, 1005, 1521, 1563, 1967, 1983, 1984, 1986, 1987, 1988, 2002, 2014, 2070,
    2096, 2097, 2098, 2099, 2100, 2104, 2109, 2112, 2115, 2116, 2117, 2120, 2121, 2123, 2124, 2192,
];

const SHARDS: &[&[Mutation]] = &[
    xtask_api::MUTATIONS,
    xtask_security_deep::MUTATIONS,
    xtask_qemu::MUTATIONS,
    xtask_qualification::MUTATIONS,
    xtask_contracts::MUTATIONS,
    xtask_mutation::MUTATIONS,
    xtask_runners::MUTATIONS,
    xtask_build::MUTATIONS,
    xtask_gate::MUTATIONS,
    kernel::MUTATIONS,
    wost::MUTATIONS,
    libs::MUTATIONS,
    repository::MUTATIONS,
];

/// The defects, each of which some check must notice.
///
/// Chosen to be *load-bearing rather than merely different*: every one removes
/// a guard, a bound or a strip that something depends on, and none is a
/// cosmetic edit that would fail a build instead. A mutation that cannot
/// compile tests nothing.
pub fn mutations() -> impl Iterator<Item = &'static Mutation> {
    SHARDS.iter().flat_map(|shard| shard.iter())
}
