//! Compiler-resolved evidence that every public foundation API has a caller.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use proc_macro2::{Delimiter, LineColumn, Span, TokenStream, TokenTree};
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Attribute, Expr, ForeignItem, ImplItem, Item, Lit, Meta, Token, TraitItem, UseTree, Visibility,
};

use crate::gate;
use crate::paths::Paths;

const MARKER: &str = "wizard-api-audit:";
const SOURCE_ROOTS: [&str; 2] = ["libs", "wost/src"];
const COPY_DIRECTORIES: [&str; 6] = [".cargo", "kernel", "libs", "tools", "wost", "xtask"];
const COPY_FILES: [&str; 3] = ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml"];
const RESERVED_LANGUAGE_ROOTS: [&str; 3] = ["std", "core", "alloc"];

#[derive(Debug, Clone)]
struct Node {
    id: usize,
    file: usize,
    label: String,
    symbol: String,
    type_declaration: bool,
    type_alias: bool,
    callable: bool,
    trait_default: bool,
    macro_declaration: bool,
    range: SourceRange,
    insertion: LineColumn,
    code: u64,
    isolation: Option<Isolation>,
    report: bool,
    instrument: bool,
    roots: RootSet,
    trait_route: Option<TraitRoute>,
    implementation_owner: Option<String>,
    direct_self_construction: bool,
    reviewed_drop_producer: Option<usize>,
    global_allocator: bool,
    global_allocator_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TraitRoute {
    Declaration(TraitMemberKey),
    Implementation {
        key: TraitMemberKey,
        self_type: Option<String>,
        exact_self_type: bool,
        external: bool,
        implicit: Option<ImplicitTrait>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImplicitTrait {
    Drop,
    GlobalAlloc,
}

const REVIEWED_IMPLICIT_DROP_TYPES: &[(&str, &str)] = &[
    ("kernel/src/handle.rs", "handle::HandleTable"),
    ("kernel/src/handle.rs", "handle::SlotReservation"),
    ("kernel/src/nvme.rs", "nvme::Controller"),
    ("kernel/src/object.rs", "object::Channel"),
    ("kernel/src/queue.rs", "queue::Queue"),
    ("kernel/src/user.rs", "user::ProcessConstruction"),
    (
        "wost/src/arch/x86_64/apic.rs",
        "arch::x86_64::apic::EoiToken",
    ),
    (
        "wost/src/arch/x86_64/apic.rs",
        "arch::x86_64::apic::Initialization",
    ),
    (
        "wost/src/arch/x86_64/cpu.rs",
        "arch::x86_64::cpu::WriteCombining",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        "arch::x86_64::paging::Activation",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        "arch::x86_64::paging::AddressSpace",
    ),
    (
        "wost/src/arch/x86_64/paging.rs",
        "arch::x86_64::paging::Unmapped",
    ),
    (
        "wost/src/arch/x86_64/usercopy.rs",
        "arch::x86_64::usercopy::Window",
    ),
    ("wost/src/mm/mod.rs", "mm::ExternalZeroRefusal"),
    ("wost/src/mm/mod.rs", "mm::Frame"),
    ("wost/src/mm/mod.rs", "mm::Segment"),
    ("wost/src/mm/mod.rs", "mm::USegment"),
    ("wost/src/sync.rs", "sync::ForgottenSpinLockGuard"),
    ("wost/src/sync.rs", "sync::SpinLockGuard"),
    ("wost/src/sync.rs", "sync::ThreadLockGuard"),
    ("xtask/src/bounded_process.rs", "bounded_process::Capture"),
    (
        "xtask/src/bounded_process.rs",
        "bounded_process::CaptureReservation",
    ),
    ("xtask/src/contracts.rs", "contracts::Probe"),
    ("xtask/src/image/mod.rs", "image::RunImage"),
    (
        "xtask/src/mutation_workspace.rs",
        "mutation_workspace::tests::TempTree",
    ),
    ("xtask/src/qemu.rs", "qemu::DebugMachine"),
    ("xtask/src/qemu.rs", "qemu::RunVars"),
    ("xtask/src/qemu.rs", "qemu::tests::QemuFixture"),
];

struct ReviewedDropProducer {
    relative: &'static str,
    owner: &'static str,
    member: &'static str,
    implemented_trait: Option<&'static str>,
    produced: &'static str,
}

const REVIEWED_IMPLICIT_DROP_PRODUCERS: &[ReviewedDropProducer] = &[
    ReviewedDropProducer {
        relative: "kernel/src/handle.rs",
        owner: "handle::HandleTable",
        member: "new",
        implemented_trait: None,
        produced: "handle::HandleTable",
    },
    ReviewedDropProducer {
        relative: "kernel/src/queue.rs",
        owner: "queue::Queue",
        member: "new",
        implemented_trait: None,
        produced: "queue::Queue",
    },
    ReviewedDropProducer {
        relative: "kernel/src/user.rs",
        owner: "user::ProcessConstruction",
        member: "new",
        implemented_trait: None,
        produced: "user::ProcessConstruction",
    },
    ReviewedDropProducer {
        relative: "wost/src/arch/x86_64/apic.rs",
        owner: "arch::x86_64::apic::Initialization",
        member: "begin",
        implemented_trait: None,
        produced: "arch::x86_64::apic::Initialization",
    },
    ReviewedDropProducer {
        relative: "wost/src/arch/x86_64/paging.rs",
        owner: "arch::x86_64::paging::Activation",
        member: "begin_with",
        implemented_trait: None,
        produced: "arch::x86_64::paging::Activation",
    },
    ReviewedDropProducer {
        relative: "wost/src/arch/x86_64/paging.rs",
        owner: "arch::x86_64::paging::AddressSpace",
        member: "new",
        implemented_trait: None,
        produced: "arch::x86_64::paging::AddressSpace",
    },
    ReviewedDropProducer {
        relative: "wost/src/arch/x86_64/usercopy.rs",
        owner: "arch::x86_64::usercopy::Window",
        member: "open",
        implemented_trait: None,
        produced: "arch::x86_64::usercopy::Window",
    },
    ReviewedDropProducer {
        relative: "wost/src/mm/mod.rs",
        owner: "mm::ExternalZeroRefusal",
        member: "arm",
        implemented_trait: None,
        produced: "mm::ExternalZeroRefusal",
    },
    ReviewedDropProducer {
        relative: "wost/src/mm/mod.rs",
        owner: "mm::Frame",
        member: "from_raw",
        implemented_trait: None,
        produced: "mm::Frame",
    },
    ReviewedDropProducer {
        relative: "wost/src/mm/mod.rs",
        owner: "mm::Segment",
        member: "from_raw",
        implemented_trait: None,
        produced: "mm::Segment",
    },
    ReviewedDropProducer {
        relative: "wost/src/mm/mod.rs",
        owner: "mm::USegment",
        member: "from",
        implemented_trait: Some("From"),
        produced: "mm::USegment",
    },
    ReviewedDropProducer {
        relative: "xtask/src/bounded_process.rs",
        owner: "bounded_process::Capture",
        member: "create_in",
        implemented_trait: None,
        produced: "bounded_process::Capture",
    },
    ReviewedDropProducer {
        relative: "xtask/src/bounded_process.rs",
        owner: "bounded_process::CaptureReservation",
        member: "new",
        implemented_trait: None,
        produced: "bounded_process::CaptureReservation",
    },
    ReviewedDropProducer {
        relative: "xtask/src/contracts.rs",
        owner: "contracts::Probe",
        member: "create",
        implemented_trait: None,
        produced: "contracts::Probe",
    },
    ReviewedDropProducer {
        relative: "xtask/src/image/mod.rs",
        owner: "image::RunImage",
        member: "reserve",
        implemented_trait: None,
        produced: "image::RunImage",
    },
    ReviewedDropProducer {
        relative: "xtask/src/mutation_workspace.rs",
        owner: "mutation_workspace::tests::TempTree",
        member: "new",
        implemented_trait: None,
        produced: "mutation_workspace::tests::TempTree",
    },
    ReviewedDropProducer {
        relative: "xtask/src/qemu.rs",
        owner: "qemu::DebugMachine",
        member: "start",
        implemented_trait: None,
        produced: "qemu::DebugMachine",
    },
    ReviewedDropProducer {
        relative: "xtask/src/qemu.rs",
        owner: "qemu::RunVars",
        member: "create_with",
        implemented_trait: None,
        produced: "qemu::RunVars",
    },
    ReviewedDropProducer {
        relative: "xtask/src/qemu.rs",
        owner: "qemu::tests::QemuFixture",
        member: "create",
        implemented_trait: None,
        produced: "qemu::tests::QemuFixture",
    },
];

const REVIEWED_DEPENDENCY_BUILD_SCRIPTS: &[(&str, &str, &str)] = &[
    (
        "anyhow",
        "1.0.104",
        "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470",
    ),
    (
        "corosensei",
        "0.3.4",
        "6886a0c0f263965933c438626e7179139a62b978a33aa18281cbf0cd5a975f34",
    ),
    (
        "getrandom",
        "0.3.1",
        "43a49c392881ce6d5c3b8cb70f98717b7c07aabbdff06687b9030dbfbe2725f8",
    ),
    (
        "num-traits",
        "0.2.19",
        "071dfc062690e90b734c0b2273ce72ad0ffa95f0c74596bc250dcfd960262841",
    ),
    (
        "owo-colors",
        "4.3.0",
        "d211803b9b6b570f68772237e415a029d5a50c65d382910b879fb19d3271f94d",
    ),
    (
        "proc-macro2",
        "1.0.107",
        "985e7ec9bb745e6ce6535b544d84d6cd6f7ad8bd711c398938ae983b91a766d9",
    ),
    (
        "quote",
        "1.0.47",
        "1fbf4db142a473a8d80c26bbf18454ed458bf8d26c8219c331daecfdbd079001",
    ),
    (
        "radium",
        "0.7.0",
        "dc33ff2d4973d518d823d61aa239014831e521c75da58e3df4840d3f47749d09",
    ),
    (
        "windows_x86_64_gnu",
        "0.52.6",
        "147a5c80aabfbf0c7d901cb5895d1de30ef2907eb21fbbab29ca94c5b08b1a78",
    ),
    (
        "zerocopy",
        "0.8.56",
        "556764e583adb45a9f8d413c2a147fa7e8d821e48e12b14fd560b607998b75eb",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct TraitMemberKey {
    trait_path: String,
    member: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct RootSet {
    production: bool,
    verification: bool,
}

impl RootSet {
    fn contains(self, kind: RootKind) -> bool {
        match kind {
            RootKind::Production => self.production,
            RootKind::Verification => self.production || self.verification,
            RootKind::TestHarness => self.verification,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Isolation {
    Variant,
    TraitMember,
}

const ISOLATIONS: [Isolation; 2] = [Isolation::Variant, Isolation::TraitMember];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceRange {
    start: LineColumn,
    end: LineColumn,
}

impl SourceRange {
    fn contains(self, at: LineColumn) -> bool {
        (at.line > self.start.line || at.line == self.start.line && at.column >= self.start.column)
            && (at.line < self.end.line || at.line == self.end.line && at.column < self.end.column)
    }

    fn extent(self) -> (usize, usize) {
        (
            self.end.line.saturating_sub(self.start.line),
            self.end.column.saturating_sub(self.start.column),
        )
    }
}

#[derive(Debug, Clone)]
struct SourceFile {
    relative: PathBuf,
    crate_name: String,
    origin: CargoTarget,
    text: String,
    reexports: Vec<SourceRange>,
    non_call_uses: Vec<SourceRange>,
    macro_syntax: Vec<SourceRange>,
    transparent_macros: Vec<SourceRange>,
    macro_paths: Vec<SourceRange>,
    dispatch_sites: Vec<DispatchSite>,
    value_constructions: Vec<ValueConstruction>,
}

#[derive(Debug, Clone, Copy)]
struct EntrySpec {
    file: usize,
    invocation: SourceRange,
    macro_path: SourceRange,
    argument: SourceRange,
}

#[derive(Debug, Clone, Copy)]
struct EntryContractUse {
    symbol: &'static str,
    primary: SourceRange,
}

#[derive(Debug, Clone, Copy)]
struct EntryContractAuthority {
    uses: [EntryContractUse; 4],
}

#[derive(Debug, Clone, Copy)]
struct EntryContractNode {
    id: usize,
    primary: SourceRange,
}

#[derive(Debug, Clone, Copy)]
struct EntryLink {
    file: usize,
    invocation: SourceRange,
    macro_path: SourceRange,
    argument: SourceRange,
    invocation_node: usize,
    function_node: usize,
    macro_declaration: usize,
    contract_nodes: [EntryContractNode; 4],
}

#[derive(Debug, Clone)]
struct DispatchSite {
    range: SourceRange,
    callee_evidence: Vec<SourceRange>,
    self_evidence: Vec<SourceRange>,
    exact_self_syntax: bool,
    exact_callee_self_syntax: bool,
    explicit_qself: bool,
    generic_qself_trait: Option<String>,
    kind: DispatchKind,
}

impl DispatchSite {
    fn callee_names(&self, callee_nodes: &BTreeSet<usize>, self_type: usize) -> bool {
        self.exact_callee_self_syntax && !self.explicit_qself && callee_nodes.contains(&self_type)
    }
}

#[derive(Debug, Clone)]
struct ValueConstruction {
    range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DispatchKind {
    Call {
        member: String,
        trait_qualifier: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ModuleContext {
    scope: Vec<String>,
    availability: Availability,
    package: String,
    crate_name: String,
    target_root: PathBuf,
    target_kind: CargoTargetKind,
    language_main: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CargoTargetKind {
    Library,
    Bin,
    Example,
    Test,
    Bench,
    CustomBuild,
}

impl CargoTargetKind {
    fn parse(kinds: &[&str]) -> Result<Self> {
        let [kind] = kinds else {
            bail!(
                "Cargo metadata target has ambiguous kinds: {}",
                kinds.join(", ")
            );
        };
        match *kind {
            "lib" => Ok(Self::Library),
            "bin" => Ok(Self::Bin),
            "example" => Ok(Self::Example),
            "test" => Ok(Self::Test),
            "bench" => Ok(Self::Bench),
            "custom-build" => Ok(Self::CustomBuild),
            _ => bail!("Cargo metadata target has unsupported kind `{kind}`"),
        }
    }

    fn metadata_name(self) -> &'static str {
        match self {
            Self::Library => "lib",
            Self::Bin => "bin",
            Self::Example => "example",
            Self::Test => "test",
            Self::Bench => "bench",
            Self::CustomBuild => "custom-build",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CargoTarget {
    package: String,
    package_id: String,
    name: String,
    relative: PathBuf,
    kind: CargoTargetKind,
    test: bool,
    executable: bool,
}

impl CargoTarget {
    fn context(&self) -> ModuleContext {
        let availability = match self.kind {
            CargoTargetKind::Test | CargoTargetKind::Bench => Availability::Verification,
            _ => Availability::Production,
        };
        let language_main = match self.kind {
            CargoTargetKind::Bin | CargoTargetKind::CustomBuild => true,
            CargoTargetKind::Example => self.executable,
            CargoTargetKind::Library | CargoTargetKind::Test | CargoTargetKind::Bench => false,
        };
        ModuleContext {
            scope: Vec::new(),
            availability,
            package: self.package.clone(),
            crate_name: self.name.replace('-', "_"),
            target_root: self.relative.clone(),
            target_kind: self.kind,
            language_main,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedDependency {
    name: String,
    package_id: String,
    normal: bool,
    build: bool,
    dev: bool,
}

#[derive(Debug)]
struct DependencyBuildScript {
    name: String,
    version: String,
    registry: bool,
}

#[derive(Debug)]
struct CargoCatalog {
    packages: BTreeMap<String, Vec<CargoTarget>>,
    resolved: BTreeMap<String, Vec<ResolvedDependency>>,
}

impl CargoCatalog {
    fn discover(
        workspace: &AuditWorkspace,
        expected: &BTreeMap<PathBuf, [u8; 32]>,
        pass: gate::CargoPass,
        compiler: &CompilerIdentity,
    ) -> Result<Self> {
        let manifest = workspace.root.join("Cargo.toml");
        let mut command = audited_cargo_command(
            &workspace.execution_root,
            &workspace.cargo_home,
            &workspace.registry_source,
            compiler,
        )?;
        command
            .env("CARGO_BUILD_WARNINGS", "warn")
            .env(
                "CARGO_ENCODED_RUSTFLAGS",
                cargo_encoded_rustflags(pass.target)?,
            )
            .args([
                "metadata",
                "--format-version=1",
                "--offline",
                "--locked",
                "--color=never",
                "--manifest-path",
            ])
            .arg(&manifest)
            .args([
                "--filter-platform",
                pass.target.unwrap_or(&compiler.host_target),
            ]);
        if let Some(features) = pass.features {
            command.args(["--features", features]);
        }
        let result = command
            .output()
            .context("spawning Cargo metadata for the API audit")?;
        workspace.require_copy_unchanged(expected)?;
        workspace.require_source_unchanged(&workspace.source_root)?;
        workspace.require_cargo_inputs_unchanged()?;
        compiler.require_unchanged()?;
        ensure!(
            result.status.success(),
            "Cargo metadata for the API audit failed ({}):\n{}",
            result.status,
            String::from_utf8_lossy(&result.stderr)
        );
        Self::parse(
            &workspace.root,
            &workspace.registry_source,
            &result.stdout,
            pass.packages,
            pass.target.is_none(),
        )
    }

    fn parse(
        root: &Path,
        registry_source: &Path,
        bytes: &[u8],
        selected_packages: &[&str],
        include_root_dev_dependencies: bool,
    ) -> Result<Self> {
        reject_custom_harness_manifests(root)?;
        let value = crate::json::parse_json(bytes, "Cargo metadata JSON")?;
        let document = crate::json::json_object(&value, "Cargo metadata JSON")?;
        ensure!(
            crate::json::json_u64(
                crate::json::json_field(document, "version", "Cargo metadata JSON")?,
                "Cargo metadata `version`"
            )? == 1,
            "Cargo metadata has an unsupported format version"
        );

        let declared_root = PathBuf::from(crate::json::json_string(
            crate::json::json_field(document, "workspace_root", "Cargo metadata JSON")?,
            "Cargo metadata `workspace_root`",
        )?);
        let canonical_root = std::fs::canonicalize(root)
            .with_context(|| format!("canonicalizing API audit root {}", root.display()))?;
        ensure!(
            std::fs::canonicalize(&declared_root).with_context(|| format!(
                "canonicalizing Cargo metadata workspace root {}",
                declared_root.display()
            ))? == canonical_root,
            "Cargo metadata describes a different workspace root"
        );

        let members = crate::json::json_string_array(
            crate::json::json_field(document, "workspace_members", "Cargo metadata JSON")?,
            "Cargo metadata `workspace_members`",
        )?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let package_values = crate::json::json_array(
            crate::json::json_field(document, "packages", "Cargo metadata JSON")?,
            "Cargo metadata `packages`",
        )?;
        let mut package_names = BTreeMap::new();
        let mut procedural_macros = BTreeMap::new();
        let mut dependency_build_scripts = BTreeMap::new();
        for package_value in package_values {
            let package = crate::json::json_object(package_value, "Cargo metadata package")?;
            let id = crate::json::json_string(
                crate::json::json_field(package, "id", "Cargo metadata package")?,
                "Cargo metadata package `id`",
            )?;
            let name = crate::json::json_string(
                crate::json::json_field(package, "name", "Cargo metadata package")?,
                "Cargo metadata package `name`",
            )?;
            ensure!(
                package_names
                    .insert(id.to_string(), name.to_string())
                    .is_none(),
                "Cargo metadata contains duplicate package ID `{id}`"
            );
            let targets = crate::json::json_array(
                crate::json::json_field(package, "targets", "Cargo metadata package")?,
                "Cargo metadata package `targets`",
            )?;
            let mut custom_build = false;
            for target in targets {
                let target = crate::json::json_object(target, "Cargo metadata target")?;
                let kinds = crate::json::json_string_array(
                    crate::json::json_field(target, "kind", "Cargo metadata target")?,
                    "Cargo metadata target `kind`",
                )?;
                if kinds.contains(&"proc-macro") {
                    procedural_macros.insert(id.to_string(), name.to_string());
                }
                custom_build |= kinds.contains(&"custom-build");
            }
            if custom_build && !members.contains(id) {
                let version = crate::json::json_string(
                    crate::json::json_field(package, "version", "Cargo metadata package")?,
                    "Cargo metadata package `version`",
                )?;
                let registry = matches!(
                    crate::json::json_field(package, "source", "Cargo metadata package")?,
                    crate::json::JsonValue::String(source)
                        if source == "registry+https://github.com/rust-lang/crates.io-index"
                );
                dependency_build_scripts.insert(
                    id.to_string(),
                    DependencyBuildScript {
                        name: name.to_string(),
                        version: version.to_string(),
                        registry,
                    },
                );
            }
        }

        let resolved = parse_resolved_dependencies(document, &package_names)?;
        let workspace_ids = members
            .iter()
            .map(|id| {
                package_names
                    .get(*id)
                    .map(|name| (name.clone(), (*id).to_string()))
                    .with_context(|| format!("workspace member `{id}` has no package definition"))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let root_ids = selected_packages
            .iter()
            .map(|name| {
                workspace_ids.get(*name).cloned().with_context(|| {
                    format!("registered Cargo package `{name}` is absent from metadata")
                })
            })
            .collect::<Result<BTreeSet<_>>>()?;
        let active = active_resolved_packages(&resolved, &root_ids, include_root_dev_dependencies)?;
        reject_reserved_dependency_roots(
            &resolved,
            &active,
            &root_ids,
            include_root_dev_dependencies,
            &package_names,
        )?;
        if let Some((_, name)) = procedural_macros
            .iter()
            .find(|(id, _)| active.contains(*id))
        {
            bail!(
                "active procedural-macro package `{name}` is outside the API audit's syntactic trust boundary"
            );
        }
        validate_dependency_build_scripts(root, &active, &dependency_build_scripts)?;
        validate_active_external_package_roots(registry_source, package_values, &members, &active)?;

        let mut packages = BTreeMap::new();
        for package_value in package_values {
            let package = crate::json::json_object(package_value, "Cargo metadata package")?;
            let id = crate::json::json_string(
                crate::json::json_field(package, "id", "Cargo metadata package")?,
                "Cargo metadata package `id`",
            )?;
            let name = crate::json::json_string(
                crate::json::json_field(package, "name", "Cargo metadata package")?,
                "Cargo metadata package `name`",
            )?;
            let target_values = crate::json::json_array(
                crate::json::json_field(package, "targets", "Cargo metadata package")?,
                "Cargo metadata package `targets`",
            )?;
            if !members.contains(id) {
                continue;
            }
            let mut targets = Vec::new();
            for target_value in target_values {
                let target = parse_cargo_target(&canonical_root, name, id, target_value)?;
                if active.contains(id) && target.kind == CargoTargetKind::CustomBuild {
                    validate_workspace_build_script(&canonical_root, name, &target)?;
                }
                targets.push(target);
            }
            ensure!(
                !targets.is_empty(),
                "Cargo workspace package `{name}` has no target"
            );
            ensure!(
                packages.insert(name.to_string(), targets).is_none(),
                "Cargo workspace contains duplicate package name `{name}`"
            );
        }
        ensure!(
            packages.len() == members.len(),
            "Cargo metadata omitted one or more workspace members"
        );
        Ok(Self { packages, resolved })
    }

    fn targets(&self) -> impl Iterator<Item = &CargoTarget> {
        self.packages.values().flatten()
    }

    fn package(&self, name: &str) -> Result<&[CargoTarget]> {
        self.packages
            .get(name)
            .map(Vec::as_slice)
            .with_context(|| format!("registered Cargo package `{name}` is absent from metadata"))
    }

    fn has_direct_normal_dependency(
        &self,
        package: &str,
        dependency_name: &str,
        dependency_package: &str,
    ) -> bool {
        let Some(package_id) = self
            .packages
            .get(package)
            .and_then(|targets| targets.first())
            .map(|target| target.package_id.as_str())
        else {
            return false;
        };
        let Some(dependency_id) = self
            .packages
            .get(dependency_package)
            .and_then(|targets| targets.first())
            .map(|target| target.package_id.as_str())
        else {
            return false;
        };
        self.resolved
            .get(package_id)
            .into_iter()
            .flatten()
            .any(|dependency| {
                dependency.normal
                    && dependency.package_id == dependency_id
                    && dependency.name == dependency_name
            })
    }
}

fn validate_active_external_package_roots(
    registry_source: &Path,
    packages: &[crate::json::JsonValue],
    members: &BTreeSet<&str>,
    active: &BTreeSet<String>,
) -> Result<()> {
    for package in packages {
        let package = crate::json::json_object(package, "Cargo metadata package")?;
        let id = crate::json::json_string(
            crate::json::json_field(package, "id", "Cargo metadata package")?,
            "Cargo metadata package `id`",
        )?;
        if !active.contains(id) || members.contains(id) {
            continue;
        }
        let name = crate::json::json_string(
            crate::json::json_field(package, "name", "Cargo metadata package")?,
            "Cargo metadata package `name`",
        )?;
        let version = crate::json::json_string(
            crate::json::json_field(package, "version", "Cargo metadata package")?,
            "Cargo metadata package `version`",
        )?;
        for (label, value) in [("name", name), ("version", version)] {
            ensure!(
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+')
                    }),
                "active registry package has unsafe {label} `{value}`"
            );
        }
        ensure!(
            matches!(
                crate::json::json_field(package, "source", "Cargo metadata package")?,
                crate::json::JsonValue::String(source)
                    if source == "registry+https://github.com/rust-lang/crates.io-index"
            ),
            "active non-workspace package {name} {version} is not a checksum-bound crates.io package"
        );

        let registry_source = crate::mutation_workspace::canonical_existing_real_directory(
            registry_source,
            "API audit materialized registry source",
        )?;
        let package_root = registry_source.join(format!("{name}-{version}"));
        let package_root = crate::mutation_workspace::canonical_existing_real_directory(
            &package_root,
            "API audit materialized registry package",
        )?;
        ensure!(
            package_root.starts_with(&registry_source),
            "active registry package {name} {version} escaped the materialized registry source"
        );
        let manifest = PathBuf::from(crate::json::json_string(
            crate::json::json_field(package, "manifest_path", "Cargo metadata package")?,
            "Cargo metadata package `manifest_path`",
        )?);
        let manifest = std::fs::canonicalize(&manifest).with_context(|| {
            format!(
                "canonicalizing Cargo package manifest {}",
                manifest.display()
            )
        })?;
        ensure!(
            manifest == package_root.join("Cargo.toml"),
            "active package {name} {version} is outside the materialized registry source"
        );
        crate::mutation_workspace::require_direct_file(
            &package_root,
            &manifest,
            "API audit materialized registry manifest",
        )?;

        let targets = crate::json::json_array(
            crate::json::json_field(package, "targets", "Cargo metadata package")?,
            "Cargo metadata package `targets`",
        )?;
        ensure!(
            !targets.is_empty(),
            "active registry package {name} {version} has no compiler target"
        );
        for target in targets {
            let target = crate::json::json_object(target, "Cargo metadata target")?;
            let source = PathBuf::from(crate::json::json_string(
                crate::json::json_field(target, "src_path", "Cargo metadata target")?,
                "Cargo metadata target `src_path`",
            )?);
            let source = std::fs::canonicalize(&source).with_context(|| {
                format!(
                    "canonicalizing Cargo dependency target {}",
                    source.display()
                )
            })?;
            ensure!(
                source.starts_with(&package_root),
                "active package {name} {version} has a compiler target outside the materialized registry source"
            );
            let parent = source
                .parent()
                .context("Cargo dependency target has no parent directory")?;
            crate::mutation_workspace::require_direct_file(
                parent,
                &source,
                "API audit registry target source",
            )?;
            ensure!(
                source
                    .extension()
                    .is_some_and(|extension| extension == "rs"),
                "active registry package {name} {version} has a non-Rust compiler target"
            );
        }
    }

    Ok(())
}

fn reject_custom_harness_manifests(root: &Path) -> Result<()> {
    let mut manifests = Vec::new();
    let workspace = root.join("Cargo.toml");
    if workspace.is_file() {
        manifests.push(workspace);
    }
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        if directory.is_dir() {
            collect_named_files(&directory, "Cargo.toml", &mut manifests)?;
        }
    }
    for manifest in manifests {
        let text = std::fs::read_to_string(&manifest)
            .with_context(|| format!("reading {}", manifest.display()))?;
        ensure!(
            !text.contains("harness") && !text.contains("\\u") && !text.contains("\\U"),
            "{} contains a custom-harness or encoded TOML key outside the API audit's target model",
            manifest.display()
        );
    }
    Ok(())
}

fn collect_named_files(directory: &Path, name: &str, files: &mut Vec<PathBuf>) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(
        directory,
        "API audit manifest directory",
    )?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            collect_named_files(&entry.path(), name, files)?;
        } else if kind.is_file() && entry.file_name() == name {
            files.push(entry.path());
        } else if !kind.is_file() {
            bail!(
                "API audit manifest tree {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn parse_resolved_dependencies(
    document: &BTreeMap<String, crate::json::JsonValue>,
    package_names: &BTreeMap<String, String>,
) -> Result<BTreeMap<String, Vec<ResolvedDependency>>> {
    let resolution = crate::json::json_object(
        crate::json::json_field(document, "resolve", "Cargo metadata JSON")?,
        "Cargo metadata `resolve`",
    )?;
    let nodes = crate::json::json_array(
        crate::json::json_field(resolution, "nodes", "Cargo metadata resolution")?,
        "Cargo metadata resolution `nodes`",
    )?;
    let mut resolved = BTreeMap::new();
    for node in nodes {
        let node = crate::json::json_object(node, "Cargo metadata resolved node")?;
        let id = crate::json::json_string(
            crate::json::json_field(node, "id", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `id`",
        )?;
        ensure!(
            package_names.contains_key(id),
            "Cargo resolution names unknown package ID `{id}`"
        );
        let declared = crate::json::json_string_array(
            crate::json::json_field(node, "dependencies", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `dependencies`",
        )?
        .into_iter()
        .collect::<BTreeSet<_>>();
        let dependencies = crate::json::json_array(
            crate::json::json_field(node, "deps", "Cargo metadata resolved node")?,
            "Cargo metadata resolved node `deps`",
        )?;
        let mut parsed = Vec::new();
        let mut parsed_ids = BTreeSet::new();
        for dependency in dependencies {
            let dependency =
                crate::json::json_object(dependency, "Cargo metadata resolved dependency")?;
            let name = crate::json::json_string(
                crate::json::json_field(dependency, "name", "Cargo metadata resolved dependency")?,
                "Cargo metadata resolved dependency `name`",
            )?;
            let package_id = crate::json::json_string(
                crate::json::json_field(dependency, "pkg", "Cargo metadata resolved dependency")?,
                "Cargo metadata resolved dependency `pkg`",
            )?;
            ensure!(
                package_names.contains_key(package_id),
                "Cargo resolution dependency names unknown package ID `{package_id}`"
            );
            parsed_ids.insert(package_id);
            let kinds = crate::json::json_array(
                crate::json::json_field(
                    dependency,
                    "dep_kinds",
                    "Cargo metadata resolved dependency",
                )?,
                "Cargo metadata resolved dependency `dep_kinds`",
            )?;
            let mut normal = false;
            let mut build = false;
            let mut dev = false;
            for kind in kinds {
                let kind =
                    crate::json::json_object(kind, "Cargo metadata resolved dependency kind")?;
                match crate::json::json_field(
                    kind,
                    "kind",
                    "Cargo metadata resolved dependency kind",
                )? {
                    crate::json::JsonValue::Null => normal = true,
                    crate::json::JsonValue::String(kind) if kind == "build" => build = true,
                    crate::json::JsonValue::String(kind) if kind == "dev" => dev = true,
                    crate::json::JsonValue::String(kind) => {
                        bail!("Cargo metadata has unsupported dependency kind `{kind}`")
                    }
                    _ => bail!("Cargo metadata dependency kind is not null or a string"),
                }
            }
            ensure!(
                normal || build || dev,
                "Cargo metadata dependency `{name}` has no dependency kind"
            );
            parsed.push(ResolvedDependency {
                name: name.to_string(),
                package_id: package_id.to_string(),
                normal,
                build,
                dev,
            });
        }
        ensure!(
            declared == parsed_ids,
            "Cargo metadata dependency identities disagree for package `{id}`"
        );
        ensure!(
            resolved.insert(id.to_string(), parsed).is_none(),
            "Cargo metadata contains duplicate resolved node `{id}`"
        );
    }
    Ok(resolved)
}

fn active_resolved_packages(
    resolved: &BTreeMap<String, Vec<ResolvedDependency>>,
    root_ids: &BTreeSet<String>,
    include_root_dev_dependencies: bool,
) -> Result<BTreeSet<String>> {
    let mut active = BTreeSet::new();
    let mut pending = VecDeque::new();
    for root in root_ids {
        ensure!(
            resolved.contains_key(root),
            "registered Cargo root `{root}` has no resolved dependency node"
        );
        active.insert(root.clone());
        pending.push_back(root.clone());
    }
    while let Some(package) = pending.pop_front() {
        for dependency in resolved
            .get(&package)
            .context("an active Cargo package lost its dependency node")?
        {
            let enabled = dependency.normal
                || dependency.build
                || include_root_dev_dependencies && root_ids.contains(&package) && dependency.dev;
            if enabled && active.insert(dependency.package_id.clone()) {
                pending.push_back(dependency.package_id.clone());
            }
        }
    }
    Ok(active)
}

fn reject_reserved_dependency_roots(
    resolved: &BTreeMap<String, Vec<ResolvedDependency>>,
    active: &BTreeSet<String>,
    root_ids: &BTreeSet<String>,
    include_root_dev_dependencies: bool,
    package_names: &BTreeMap<String, String>,
) -> Result<()> {
    for package in active {
        for dependency in resolved
            .get(package)
            .context("an active Cargo package lost its dependency node")?
        {
            let enabled = dependency.normal
                || dependency.build
                || include_root_dev_dependencies && root_ids.contains(package) && dependency.dev;
            if enabled && RESERVED_LANGUAGE_ROOTS.contains(&dependency.name.as_str()) {
                let owner = package_names
                    .get(package)
                    .context("an active Cargo package has no package name")?;
                bail!(
                    "active Cargo package `{owner}` exposes a dependency under reserved language root `{}`",
                    dependency.name
                );
            }
        }
    }
    Ok(())
}

fn validate_dependency_build_scripts(
    root: &Path,
    active: &BTreeSet<String>,
    scripts: &BTreeMap<String, DependencyBuildScript>,
) -> Result<()> {
    let active_scripts = scripts
        .iter()
        .filter(|(id, _)| active.contains(*id))
        .map(|(_, script)| script)
        .collect::<Vec<_>>();
    if active_scripts.is_empty() {
        return Ok(());
    }
    let locked = locked_registry_packages(&root.join("Cargo.lock"))?;
    for script in active_scripts {
        ensure!(
            script.registry,
            "active dependency build script {} {} is not a checksum-bound crates.io package",
            script.name,
            script.version
        );
        let checksums = locked
            .iter()
            .filter(|package| package.name == script.name && package.version == script.version)
            .map(|package| package.checksum.as_str())
            .collect::<Vec<_>>();
        let [checksum] = checksums.as_slice() else {
            bail!(
                "active dependency build script {} {} has {} Cargo.lock identities",
                script.name,
                script.version,
                checksums.len()
            );
        };
        ensure!(
            REVIEWED_DEPENDENCY_BUILD_SCRIPTS
                .iter()
                .any(|(name, version, reviewed_checksum)| {
                    script.name == *name
                        && script.version == *version
                        && checksum == reviewed_checksum
                }),
            "active dependency build script {} {} is outside the API audit's checksum-bound reviewed set",
            script.name,
            script.version
        );
    }
    Ok(())
}

fn parse_cargo_target(
    canonical_root: &Path,
    package: &str,
    package_id: &str,
    value: &crate::json::JsonValue,
) -> Result<CargoTarget> {
    let target = crate::json::json_object(value, "Cargo metadata target")?;
    let name = crate::json::json_string(
        crate::json::json_field(target, "name", "Cargo metadata target")?,
        "Cargo metadata target `name`",
    )?;
    let kinds = crate::json::json_string_array(
        crate::json::json_field(target, "kind", "Cargo metadata target")?,
        "Cargo metadata target `kind`",
    )?;
    let kind = CargoTargetKind::parse(&kinds)
        .with_context(|| format!("classifying target `{package}:{name}`"))?;
    let crate_types = crate::json::json_string_array(
        crate::json::json_field(target, "crate_types", "Cargo metadata target")?,
        "Cargo metadata target `crate_types`",
    )?;
    let executable = crate_types.contains(&"bin");
    let test = match crate::json::json_field(target, "test", "Cargo metadata target")? {
        crate::json::JsonValue::Bool(test) => *test,
        _ => bail!("Cargo metadata target `test` is not a Boolean"),
    };
    if let Some(required) = target.get("required-features") {
        let required =
            crate::json::json_string_array(required, "Cargo metadata target `required-features`")?;
        ensure!(
            required.is_empty(),
            "Cargo target `{package}:{name}` uses required-features, which the API audit does not yet model"
        );
    }
    let source = PathBuf::from(crate::json::json_string(
        crate::json::json_field(target, "src_path", "Cargo metadata target")?,
        "Cargo metadata target `src_path`",
    )?);
    let canonical_source = std::fs::canonicalize(&source)
        .with_context(|| format!("canonicalizing Cargo target source {}", source.display()))?;
    let relative = canonical_source
        .strip_prefix(canonical_root)
        .with_context(|| {
            format!(
                "Cargo target source {} is outside the API audit copy",
                source.display()
            )
        })?
        .to_path_buf();
    ensure!(
        relative
            .extension()
            .is_some_and(|extension| extension == "rs"),
        "Cargo target source {} is not Rust source",
        relative.display()
    );
    Ok(CargoTarget {
        package: package.to_string(),
        package_id: package_id.to_string(),
        name: name.to_string(),
        relative,
        kind,
        test,
        executable,
    })
}

fn validate_workspace_build_script(root: &Path, package: &str, target: &CargoTarget) -> Result<()> {
    ensure!(
        package == crate::paths::KERNEL_PACKAGE && target.relative == Path::new("kernel/build.rs"),
        "active workspace build script `{package}:{}` is outside the API audit's exact build-script contract",
        target.relative.display()
    );
    let path = root.join(&target.relative);
    let source = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "reading API audit build script {}",
            target.relative.display()
        )
    })?;
    let syntax = syn::parse_file(&source).with_context(|| {
        format!(
            "parsing API audit build script {}",
            target.relative.display()
        )
    })?;
    ensure!(
        syntax.shebang.is_none()
            && syntax
                .attrs
                .iter()
                .all(|attribute| attribute.path().is_ident("doc")),
        "kernel build script has crate syntax outside its exact linker contract"
    );
    let [Item::Fn(main)] = syntax.items.as_slice() else {
        bail!("kernel build script must contain only its exact `main` function");
    };
    ensure!(
        main.attrs
            .iter()
            .all(|attribute| attribute.path().is_ident("doc"))
            && matches!(main.vis, Visibility::Inherited)
            && main.sig.ident == "main"
            && main.sig.constness.is_none()
            && main.sig.asyncness.is_none()
            && main.sig.unsafety.is_none()
            && main.sig.abi.is_none()
            && main.sig.generics.params.is_empty()
            && main.sig.generics.where_clause.is_none()
            && main.sig.inputs.is_empty()
            && main.sig.variadic.is_none()
            && matches!(main.sig.output, syn::ReturnType::Default),
        "kernel build script `main` has syntax outside its exact linker contract"
    );
    let actual = source_tokens(&source, main.block.span())?;
    let expected = <TokenStream as core::str::FromStr>::from_str(
        r#"{
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        println!("cargo:rustc-link-arg=-T{manifest_dir}/linker.ld");
        println!("cargo:rerun-if-changed=linker.ld");
    }"#,
    )
    .map_err(|_| anyhow::anyhow!("parsing the built-in kernel build-script contract"))?
    .to_string();
    ensure!(
        actual == expected,
        "kernel build script performs work outside its exact linker contract"
    );
    Ok(())
}

fn source_tokens(source: &str, span: Span) -> Result<String> {
    let start = byte_offset(source, span.start())?;
    let end = byte_offset(source, span.end())?;
    let selected = source
        .get(start..end)
        .context("source span is not an ordered UTF-8 range")?;
    Ok(<TokenStream as core::str::FromStr>::from_str(selected)
        .map_err(|_| anyhow::anyhow!("parsing source span as Rust tokens"))?
        .to_string())
}

#[derive(Debug)]
struct ModuleRoute {
    relative: PathBuf,
    module_directory: PathBuf,
    context: ModuleContext,
}

#[derive(Debug, Clone)]
struct Inventory {
    files: Vec<SourceFile>,
    nodes: Vec<Node>,
    antichain_rounds: usize,
    rounds: usize,
    trait_activations: Vec<TraitActivation>,
    implicit_trait_activations: Vec<ImplicitTraitActivation>,
    unresolved_trait_implementations: BTreeSet<usize>,
    entry: Option<EntryLink>,
}

#[derive(Debug, Clone)]
struct TraitActivation {
    self_type: usize,
    exact_self_type: bool,
    declaration: Option<usize>,
    key: TraitMemberKey,
    implementation: usize,
}

#[derive(Debug, Clone)]
struct ImplicitTraitActivation {
    trigger: ImplicitTraitTrigger,
    implementation: usize,
}

#[derive(Debug, Clone, Copy)]
enum ImplicitTraitTrigger {
    ConstructedType(usize),
    LiveNode(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportOrigin {
    Standard,
    Dependency,
    Other,
}

#[derive(Debug, Default)]
struct TraitImports {
    paths: BTreeMap<Vec<String>, BTreeMap<String, Option<Vec<String>>>>,
    local_names: BTreeMap<Vec<String>, BTreeSet<String>>,
    external_names: BTreeMap<Vec<String>, BTreeMap<String, ImportOrigin>>,
    globs: BTreeMap<Vec<String>, Vec<Vec<String>>>,
}

#[cfg(test)]
fn fixture_targets(root: &Path) -> Result<Vec<CargoTarget>> {
    let mut paths = Vec::new();
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        collect_optional_rust_files(&directory, &mut paths)?;
    }
    let mut targets = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is outside the fixture root", path.display()))?
            .to_path_buf();
        let components = relative
            .components()
            .filter_map(|component| component.as_os_str().to_str())
            .collect::<Vec<_>>();
        let (package, name, kind, executable) = match components.as_slice() {
            ["libs", package, "src", "lib.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Library,
                false,
            ),
            ["libs", package, "src", "main.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Bin,
                true,
            ),
            [package @ ("kernel" | "wost" | "xtask"), "src", "lib.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Library,
                false,
            ),
            [package @ ("kernel" | "wost" | "xtask"), "src", "main.rs"] => (
                *package,
                package.replace('-', "_"),
                CargoTargetKind::Bin,
                true,
            ),
            _ => continue,
        };
        targets.push(CargoTarget {
            package: package.to_string(),
            package_id: package.to_string(),
            name,
            relative,
            kind,
            test: true,
            executable,
        });
    }
    Ok(targets)
}

impl Inventory {
    fn discover_from_catalog(root: &Path, catalog: &CargoCatalog) -> Result<Self> {
        let authority = if catalog.packages.contains_key(crate::paths::KERNEL_PACKAGE) {
            ensure!(
                catalog.has_direct_normal_dependency(crate::paths::KERNEL_PACKAGE, "wost", "wost"),
                "the kernel entry model requires a direct normal `wost` dependency"
            );
            Some(EntryContractAuthority {
                uses: validate_wost_entry_contract(root)?,
            })
        } else {
            None
        };
        let targets = catalog.targets().cloned().collect::<Vec<_>>();
        Self::discover_with_targets_and_entry_authority(root, &targets, authority.as_ref())
    }

    #[cfg(test)]
    fn discover(root: &Path) -> Result<Self> {
        let targets = fixture_targets(root)?;
        Self::discover_with_targets(root, &targets)
    }

    #[cfg(test)]
    fn discover_with_targets(root: &Path, targets: &[CargoTarget]) -> Result<Self> {
        Self::discover_with_targets_and_entry_authority(root, targets, None)
    }

    fn discover_with_targets_and_entry_authority(
        root: &Path,
        targets: &[CargoTarget],
        authority: Option<&EntryContractAuthority>,
    ) -> Result<Self> {
        let mut audited = BTreeSet::new();
        for relative in SOURCE_ROOTS {
            let mut paths = Vec::new();
            collect_rust_files(&root.join(relative), &mut paths)?;
            audited.extend(paths);
        }
        audited.retain(|path| audited_source(root, path));
        let mut paths = Vec::new();
        for directory in COPY_DIRECTORIES {
            let directory = root.join(directory);
            collect_optional_rust_files(&directory, &mut paths)?;
        }
        paths.sort();
        paths.dedup();
        let contexts = discover_module_contexts(root, &paths, &audited, targets)?;
        let macro_policies = discover_macro_policies(root, &paths, &contexts)?;

        let mut files = Vec::new();
        let mut nodes = Vec::new();
        let mut entry_specs = Vec::new();
        for path in paths {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("{} is outside the audit root", path.display()))?
                .to_path_buf();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            ensure!(
                !contains_diagnostic_marker(&text).with_context(|| {
                    format!(
                        "decoding literals while scanning {} for the reserved API marker",
                        relative.display()
                    )
                })?,
                "{} contains the API audit's reserved diagnostic marker",
                relative.display()
            );
            let Some(context) = contexts.get(&relative) else {
                continue;
            };
            let origins = targets
                .iter()
                .filter(|target| {
                    target.package == context.package
                        && target.relative == context.target_root
                        && target.kind == context.target_kind
                        && target.name.replace('-', "_") == context.crate_name
                })
                .collect::<Vec<_>>();
            let [origin] = origins.as_slice() else {
                bail!(
                    "{} resolves to {} Cargo target origins",
                    relative.display(),
                    origins.len()
                );
            };
            let syntax =
                syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
            validate_kernel_entry_source(&relative, context, &syntax)?;
            validate_harness_target_source(&relative, context, &syntax)?;
            if audited.contains(&path) {
                validate_audited_attributes(&relative, &syntax)?;
            }
            let file = files.len();
            let mut non_call_uses = NonCallUses::default();
            non_call_uses.visit_file(&syntax);
            let mut macro_syntax = MacroSyntax {
                unqualified_standard: *macro_policies
                    .get(&context.target_root)
                    .context("a classified source has no macro-resolution policy")?,
                ..MacroSyntax::default()
            };
            macro_syntax.visit_file(&syntax);
            let mut dispatch_sites = DispatchSites::default();
            dispatch_sites.visit_file(&syntax);
            let mut value_constructions = ValueConstructions::default();
            value_constructions.visit_file(&syntax);
            let trait_imports = TraitImports::collect_in_scope(&syntax.items, &context.scope);
            let reexports = {
                let mut collector = Collector {
                    audit: audited.contains(&path),
                    file,
                    relative: &relative,
                    nodes: &mut nodes,
                    trait_imports: &trait_imports,
                    reexports: Vec::new(),
                    unsupported_macros: Vec::new(),
                    entry_specs: &mut entry_specs,
                    scope: context.scope.clone(),
                    package: &context.package,
                    target_root: &context.target_root,
                    target_kind: context.target_kind,
                    language_main: context.language_main,
                    lexical_item_depth: 0,
                };
                for item in &syntax.items {
                    collector.item(item, context.availability);
                }
                ensure!(
                    collector.unsupported_macros.is_empty(),
                    "{} contains item macro invocation(s) whose public expansion the API audit cannot inventory: {}",
                    relative.display(),
                    collector.unsupported_macros.join(", ")
                );
                collector.reexports
            };
            files.push(SourceFile {
                relative,
                crate_name: context.crate_name.clone(),
                origin: (*origin).clone(),
                text,
                reexports,
                non_call_uses: non_call_uses.ranges,
                macro_syntax: macro_syntax.ranges,
                transparent_macros: macro_syntax.transparent,
                macro_paths: macro_syntax.paths,
                dispatch_sites: dispatch_sites.sites,
                value_constructions: value_constructions.sites,
            });
        }

        ensure!(
            nodes.iter().any(|node| node.report),
            "the public API inventory is empty"
        );
        nodes.sort_by_key(|node| {
            (
                files[node.file].relative.clone(),
                node.insertion.line,
                node.insertion.column,
                node.label.clone(),
            )
        });
        let codes = antichain_codes(nodes.iter().filter(|node| node.instrument).count());
        let antichain_rounds = codes
            .iter()
            .map(|code| 64 - code.leading_zeros() as usize)
            .max()
            .unwrap_or(0);
        let mut codes = codes.into_iter();
        for (id, node) in nodes.iter_mut().enumerate() {
            node.id = id;
            if node.instrument {
                node.code = core::iter::Iterator::next(&mut codes)
                    .with_context(|| format!("assigning an antichain code to {}", node.label))?;
            }
        }
        let expects_entry = targets.iter().any(|target| {
            target.package == crate::paths::KERNEL_PACKAGE && target.kind == CargoTargetKind::Bin
        });
        let entry = resolve_entry_link(&nodes, &files, &entry_specs, expects_entry, authority)?;
        ensure!(
            antichain_rounds > 0,
            "the API audit produced no compiler rounds"
        );
        let (trait_activations, implicit_trait_activations, unresolved_trait_implementations) =
            trait_activations(&nodes, &files)?;

        Ok(Self {
            files,
            nodes,
            antichain_rounds,
            rounds: antichain_rounds + ISOLATIONS.len(),
            trait_activations,
            implicit_trait_activations,
            unresolved_trait_implementations,
            entry,
        })
    }

    fn instrument(&self, root: &Path, round: usize) -> Result<Instrumentation> {
        let mut by_file: BTreeMap<usize, Vec<&Node>> = BTreeMap::new();
        for node in self.nodes.iter().filter(|node| {
            node.instrument
                && if round < self.antichain_rounds {
                    node.code & (1 << round) != 0
                } else {
                    node.isolation == Some(ISOLATIONS[round - self.antichain_rounds])
                }
        }) {
            by_file.entry(node.file).or_default().push(node);
        }

        let mut inserted = BTreeMap::new();
        let mut ids = BTreeSet::new();
        for (index, file) in self.files.iter().enumerate() {
            let mut text = file.text.clone();
            let mut additions = Vec::new();
            let mut selected = by_file.get(&index).cloned().unwrap_or_default();
            selected.sort_by_key(|node| {
                std::cmp::Reverse((node.insertion.line, node.insertion.column))
            });
            for node in selected {
                ids.insert(node.id);
                let attribute = format!("#[deprecated(note = \"{MARKER}{}\")] ", node.id);
                let offset = byte_offset(&text, node.insertion)?;
                additions.push(Insertion {
                    line: node.insertion.line,
                    column: node.insertion.column,
                    bytes: attribute.len(),
                });
                text.insert_str(offset, &attribute);
            }
            additions.sort_by_key(|addition| addition.column);
            inserted.insert(index, additions);
            let path = root.join(&file.relative);
            std::fs::write(&path, text)
                .with_context(|| format!("instrumenting {}", path.display()))?;
        }
        Ok(Instrumentation {
            insertions: inserted,
            ids,
        })
    }

    fn caller(&self, file: usize, at: LineColumn) -> Option<usize> {
        self.nodes
            .iter()
            .filter(|node| node.file == file && node.range.contains(at))
            .min_by_key(|node| node.range.extent())
            .map(|node| node.id)
    }

    fn is_reexport(&self, file: usize, at: LineColumn) -> bool {
        self.files[file]
            .reexports
            .iter()
            .any(|range| range.contains(at))
    }

    fn is_non_call_use(&self, file: usize, at: LineColumn) -> bool {
        self.files[file]
            .non_call_uses
            .iter()
            .any(|range| range.contains(at))
    }

    fn exact_entry_invocation_caller(
        &self,
        file: usize,
        at: LineColumn,
        used: usize,
    ) -> Option<usize> {
        let entry = self.entry?;
        (entry.file == file
            && entry.invocation.contains(at)
            && (used == entry.function_node && entry.argument.contains(at)
                || used == entry.macro_declaration && entry.macro_path.contains(at)))
        .then_some(entry.invocation_node)
    }

    fn exact_entry_contract_expansion_caller(
        &self,
        primary_file: usize,
        primary_at: LineColumn,
        used: usize,
        expansion_file: usize,
        expansion_at: LineColumn,
        expansion: &DiagnosticExpansion,
    ) -> Option<usize> {
        let entry = self.entry?;
        let declaration = &self.nodes[entry.macro_declaration];
        let contract = entry
            .contract_nodes
            .iter()
            .find(|contract| contract.id == used)?;
        (contract.primary.contains(primary_at)
            && declaration.file == primary_file
            && declaration.range.contains(primary_at)
            && expansion.terminal
            && expansion.macro_decl_name == "::wost::entry!"
            && expansion_file == entry.file
            && expansion_at == entry.invocation.start)
            .then_some(entry.invocation_node)
    }

    fn is_opaque_macro_use(&self, file: usize, at: LineColumn) -> bool {
        let Some(innermost) = self.files[file]
            .macro_syntax
            .iter()
            .filter(|range| range.contains(at))
            .min_by_key(|range| range.extent())
        else {
            return false;
        };
        !self.files[file].transparent_macros.contains(innermost)
    }

    fn is_direct_macro_call(&self, file: usize, at: LineColumn, used: usize) -> bool {
        self.nodes[used].macro_declaration
            && self.files[file]
                .macro_paths
                .iter()
                .any(|range| range.contains(at))
    }

    fn dispatch_site(&self, file: usize, at: LineColumn) -> Option<usize> {
        self.files[file]
            .dispatch_sites
            .iter()
            .enumerate()
            .filter(|(_, site)| {
                site.callee_evidence
                    .iter()
                    .chain(&site.self_evidence)
                    .any(|range| range.contains(at))
            })
            .min_by_key(|(_, site)| site.range.extent())
            .map(|(index, _)| index)
    }

    fn is_value_construction(&self, file: usize, at: LineColumn, used: usize) -> bool {
        self.nodes[used].type_declaration
            && self.files[file]
                .value_constructions
                .iter()
                .any(|site| site.range.contains(at))
    }

    fn file_index(&self, root: &Path, reported: &str) -> Option<usize> {
        let reported = Path::new(reported);
        let reported = if reported.is_absolute() {
            reported.to_path_buf()
        } else {
            root.join(reported)
        };
        let reported = reported.canonicalize().ok()?;
        let root = root.canonicalize().ok()?;
        let relative = reported.strip_prefix(root).ok()?;
        self.files.iter().position(|file| file.relative == relative)
    }

    fn report_count(&self) -> usize {
        self.nodes.iter().filter(|node| node.report).count()
    }
}

fn resolve_entry_link(
    nodes: &[Node],
    files: &[SourceFile],
    specs: &[EntrySpec],
    expected: bool,
    authority: Option<&EntryContractAuthority>,
) -> Result<Option<EntryLink>> {
    if !expected {
        ensure!(
            specs.is_empty() && authority.is_none(),
            "an unexpected kernel entry model was admitted"
        );
        return Ok(None);
    }
    let [spec] = specs else {
        bail!("the kernel must contain exactly one trusted `::wost::entry!(kernel_main)` binding");
    };
    let Some(authority) = authority else {
        return Ok(None);
    };
    let unique = |description: &str, candidates: Vec<&Node>| -> Result<usize> {
        let [node] = candidates.as_slice() else {
            bail!(
                "the kernel entry model found {} {description} candidates",
                candidates.len()
            );
        };
        Ok(node.id)
    };
    let invocation_node = unique(
        "entry invocation",
        nodes
            .iter()
            .filter(|node| {
                node.file == spec.file
                    && node.range == spec.invocation
                    && node.symbol == "entry"
                    && node.roots.production
            })
            .collect(),
    )?;
    let function_node = unique(
        "kernel handler",
        nodes
            .iter()
            .filter(|node| {
                node.file == spec.file
                    && node.symbol == "kernel_main"
                    && !node.type_declaration
                    && !node.macro_declaration
            })
            .collect(),
    )?;
    let macro_declaration = unique(
        "audited `wost::entry` declaration",
        nodes
            .iter()
            .filter(|node| {
                node.macro_declaration
                    && node.symbol == "boot::entry"
                    && files[node.file].relative == Path::new("wost/src/boot/mod.rs")
            })
            .collect(),
    )?;
    let contract_node = |contract: EntryContractUse| -> Result<EntryContractNode> {
        Ok(EntryContractNode {
            id: unique(
                "audited entry-contract item",
                nodes
                    .iter()
                    .filter(|node| node.symbol == contract.symbol)
                    .collect(),
            )?,
            primary: contract.primary,
        })
    };
    let [first, second, third, fourth] = authority.uses;
    let contract_nodes = [
        contract_node(first)?,
        contract_node(second)?,
        contract_node(third)?,
        contract_node(fourth)?,
    ];
    Ok(Some(EntryLink {
        file: spec.file,
        invocation: spec.invocation,
        macro_path: spec.macro_path,
        argument: spec.argument,
        invocation_node,
        function_node,
        macro_declaration,
        contract_nodes,
    }))
}

fn entry_contract_use(
    generated: &syn::File,
    symbol: &'static str,
    normalized_path: &str,
) -> Result<EntryContractUse> {
    struct MatchingPaths<'a> {
        expected: &'a str,
        ranges: Vec<SourceRange>,
    }

    impl<'ast> Visit<'ast> for MatchingPaths<'_> {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            if trait_path(path) == self.expected {
                self.ranges.push(range(path.span()));
            }
            visit::visit_path(self, path);
        }
    }

    let mut matching = MatchingPaths {
        expected: normalized_path,
        ranges: Vec::new(),
    };
    matching.visit_file(generated);
    let [primary] = matching.ranges.as_slice() else {
        bail!(
            "the validated `wost::entry` expansion contains {} exact `{normalized_path}` uses",
            matching.ranges.len()
        );
    };
    Ok(EntryContractUse {
        symbol,
        primary: *primary,
    })
}

fn validate_wost_entry_contract(root: &Path) -> Result<[EntryContractUse; 4]> {
    let relative = Path::new("wost/src/boot/mod.rs");
    let source = std::fs::read_to_string(root.join(relative))
        .with_context(|| format!("reading {}", relative.display()))?;
    let syntax =
        syn::parse_file(&source).with_context(|| format!("parsing {}", relative.display()))?;
    let candidates = syntax
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Macro(item) if item.ident.as_ref().is_some_and(|ident| ident == "entry") => {
                Some(item)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let [entry] = candidates.as_slice() else {
        bail!(
            "{} must define exactly one `entry` macro",
            relative.display()
        );
    };
    let semantic_attributes = entry
        .attrs
        .iter()
        .filter(|attribute| !attribute.path().is_ident("doc"))
        .collect::<Vec<_>>();
    ensure!(
        entry.mac.path.is_ident("macro_rules")
            && matches!(semantic_attributes.as_slice(), [attribute]
                if matches!(attribute.style, syn::AttrStyle::Outer)
                    && attribute.path().is_ident("macro_export")
                    && matches!(&attribute.meta, Meta::Path(_))),
        "the `wost::entry` declaration lost its exported macro identity"
    );
    let tokens = entry.mac.tokens.clone().into_iter().collect::<Vec<_>>();
    ensure!(
        matches!(tokens.as_slice(), [TokenTree::Group(_), TokenTree::Punct(equal), TokenTree::Punct(arrow), TokenTree::Group(_), TokenTree::Punct(semi)]
            if equal.as_char() == '=' && arrow.as_char() == '>' && semi.as_char() == ';'),
        "the `wost::entry` macro must contain one explicit rule"
    );
    let TokenTree::Group(matcher) = &tokens[0] else {
        unreachable!("the checked entry matcher is a group");
    };
    ensure!(
        matcher.delimiter() == Delimiter::Parenthesis
            && matcher.stream().to_string() == "$ path : path",
        "the `wost::entry` macro must accept exactly one path"
    );
    let TokenTree::Group(expansion) = &tokens[3] else {
        unreachable!("the checked entry expansion is a group");
    };
    ensure!(
        expansion.delimiter() == Delimiter::Brace,
        "the `wost::entry` expansion is not a block"
    );
    let mut handler_uses = 0;
    let expanded = substitute_entry_metavariables(expansion.stream(), &mut handler_uses)?;
    ensure!(
        handler_uses == 1,
        "the `wost::entry` expansion must bind its handler exactly once"
    );
    let generated = syn::parse2::<syn::File>(expanded.clone())
        .context("parsing the normalized `wost::entry` expansion")?;
    let expected_statics = [
        "__WOST_REQUESTS_START",
        "__WOST_BASE_REVISION",
        "__WOST_FRAMEBUFFER",
        "__WOST_MEMORY_MAP",
        "__WOST_DIRECT_MAP",
        "__WOST_IMAGE",
        "__WOST_RSDP",
        "__WOST_MODULES",
        "__WOST_REQUESTS_END",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let mut actual_statics = BTreeSet::new();
    for item in &generated.items {
        match item {
            Item::Const(item)
                if item.ident == "__WOST_ENTRY_SIGNATURE"
                    && matches!(item.vis, Visibility::Inherited) => {}
            Item::Static(item) if matches!(item.vis, Visibility::Inherited) => {
                actual_statics.insert(item.ident.to_string());
            }
            Item::Fn(item)
                if item.sig.ident == "kmain" && matches!(item.vis, Visibility::Inherited) => {}
            _ => bail!("the `wost::entry` expansion contains an unapproved or visible item"),
        }
    }
    ensure!(
        generated.items.len() == expected_statics.len() + 2
            && actual_statics
                == expected_statics
                    .iter()
                    .map(|name| (*name).to_string())
                    .collect(),
        "the `wost::entry` expansion changed its exact request-item set"
    );
    let signatures = generated
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Const(item) if item.ident == "__WOST_ENTRY_SIGNATURE" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [signature] = signatures.as_slice() else {
        bail!("the `wost::entry` expansion must bind one entry signature");
    };
    ensure!(
        signature.attrs.is_empty()
            && matches!(&*signature.expr, Expr::Path(path) if path.path.is_ident("kernel_main")),
        "the `wost::entry` signature must bind the supplied handler directly"
    );
    let base_revisions = generated
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Static(item) if item.ident == "__WOST_BASE_REVISION" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [base_revision] = base_revisions.as_slice() else {
        bail!("the `wost::entry` expansion must define one base revision request");
    };
    ensure!(
        matches!(&*base_revision.expr, Expr::Call(call)
            if call.args.is_empty()
                && matches!(&*call.func, Expr::Path(path)
                    if path.qself.is_none()
                        && trait_path(&path.path) == "wost::boot::__macro::base_revision")),
        "the `wost::entry` base revision must use the audited constructor directly"
    );
    let entry_points = generated
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(item) if item.sig.ident == "kmain" => Some(item),
            _ => None,
        })
        .collect::<Vec<_>>();
    let [entry_point] = entry_points.as_slice() else {
        bail!("the `wost::entry` expansion must define one `kmain`");
    };
    ensure!(
        entry_point
            .sig
            .abi
            .as_ref()
            .and_then(|abi| abi.name.as_ref())
            .is_some_and(|name| name.value() == "C")
            && matches!(entry_point.sig.output, syn::ReturnType::Type(_, ref ty) if matches!(&**ty, syn::Type::Never(_)))
            && has_semantic_attribute(&entry_point.attrs, "no_mangle"),
        "the `wost::entry` expansion lost the linker-visible non-returning C entry point"
    );
    let Some(syn::Stmt::Expr(Expr::Call(call), None)) = entry_point.block.stmts.last() else {
        bail!("the `wost::entry` `kmain` must end by entering the typed boot path");
    };
    let Some(Expr::Struct(requests)) = call.args.first() else {
        bail!("the `wost::entry` `kmain` must pass the exact request block to `boot::enter`");
    };
    let expected_requests = [
        ("base_revision", "__WOST_BASE_REVISION"),
        ("framebuffer", "__WOST_FRAMEBUFFER"),
        ("memory_map", "__WOST_MEMORY_MAP"),
        ("hhdm", "__WOST_DIRECT_MAP"),
        ("image", "__WOST_IMAGE"),
        ("rsdp", "__WOST_RSDP"),
        ("modules", "__WOST_MODULES"),
    ];
    let request_fields_match = requests.fields.len() == expected_requests.len()
        && requests.fields.iter().zip(expected_requests).all(
            |(field, (expected_field, expected_static))| {
                matches!(&field.member, syn::Member::Named(name) if name == expected_field)
                    && field.colon_token.is_some()
                    && matches!(&field.expr, Expr::Reference(reference)
                        if reference.attrs.is_empty()
                            && reference.mutability.is_none()
                            && matches!(&*reference.expr, Expr::Path(path)
                                if path.qself.is_none()
                                    && path.path.is_ident(expected_static)))
            },
        );
    ensure!(
        matches!(&*call.func, Expr::Path(path) if trait_path(&path.path) == "wost::boot::enter")
            && call.args.len() == 2
            && requests.attrs.is_empty()
            && requests.qself.is_none()
            && trait_path(&requests.path) == "wost::boot::__macro::Requests"
            && requests.rest.is_none()
            && request_fields_match
            && matches!(&call.args[1], Expr::Path(path) if path.path.is_ident("__WOST_ENTRY_SIGNATURE")),
        "the `wost::entry` `kmain` must pass the exact request block and bound handler directly to `boot::enter`"
    );
    let expected = expected_wost_entry_expansion()?;
    ensure!(
        expanded.to_string() == expected.to_string(),
        "the `wost::entry` expansion changed outside its exact audited contract"
    );
    Ok([
        entry_contract_use(&generated, "boot::BootInfo", "wost::boot::BootInfo")?,
        entry_contract_use(
            &generated,
            "boot::limine_glue::base_revision",
            "wost::boot::__macro::base_revision",
        )?,
        entry_contract_use(&generated, "boot::limine_glue::enter", "wost::boot::enter")?,
        EntryContractUse {
            symbol: "boot::limine_glue::Requests",
            primary: range(requests.span()),
        },
    ])
}

fn expected_wost_entry_expansion() -> Result<TokenStream> {
    <TokenStream as core::str::FromStr>::from_str(
        r#"
const __WOST_ENTRY_SIGNATURE: fn(wost::boot::BootInfo) -> ! = kernel_main;

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests_start_marker")]
static __WOST_REQUESTS_START: wost::boot::__macro::RequestsStartMarker =
    wost::boot::__macro::RequestsStartMarker::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_BASE_REVISION: wost::boot::__macro::BaseRevision =
    wost::boot::__macro::base_revision();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_FRAMEBUFFER: wost::boot::__macro::FramebufferRequest =
    wost::boot::__macro::FramebufferRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_MEMORY_MAP: wost::boot::__macro::MemoryMapRequest =
    wost::boot::__macro::MemoryMapRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_DIRECT_MAP: wost::boot::__macro::HhdmRequest =
    wost::boot::__macro::HhdmRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_IMAGE: wost::boot::__macro::ExecutableAddressRequest =
    wost::boot::__macro::ExecutableAddressRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_RSDP: wost::boot::__macro::RsdpRequest =
    wost::boot::__macro::RsdpRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests")]
static __WOST_MODULES: wost::boot::__macro::ModuleRequest =
    wost::boot::__macro::ModuleRequest::new();

#[used]
#[allow(unsafe_code)]
#[unsafe(link_section = ".requests_end_marker")]
static __WOST_REQUESTS_END: wost::boot::__macro::RequestsEndMarker =
    wost::boot::__macro::RequestsEndMarker::new();

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
extern "C" fn kmain() -> ! {
    wost::boot::enter(
        wost::boot::__macro::Requests {
            base_revision: &__WOST_BASE_REVISION,
            framebuffer: &__WOST_FRAMEBUFFER,
            memory_map: &__WOST_MEMORY_MAP,
            hhdm: &__WOST_DIRECT_MAP,
            image: &__WOST_IMAGE,
            rsdp: &__WOST_RSDP,
            modules: &__WOST_MODULES,
        },
        __WOST_ENTRY_SIGNATURE,
    )
}
"#,
    )
    .map_err(|error| anyhow::anyhow!("parsing the expected `wost::entry` expansion: {error}"))
}

fn substitute_entry_metavariables(
    stream: TokenStream,
    handler_uses: &mut usize,
) -> Result<TokenStream> {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    let mut normalized = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        match &tokens[index] {
            TokenTree::Punct(punctuation) if punctuation.as_char() == '$' => {
                let Some(TokenTree::Ident(name)) = tokens.get(index + 1) else {
                    bail!("the `wost::entry` expansion contains unsupported macro repetition");
                };
                let replacement = if name == "crate" {
                    "wost"
                } else if name == "path" {
                    *handler_uses += 1;
                    "kernel_main"
                } else {
                    bail!("the `wost::entry` expansion contains unknown metavariable `${name}`");
                };
                normalized.push(TokenTree::Ident(proc_macro2::Ident::new(
                    replacement,
                    name.span(),
                )));
                index += 2;
            }
            TokenTree::Group(group) => {
                let mut replacement = proc_macro2::Group::new(
                    group.delimiter(),
                    substitute_entry_metavariables(group.stream(), handler_uses)?,
                );
                replacement.set_span(group.span());
                normalized.push(TokenTree::Group(replacement));
                index += 1;
            }
            token => {
                normalized.push(token.clone());
                index += 1;
            }
        }
    }
    Ok(normalized.into_iter().collect())
}

#[derive(Default)]
struct AttributeValidator {
    unsupported: Vec<String>,
}

impl<'ast> Visit<'ast> for AttributeValidator {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if let Err(error) = validate_attribute_meta(&attribute.meta) {
            self.unsupported
                .push(format!("line {}: {error:#}", attribute.span().start().line));
        }
        visit::visit_attribute(self, attribute);
    }
}

fn validate_audited_attributes(relative: &Path, syntax: &syn::File) -> Result<()> {
    let mut validator = AttributeValidator::default();
    validator.visit_file(syntax);
    ensure!(
        validator.unsupported.is_empty(),
        "{} contains attribute syntax outside the API audit's built-in trust boundary: {}",
        relative.display(),
        validator.unsupported.join(", ")
    );
    Ok(())
}

fn validate_attribute_meta(meta: &Meta) -> Result<()> {
    let name = single_path_name(meta.path()).context("qualified attribute path")?;
    match name.as_str() {
        "derive" => {
            let Meta::List(list) = meta else {
                bail!("derive attribute has no trait list");
            };
            let derives = Punctuated::<syn::Path, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .context("derive attribute has malformed traits")?;
            for derive in derives {
                let derive = single_path_name(&derive).context("qualified derive path")?;
                ensure!(
                    matches!(
                        derive.as_str(),
                        "Clone"
                            | "Copy"
                            | "Debug"
                            | "Default"
                            | "Eq"
                            | "Ord"
                            | "PartialEq"
                            | "PartialOrd"
                    ),
                    "unsupported derive `{derive}`"
                );
            }
        }
        "cfg_attr" => {
            let Meta::List(list) = meta else {
                bail!("cfg_attr has no arguments");
            };
            let nested = Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .context("cfg_attr has malformed arguments")?;
            ensure!(nested.len() >= 2, "cfg_attr has no conditional attribute");
            for attribute in nested.iter().skip(1) {
                ensure!(
                    !conditional_semantic_root(attribute),
                    "cfg_attr conditionally controls a semantic runtime root"
                );
                validate_attribute_meta(attribute)?;
            }
        }
        "unsafe" => {
            let Meta::List(list) = meta else {
                bail!("unsafe attribute has no payload");
            };
            let nested = Punctuated::<Meta, Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .context("unsafe attribute has malformed payload")?;
            ensure!(nested.len() == 1, "unsafe attribute has ambiguous payload");
            let payload =
                single_path_name(nested[0].path()).context("qualified unsafe attribute")?;
            ensure!(
                matches!(payload.as_str(), "link_section" | "no_mangle" | "naked"),
                "unsupported unsafe attribute `{payload}`"
            );
        }
        "allow" | "cfg" | "default" | "deny" | "deprecated" | "doc" | "expect" | "forbid"
        | "global_allocator" | "ignore" | "inline" | "link_name" | "macro_export" | "must_use"
        | "no_main" | "no_std" | "non_exhaustive" | "panic_handler" | "path" | "repr"
        | "should_panic" | "test" | "track_caller" | "used" | "warn" => {}
        _ => bail!("unsupported attribute `{name}`"),
    }
    Ok(())
}

fn conditional_semantic_root(meta: &Meta) -> bool {
    let Some(name) = single_path_name(meta.path()) else {
        return false;
    };
    if matches!(
        name.as_str(),
        "no_mangle"
            | "export_name"
            | "panic_handler"
            | "alloc_error_handler"
            | "used"
            | "global_allocator"
    ) {
        return true;
    }
    if !matches!(name.as_str(), "unsafe" | "cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    Punctuated::<Meta, Token![,]>::parse_terminated
        .parse2(list.tokens.clone())
        .is_ok_and(|nested| {
            let skip = usize::from(name == "cfg_attr");
            nested.iter().skip(skip).any(conditional_semantic_root)
        })
}

fn single_path_name(path: &syn::Path) -> Option<String> {
    if path.leading_colon.is_some() || path.segments.len() != 1 {
        return None;
    }
    let segment = path.segments.first()?;
    matches!(segment.arguments, syn::PathArguments::None).then(|| segment.ident.to_string())
}

fn trait_activations(
    nodes: &[Node],
    files: &[SourceFile],
) -> Result<(
    Vec<TraitActivation>,
    Vec<ImplicitTraitActivation>,
    BTreeSet<usize>,
)> {
    for producer in REVIEWED_IMPLICIT_DROP_PRODUCERS {
        ensure!(
            REVIEWED_IMPLICIT_DROP_TYPES
                .iter()
                .any(|(relative, symbol)| {
                    *relative == producer.relative && *symbol == producer.produced
                }),
            "reviewed implicit Drop producer {}::{} has an unreviewed result type {}",
            producer.owner,
            producer.member,
            producer.produced
        );
    }
    let mut reviewed_producer_nodes = BTreeMap::new();
    let mut consumed_producer_nodes = BTreeSet::new();
    for (producer_index, producer) in REVIEWED_IMPLICIT_DROP_PRODUCERS.iter().enumerate() {
        let produced_types = nodes
            .iter()
            .filter(|node| {
                node.type_declaration
                    && files[node.file].relative == Path::new(producer.relative)
                    && node.symbol == producer.produced
            })
            .collect::<Vec<_>>();
        if produced_types.is_empty() {
            continue;
        }
        ensure!(
            produced_types.len() == 1,
            "reviewed implicit Drop result type {} resolves to {} declarations",
            producer.produced,
            produced_types.len()
        );
        let produced_type = produced_types[0].id;
        let drop_implementations = nodes
            .iter()
            .filter(|node| {
                let Some(TraitRoute::Implementation {
                    self_type: Some(self_type),
                    implicit: Some(ImplicitTrait::Drop),
                    ..
                }) = &node.trait_route
                else {
                    return false;
                };
                matching_type_nodes(nodes, files, node, self_type).contains(&produced_type)
            })
            .count();
        if drop_implementations == 0 {
            continue;
        }
        ensure!(
            drop_implementations == 1,
            "reviewed implicit Drop result type {} resolves to {drop_implementations} Drop implementations",
            producer.produced
        );
        let producers = nodes
            .iter()
            .filter(|node| node.reviewed_drop_producer == Some(producer_index))
            .collect::<Vec<_>>();
        ensure!(
            producers.len() == 1,
            "reviewed implicit Drop producer {}::{} resolves to {} nodes",
            producer.owner,
            producer.member,
            producers.len()
        );
        ensure!(
            producers[0].direct_self_construction,
            "reviewed implicit Drop producer {}::{} no longer constructs Self",
            producer.owner,
            producer.member
        );
        let producer_owner = producers[0]
            .implementation_owner
            .as_deref()
            .context("a reviewed implicit Drop producer has no implementation owner")?;
        ensure!(
            matching_type_nodes(nodes, files, producers[0], producer_owner) == vec![produced_type],
            "reviewed implicit Drop producer {}::{} does not belong to its exact result type {}",
            producer.owner,
            producer.member,
            producer.produced
        );
        ensure!(
            consumed_producer_nodes.insert(producers[0].id),
            "one implicit Drop producer node consumed more than one reviewed contract"
        );
        reviewed_producer_nodes.insert(producer_index, producers[0].id);
    }
    let declarations = nodes
        .iter()
        .filter_map(|node| match &node.trait_route {
            Some(TraitRoute::Declaration(key)) => Some((key, node.id)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut allocator_types = Vec::new();
    for allocator in nodes.iter().filter(|node| node.global_allocator) {
        let name = allocator
            .global_allocator_type
            .as_deref()
            .with_context(|| {
                format!(
                    "global allocator {} has no exact nominal type",
                    allocator.label
                )
            })?;
        let candidates = matching_type_nodes(nodes, files, allocator, name);
        ensure!(
            candidates.len() == 1,
            "global allocator {} resolves type `{name}` to {} declarations",
            allocator.label,
            candidates.len()
        );
        ensure!(
            !nodes[candidates[0]].type_alias,
            "global allocator {} uses unsupported type alias `{name}`",
            allocator.label
        );
        allocator_types.push((allocator.id, candidates[0]));
    }
    let mut activations = Vec::new();
    let mut implicit_activations = Vec::new();
    let mut unresolved = BTreeSet::new();
    for node in nodes {
        let Some(TraitRoute::Implementation {
            key,
            self_type,
            exact_self_type,
            external,
            implicit,
        }) = &node.trait_route
        else {
            continue;
        };
        let exact = declarations
            .iter()
            .filter(|(declaration, _)| *declaration == key)
            .map(|(_, id)| *id)
            .collect::<Vec<_>>();
        let qualified = declarations
            .iter()
            .filter(|(declaration, id)| {
                workspace_trait_matches(declaration, key, &files[nodes[*id].file].crate_name)
            })
            .map(|(_, id)| *id)
            .collect::<Vec<_>>();
        let declarations = if !exact.is_empty() {
            exact
        } else if !qualified.is_empty() {
            qualified
        } else if !key.trait_path.contains("::") {
            declarations
                .iter()
                .filter(|(declaration, _)| trait_member_matches(declaration, key))
                .map(|(_, id)| *id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        let self_types = self_type
            .iter()
            .flat_map(|name| matching_type_nodes(nodes, files, node, name))
            .collect::<Vec<_>>();
        if self_types.len() != 1 || declarations.len() > 1 || declarations.is_empty() && !*external
        {
            unresolved.insert(node.id);
            continue;
        }
        let self_type = self_types[0];
        activations.push(TraitActivation {
            self_type,
            exact_self_type: *exact_self_type,
            declaration: declarations.first().copied(),
            key: key.clone(),
            implementation: node.id,
        });
        match implicit {
            Some(ImplicitTrait::Drop) if key.member == "drop" => {
                let declaration = &nodes[self_type];
                let file = &files[declaration.file];
                let identity = source_item_path(file, &declaration.symbol);
                ensure!(
                    REVIEWED_IMPLICIT_DROP_TYPES
                        .iter()
                        .any(|(relative, symbol)| {
                            file.relative == Path::new(relative) && declaration.symbol == *symbol
                        }),
                    "standard Drop implementation for `{identity}` is outside the API audit's reviewed implicit-drop boundary"
                );
                implicit_activations.push(ImplicitTraitActivation {
                    trigger: ImplicitTraitTrigger::ConstructedType(self_type),
                    implementation: node.id,
                });
                for (producer_index, producer) in
                    REVIEWED_IMPLICIT_DROP_PRODUCERS.iter().enumerate()
                {
                    if file.relative != Path::new(producer.relative)
                        || declaration.symbol != producer.produced
                    {
                        continue;
                    }
                    let producer_node = reviewed_producer_nodes
                        .get(&producer_index)
                        .copied()
                        .with_context(|| {
                            format!(
                                "reviewed implicit Drop producer {}::{} was not validated",
                                producer.owner, producer.member
                            )
                        })?;
                    implicit_activations.push(ImplicitTraitActivation {
                        trigger: ImplicitTraitTrigger::LiveNode(producer_node),
                        implementation: node.id,
                    });
                }
                let unreviewed_producer = nodes.iter().find(|candidate| {
                    let Some(owner) = candidate.implementation_owner.as_deref() else {
                        return false;
                    };
                    candidate.direct_self_construction
                        && candidate.reviewed_drop_producer.is_none()
                        && matching_type_nodes(nodes, files, candidate, owner).contains(&self_type)
                });
                ensure!(
                    unreviewed_producer.is_none(),
                    "{} is an unreviewed implicit Drop producer for `{identity}`",
                    unreviewed_producer
                        .map_or("an unknown method", |producer| producer.label.as_str())
                );
            }
            Some(ImplicitTrait::GlobalAlloc) => {
                ensure!(
                    *exact_self_type,
                    "standard GlobalAlloc implementation {} has a generic or qualified self type",
                    node.label
                );
                implicit_activations.extend(
                    allocator_types
                        .iter()
                        .filter(|(_, allocator_type)| *allocator_type == self_type)
                        .map(|(allocator, _)| ImplicitTraitActivation {
                            trigger: ImplicitTraitTrigger::LiveNode(*allocator),
                            implementation: node.id,
                        }),
                );
            }
            _ => {}
        }
    }
    Ok((activations, implicit_activations, unresolved))
}

fn matching_type_nodes(
    nodes: &[Node],
    files: &[SourceFile],
    reference: &Node,
    name: &str,
) -> Vec<usize> {
    nodes
        .iter()
        .filter(|candidate| {
            candidate.type_declaration
                && ((files[candidate.file].origin == files[reference.file].origin
                    && candidate.symbol == name)
                    || source_item_path(&files[candidate.file], &candidate.symbol) == name
                    || workspace_type_matches(&files[candidate.file], &candidate.symbol, name))
        })
        .map(|candidate| candidate.id)
        .collect()
}

impl TraitActivation {
    fn call_compatible(&self, site: &DispatchSite, nodes: &BTreeSet<usize>) -> bool {
        match &site.kind {
            DispatchKind::Call {
                member,
                trait_qualifier,
            } => {
                member == &self.key.member
                    && trait_qualifier.as_deref().is_none_or(|qualifier| {
                        self.key.trait_path == qualifier
                            || !qualifier.contains("::")
                                && last_path_segment(&self.key.trait_path) == qualifier
                    })
                    && self.declaration.is_none_or(|declaration| {
                        nodes.contains(&declaration) || trait_qualifier.is_none()
                    })
            }
        }
    }

    fn matches(
        &self,
        site: &DispatchSite,
        callee_nodes: &BTreeSet<usize>,
        exact_self_type: Option<usize>,
        has_trait_declaration_evidence: bool,
    ) -> bool {
        if callee_nodes.contains(&self.implementation) {
            return true;
        }
        if site.explicit_qself && site.generic_qself_trait.is_none() && exact_self_type.is_none() {
            return false;
        }
        if !self.call_compatible(site, callee_nodes) || !self.exact_self_type {
            return false;
        }
        if has_trait_declaration_evidence
            && !self
                .declaration
                .is_some_and(|declaration| callee_nodes.contains(&declaration))
        {
            return false;
        }
        if let Some(exact_self_type) = exact_self_type {
            let DispatchKind::Call {
                trait_qualifier, ..
            } = &site.kind;
            // A callee path that names `Self` lets the audited implementation
            // inventory stand in for the trait qualifier (D-295).
            if self.declaration.is_none()
                && trait_qualifier.is_none()
                && !site.callee_names(callee_nodes, exact_self_type)
            {
                return false;
            }
            return self.self_type == exact_self_type;
        }
        if self
            .declaration
            .is_some_and(|declaration| callee_nodes.contains(&declaration))
        {
            return true;
        }
        false
    }
}

fn trait_member_matches(declaration: &TraitMemberKey, implementation: &TraitMemberKey) -> bool {
    declaration.member == implementation.member
        && (declaration.trait_path == implementation.trait_path
            || !implementation.trait_path.contains("::")
                && last_path_segment(&declaration.trait_path) == implementation.trait_path)
}

fn workspace_trait_matches(
    declaration: &TraitMemberKey,
    implementation: &TraitMemberKey,
    declaration_crate: &str,
) -> bool {
    let Some((crate_name, trait_path)) = implementation.trait_path.split_once("::") else {
        return false;
    };
    declaration.member == implementation.member
        && declaration.trait_path == trait_path
        && declaration_crate == crate_name
}

fn discover_module_contexts(
    root: &Path,
    paths: &[PathBuf],
    audited: &BTreeSet<PathBuf>,
    targets: &[CargoTarget],
) -> Result<BTreeMap<PathBuf, ModuleContext>> {
    let available = paths
        .iter()
        .map(|path| {
            path.strip_prefix(root)
                .with_context(|| format!("{} is outside the audit root", path.display()))
                .map(Path::to_path_buf)
        })
        .collect::<Result<BTreeSet<_>>>()?;
    let mut contexts = BTreeMap::new();
    let mut routes = VecDeque::new();
    for target in targets {
        ensure!(
            available.contains(&target.relative),
            "Cargo target source {} is absent from the copied source manifest",
            target.relative.display()
        );
        let relative = &target.relative;
        register_module_context(
            &mut contexts,
            &mut routes,
            relative.clone(),
            relative
                .parent()
                .context("a target root has no containing directory")?
                .to_path_buf(),
            target.context(),
        )?;
    }

    while let Some(route) = routes.pop_front() {
        let source = std::fs::read_to_string(root.join(&route.relative))
            .with_context(|| format!("reading {}", route.relative.display()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parsing {}", route.relative.display()))?;
        route_modules(
            &syntax.items,
            &route.relative,
            &route.module_directory,
            &route.context,
            &available,
            &mut contexts,
            &mut routes,
        )?;
    }

    for path in audited {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is outside the audit root", path.display()))?;
        ensure!(
            contexts.contains_key(relative),
            "audited source {} has no crate-root module route",
            relative.display()
        );
    }
    Ok(contexts)
}

fn register_module_context(
    contexts: &mut BTreeMap<PathBuf, ModuleContext>,
    routes: &mut VecDeque<ModuleRoute>,
    relative: PathBuf,
    module_directory: PathBuf,
    context: ModuleContext,
) -> Result<()> {
    if let Some(existing) = contexts.get(&relative) {
        ensure!(
            existing == &context,
            "{} has conflicting module contexts {} and {}",
            relative.display(),
            display_scope(&existing.scope),
            display_scope(&context.scope)
        );
        return Ok(());
    }
    contexts.insert(relative.clone(), context.clone());
    routes.push_back(ModuleRoute {
        relative,
        module_directory,
        context,
    });
    Ok(())
}

fn display_scope(scope: &[String]) -> String {
    if scope.is_empty() {
        "<crate root>".to_string()
    } else {
        scope.join("::")
    }
}

fn route_modules(
    items: &[Item],
    source: &Path,
    module_directory: &Path,
    context: &ModuleContext,
    available: &BTreeSet<PathBuf>,
    contexts: &mut BTreeMap<PathBuf, ModuleContext>,
    routes: &mut VecDeque<ModuleRoute>,
) -> Result<()> {
    for item in items {
        let Item::Mod(module) = item else {
            continue;
        };
        let declared_path = literal_module_path(&module.attrs, source, module.ident.span())?;
        let mut scope = context.scope.clone();
        scope.push(module.ident.to_string());
        let child = ModuleContext {
            scope,
            availability: context.availability.for_item(&module.attrs),
            package: context.package.clone(),
            crate_name: context.crate_name.clone(),
            target_root: context.target_root.clone(),
            target_kind: context.target_kind,
            language_main: false,
        };
        if let Some((_, items)) = &module.content {
            ensure!(
                declared_path.is_none(),
                "inline module {} in {} has an unsupported module path",
                module.ident,
                source.display()
            );
            route_modules(
                items,
                source,
                &module_directory.join(module.ident.to_string()),
                &child,
                available,
                contexts,
                routes,
            )?;
            continue;
        }

        let relative = if let Some(path) = declared_path {
            normalize_module_path(module_directory, &path).with_context(|| {
                format!(
                    "resolving module path for {} in {}",
                    module.ident,
                    source.display()
                )
            })?
        } else {
            let direct = module_directory.join(format!("{}.rs", module.ident));
            let nested = module_directory
                .join(module.ident.to_string())
                .join("mod.rs");
            let candidates = [direct, nested]
                .into_iter()
                .filter(|candidate| available.contains(candidate))
                .collect::<Vec<_>>();
            ensure!(
                candidates.len() != 2,
                "ambiguous module route for {} in {}",
                module.ident,
                source.display()
            );
            candidates.first().cloned().with_context(|| {
                format!(
                    "module {} in {} has no source route",
                    module.ident,
                    source.display()
                )
            })?
        };
        ensure!(
            available.contains(&relative),
            "module path {} from {} is not a copied Rust source",
            relative.display(),
            source.display()
        );
        let child_directory = module_directory_for_source(&relative)?;
        register_module_context(contexts, routes, relative, child_directory, child)?;
    }
    Ok(())
}

fn literal_module_path(
    attributes: &[Attribute],
    source: &Path,
    span: Span,
) -> Result<Option<PathBuf>> {
    let mut declared = None;
    for attribute in attributes {
        if attribute.path().is_ident("cfg_attr") && meta_contains_module_path(&attribute.meta) {
            bail!(
                "module at {}:{} uses a conditional module path",
                source.display(),
                span.start().line
            );
        }
        if !attribute.path().is_ident("path") {
            continue;
        }
        ensure!(
            declared.is_none(),
            "module at {}:{} has more than one module path",
            source.display(),
            span.start().line
        );
        let Meta::NameValue(value) = &attribute.meta else {
            bail!(
                "module path at {}:{} is not a literal",
                source.display(),
                span.start().line
            );
        };
        let Expr::Lit(expression) = &value.value else {
            bail!(
                "module path at {}:{} is not a literal",
                source.display(),
                span.start().line
            );
        };
        let Lit::Str(path) = &expression.lit else {
            bail!(
                "module path at {}:{} is not a string literal",
                source.display(),
                span.start().line
            );
        };
        declared = Some(PathBuf::from(path.value()));
    }
    Ok(declared)
}

fn meta_contains_module_path(meta: &Meta) -> bool {
    if meta.path().is_ident("path") {
        return true;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .is_ok_and(|arguments| arguments.iter().any(meta_contains_module_path))
}

fn normalize_module_path(base: &Path, declared: &Path) -> Result<PathBuf> {
    ensure!(
        !declared.as_os_str().is_empty(),
        "an empty module path is unsupported"
    );
    let mut normalized = PathBuf::new();
    for component in base.components().chain(declared.components()) {
        match component {
            std::path::Component::Normal(component) => normalized.push(component),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => ensure!(
                normalized.pop(),
                "module path {} escapes the audit root",
                declared.display()
            ),
            std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                bail!("absolute module path {} is unsupported", declared.display())
            }
        }
    }
    Ok(normalized)
}

fn module_directory_for_source(relative: &Path) -> Result<PathBuf> {
    let parent = relative
        .parent()
        .context("an out-of-line module source has no parent directory")?;
    if relative.file_name().is_some_and(|name| name == "mod.rs") {
        return Ok(parent.to_path_buf());
    }
    let stem = relative
        .file_stem()
        .context("an out-of-line module source has no file stem")?;
    Ok(parent.join(stem))
}

fn source_item_path(file: &SourceFile, symbol: &str) -> String {
    format!("{}::{symbol}", file.crate_name)
}

fn workspace_type_matches(file: &SourceFile, symbol: &str, path: &str) -> bool {
    let Some((crate_name, _)) = path.split_once("::") else {
        return false;
    };
    file.crate_name == crate_name && last_path_segment(symbol) == last_path_segment(path)
}

fn last_path_segment(path: &str) -> &str {
    path.rsplit_once("::").map_or(path, |(_, name)| name)
}

fn contains_diagnostic_marker(text: &str) -> Result<bool> {
    if bytes_contain_diagnostic_marker(text.as_bytes()) {
        return Ok(true);
    }
    let tokens = <TokenStream as core::str::FromStr>::from_str(text).map_err(|error| {
        anyhow::anyhow!(
            "tokenizing Rust source while scanning for the reserved API marker: {error}"
        )
    })?;
    token_stream_contains_diagnostic_marker(tokens)
}

fn token_stream_contains_diagnostic_marker(tokens: TokenStream) -> Result<bool> {
    for token in tokens {
        match token {
            TokenTree::Group(group) => {
                if token_stream_contains_diagnostic_marker(group.stream())? {
                    return Ok(true);
                }
            }
            TokenTree::Literal(literal) => {
                let token = TokenStream::from(TokenTree::Literal(literal));
                let literal = syn::parse2::<Lit>(token).context(
                    "decoding a Rust literal while scanning for the reserved API marker",
                )?;
                let contains = match literal {
                    Lit::Str(value) => bytes_contain_diagnostic_marker(value.value().as_bytes()),
                    Lit::ByteStr(value) => bytes_contain_diagnostic_marker(&value.value()),
                    Lit::CStr(value) => bytes_contain_diagnostic_marker(value.value().as_bytes()),
                    _ => false,
                };
                if contains {
                    return Ok(true);
                }
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {}
        }
    }
    Ok(false)
}

fn bytes_contain_diagnostic_marker(bytes: &[u8]) -> bool {
    bytes.windows(MARKER.len() + 1).any(|window| {
        window.starts_with(MARKER.as_bytes()) && window[MARKER.len()].is_ascii_digit()
    })
}

fn authority_file_contains_diagnostic_marker(path: &Path, bytes: &[u8]) -> Result<bool> {
    if bytes_contain_diagnostic_marker(bytes) {
        return Ok(true);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        ensure!(
            path.extension().is_none_or(|extension| extension != "rs"),
            "Rust compiler input {} is not UTF-8",
            path.display()
        );
        return Ok(false);
    };
    if path.extension().is_some_and(|extension| extension == "rs") {
        return contains_diagnostic_marker(text);
    }
    let Ok(tokens) = <TokenStream as core::str::FromStr>::from_str(text) else {
        return Ok(false);
    };
    token_stream_contains_diagnostic_marker(tokens)
}

fn reject_authority_diagnostic_marker(path: &Path, bytes: &[u8], label: &str) -> Result<()> {
    ensure!(
        !authority_file_contains_diagnostic_marker(path, bytes)
            .with_context(|| format!("scanning {} for the reserved API marker", path.display()))?,
        "{label} {} contains the API audit's reserved diagnostic marker",
        path.display()
    );
    Ok(())
}

fn validate_kernel_entry_source(
    relative: &Path,
    context: &ModuleContext,
    syntax: &syn::File,
) -> Result<()> {
    if context.package != crate::paths::KERNEL_PACKAGE
        || context.target_kind != CargoTargetKind::Bin
        || relative != context.target_root
        || relative != Path::new("kernel/src/main.rs")
        || !context.scope.is_empty()
    {
        return Ok(());
    }
    for item in &syntax.items {
        ensure!(
            !kernel_item_binds_wost_root(item),
            "{} rebinds the kernel entry dependency root `wost`",
            relative.display()
        );
        if let Item::Macro(item) = item
            && item.ident.is_none()
            && !exact_kernel_entry_invocation(item)
        {
            bail!(
                "{} contains an opaque root item macro that can manufacture a `wost` binding before the trusted kernel entry",
                relative.display()
            );
        }
    }
    Ok(())
}

fn validate_harness_target_source(
    relative: &Path,
    context: &ModuleContext,
    syntax: &syn::File,
) -> Result<()> {
    if matches!(
        context.target_kind,
        CargoTargetKind::Test | CargoTargetKind::Bench
    ) && relative == context.target_root
        && syntax
            .items
            .iter()
            .any(|item| matches!(item, Item::Fn(function) if function.sig.ident == "main"))
    {
        bail!(
            "Cargo {} target {} contains a source `main`, which the API audit cannot distinguish from a custom harness",
            context.target_kind.metadata_name(),
            relative.display()
        );
    }
    Ok(())
}

fn kernel_item_binds_wost_root(item: &Item) -> bool {
    match item {
        Item::Const(item) => item.ident == "wost",
        Item::Enum(item) => item.ident == "wost",
        Item::ExternCrate(item) => {
            item.rename
                .as_ref()
                .map_or(&item.ident, |(_, rename)| rename)
                == "wost"
        }
        Item::Fn(item) => item.sig.ident == "wost",
        Item::Macro(item) => item.ident.as_ref().is_some_and(|name| name == "wost"),
        Item::Mod(item) => item.ident == "wost",
        Item::Static(item) => item.ident == "wost",
        Item::Struct(item) => item.ident == "wost",
        Item::Trait(item) => item.ident == "wost",
        Item::TraitAlias(item) => item.ident == "wost",
        Item::Type(item) => item.ident == "wost",
        Item::Union(item) => item.ident == "wost",
        Item::Use(item) => use_tree_may_bind_wost(&item.tree),
        _ => false,
    }
}

fn use_tree_may_bind_wost(tree: &UseTree) -> bool {
    match tree {
        UseTree::Path(path) => use_tree_may_bind_wost(&path.tree),
        UseTree::Name(name) => name.ident == "wost",
        UseTree::Rename(rename) => rename.rename == "wost",
        UseTree::Glob(_) => true,
        UseTree::Group(group) => group.items.iter().any(use_tree_may_bind_wost),
    }
}

fn exact_kernel_entry_invocation(item: &syn::ItemMacro) -> bool {
    item.attrs.is_empty()
        && exact_path(&item.mac.path, &["wost", "entry"])
        && item.mac.parse_body::<syn::Path>().is_ok_and(|handler| {
            handler.leading_colon.is_none()
                && handler.segments.len() == 1
                && handler.segments[0].arguments.is_empty()
                && handler.segments[0].ident == "kernel_main"
        })
}

fn audited_source(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.starts_with(Path::new("wost/src")) {
        return true;
    }
    let mut components = relative.components();
    core::iter::Iterator::next(&mut components).is_some_and(|part| part.as_os_str() == "libs")
        && core::iter::Iterator::next(&mut components).is_some()
        && core::iter::Iterator::next(&mut components).is_some_and(|part| part.as_os_str() == "src")
}

impl TraitImports {
    #[cfg(test)]
    fn collect(items: &[Item]) -> Self {
        Self::collect_in_scope(items, &[])
    }

    fn collect_in_scope(items: &[Item], scope: &[String]) -> Self {
        let mut imports = Self::default();
        imports.collect_names(items, &mut scope.to_vec());
        imports.collect_items(items, &mut scope.to_vec());
        imports
    }

    fn collect_names(&mut self, items: &[Item], scope: &mut Vec<String>) {
        self.local_names.entry(scope.clone()).or_default();
        for item in items {
            let name = match item {
                Item::Enum(item) => Some(item.ident.to_string()),
                Item::Mod(item) => Some(item.ident.to_string()),
                Item::Struct(item) => Some(item.ident.to_string()),
                Item::Trait(item) => Some(item.ident.to_string()),
                Item::TraitAlias(item) => Some(item.ident.to_string()),
                Item::Type(item) => Some(item.ident.to_string()),
                Item::Union(item) => Some(item.ident.to_string()),
                _ => None,
            };
            if let Some(name) = name {
                self.local_names
                    .entry(scope.clone())
                    .or_default()
                    .insert(name);
            }
            match item {
                Item::ExternCrate(item) => {
                    let binding = item
                        .rename
                        .as_ref()
                        .map_or_else(|| item.ident.to_string(), |(_, rename)| rename.to_string());
                    let origin = match item.ident.to_string().as_str() {
                        "self" => ImportOrigin::Other,
                        "std" | "core" | "alloc" => ImportOrigin::Standard,
                        _ => ImportOrigin::Dependency,
                    };
                    self.external_names
                        .entry(scope.clone())
                        .or_default()
                        .entry(binding)
                        .and_modify(|current| {
                            if *current != origin {
                                *current = ImportOrigin::Other;
                            }
                        })
                        .or_insert(origin);
                }
                Item::Mod(item) => {
                    let Some((_, items)) = &item.content else {
                        continue;
                    };
                    scope.push(item.ident.to_string());
                    self.collect_names(items, scope);
                    scope.pop();
                }
                _ => {}
            }
        }
    }

    fn collect_items(&mut self, items: &[Item], scope: &mut Vec<String>) {
        for item in items {
            match item {
                Item::Use(item) => self.collect_tree(scope, &item.tree, &mut Vec::new()),
                Item::Mod(item) => {
                    let Some((_, items)) = &item.content else {
                        continue;
                    };
                    scope.push(item.ident.to_string());
                    self.collect_items(items, scope);
                    scope.pop();
                }
                _ => {}
            }
        }
    }

    fn collect_tree(&mut self, scope: &[String], tree: &syn::UseTree, prefix: &mut Vec<String>) {
        match tree {
            syn::UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.collect_tree(scope, &path.tree, prefix);
                prefix.pop();
            }
            syn::UseTree::Name(name) => {
                let name = name.ident.to_string();
                let (binding, source) = if name == "self" {
                    let Some(binding) = prefix.last() else {
                        return;
                    };
                    (binding.clone(), prefix.clone())
                } else {
                    let mut source = prefix.clone();
                    source.push(name.clone());
                    (name, source)
                };
                self.record(scope, binding, source);
            }
            syn::UseTree::Rename(rename) => {
                let mut source = prefix.clone();
                source.push(rename.ident.to_string());
                self.record(scope, rename.rename.to_string(), source);
            }
            syn::UseTree::Glob(_) => {
                self.globs
                    .entry(scope.to_vec())
                    .or_default()
                    .push(prefix.clone());
            }
            syn::UseTree::Group(group) => {
                for tree in &group.items {
                    self.collect_tree(scope, tree, prefix);
                }
            }
        }
    }

    fn record(&mut self, scope: &[String], binding: String, source: Vec<String>) {
        self.paths
            .entry(scope.to_vec())
            .or_default()
            .entry(binding)
            .and_modify(|current| {
                if current.as_ref() != Some(&source) {
                    *current = None;
                }
            })
            .or_insert(Some(source));
    }

    fn resolve_path(&self, scope: &[String], path: &str) -> Option<String> {
        let segments = path.split("::").map(str::to_string).collect::<Vec<_>>();
        self.resolve_segments(scope, &segments, &mut BTreeSet::new())
            .map(|segments| segments.join("::"))
    }

    fn external_trait(&self, scope: &[String], path: &str) -> bool {
        matches!(
            self.trait_origin(scope, path),
            ImportOrigin::Standard | ImportOrigin::Dependency
        )
    }

    fn trait_origin(&self, scope: &[String], path: &str) -> ImportOrigin {
        let segments = path.split("::").map(str::to_string).collect::<Vec<_>>();
        let Some(first) = segments.first() else {
            return ImportOrigin::Other;
        };
        if segments.len() > 1 {
            return self.path_origin(scope, &segments, &mut BTreeSet::new());
        }

        if let Some(source) = self
            .paths
            .get(scope)
            .and_then(|bindings| bindings.get(first))
        {
            let Some(source) = source else {
                return ImportOrigin::Other;
            };
            return self.path_origin(scope, source, &mut BTreeSet::new());
        }
        if self.local_name(scope, first)
            || self.glob_binds_local(scope, first, &mut BTreeSet::new())
        {
            return ImportOrigin::Other;
        }
        if prelude_external_trait(path) {
            ImportOrigin::Standard
        } else {
            ImportOrigin::Other
        }
    }

    fn implicit_standard_trait(&self, scope: &[String], path: &str) -> Option<ImplicitTrait> {
        if self.trait_origin(scope, path) != ImportOrigin::Standard {
            return None;
        }
        let resolved = self
            .resolve_path(scope, path)
            .unwrap_or_else(|| path.to_string());
        match resolved.as_str() {
            "Drop" | "core::ops::Drop" | "std::ops::Drop" => Some(ImplicitTrait::Drop),
            "core::alloc::GlobalAlloc" | "std::alloc::GlobalAlloc" => {
                Some(ImplicitTrait::GlobalAlloc)
            }
            _ => None,
        }
    }

    fn resolve_segments(
        &self,
        scope: &[String],
        segments: &[String],
        visited: &mut BTreeSet<String>,
    ) -> Option<Vec<String>> {
        let first = segments.first()?;
        let source = self.paths.get(scope)?.get(first)?.as_ref()?;
        let key = format!("{}::{first}", scope.join("::"));
        if !visited.insert(key.clone()) {
            return None;
        }
        let mut resolved = self
            .resolve_segments(scope, source, visited)
            .unwrap_or_else(|| source.clone());
        resolved.extend_from_slice(&segments[1..]);
        visited.remove(&key);
        Some(resolved)
    }

    fn path_origin(
        &self,
        scope: &[String],
        segments: &[String],
        visited: &mut BTreeSet<String>,
    ) -> ImportOrigin {
        let Some(first) = segments.first() else {
            return ImportOrigin::Other;
        };
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return ImportOrigin::Other;
        }
        let key = format!("{}::{first}", scope.join("::"));
        if !visited.insert(key.clone()) {
            return ImportOrigin::Other;
        }
        let origin = if let Some(source) = self
            .paths
            .get(scope)
            .and_then(|bindings| bindings.get(first))
        {
            source.as_ref().map_or(ImportOrigin::Other, |source| {
                self.path_origin(scope, source, visited)
            })
        } else if self.local_name(scope, first) {
            ImportOrigin::Other
        } else if let Some(origin) = self
            .external_names
            .get(scope)
            .and_then(|bindings| bindings.get(first))
        {
            *origin
        } else {
            import_origin(segments)
        };
        visited.remove(&key);
        origin
    }

    fn local_name(&self, scope: &[String], name: &str) -> bool {
        self.local_names
            .get(scope)
            .is_some_and(|names| names.contains(name))
    }

    fn glob_binds_local(
        &self,
        scope: &[String],
        name: &str,
        visited: &mut BTreeSet<Vec<String>>,
    ) -> bool {
        if !visited.insert(scope.to_vec()) {
            return false;
        }
        let found = self
            .globs
            .get(scope)
            .into_iter()
            .flatten()
            .filter_map(|path| self.local_scope(scope, path))
            .any(|target| {
                self.local_name(&target, name)
                    || self
                        .paths
                        .get(&target)
                        .and_then(|bindings| bindings.get(name))
                        .and_then(Option::as_ref)
                        .is_some_and(|source| {
                            self.path_origin(&target, source, &mut BTreeSet::new())
                                == ImportOrigin::Other
                        })
                    || self.glob_binds_local(&target, name, visited)
            });
        visited.remove(scope);
        found
    }

    fn local_scope(&self, scope: &[String], path: &[String]) -> Option<Vec<String>> {
        let mut path = self
            .resolve_segments(scope, path, &mut BTreeSet::new())
            .unwrap_or_else(|| path.to_vec());
        let mut target = match path.first().map(String::as_str) {
            Some("crate") => {
                path.remove(0);
                Vec::new()
            }
            Some("self") => {
                path.remove(0);
                scope.to_vec()
            }
            Some("super") => {
                let mut target = scope.to_vec();
                while path.first().is_some_and(|segment| segment == "super") {
                    target.pop()?;
                    path.remove(0);
                }
                target
            }
            Some(first) if self.local_name(scope, first) => scope.to_vec(),
            _ => return None,
        };
        target.extend(path);
        self.local_names.contains_key(&target).then_some(target)
    }
}

fn import_origin(path: &[String]) -> ImportOrigin {
    match path.first().map(String::as_str) {
        Some("std" | "core" | "alloc") => ImportOrigin::Standard,
        Some("crate" | "self" | "super") | None => ImportOrigin::Other,
        Some(_) => ImportOrigin::Dependency,
    }
}

fn prelude_external_trait(path: &str) -> bool {
    matches!(
        path,
        "AsMut"
            | "AsRef"
            | "Clone"
            | "Copy"
            | "Default"
            | "DoubleEndedIterator"
            | "Drop"
            | "Eq"
            | "ExactSizeIterator"
            | "Extend"
            | "Fn"
            | "FnMut"
            | "FnOnce"
            | "From"
            | "FromIterator"
            | "Future"
            | "Hash"
            | "Into"
            | "IntoFuture"
            | "IntoIterator"
            | "Iterator"
            | "Ord"
            | "PartialEq"
            | "PartialOrd"
            | "Send"
            | "Sized"
            | "Sync"
            | "ToOwned"
            | "TryFrom"
            | "TryInto"
            | "Unpin"
    )
}

#[derive(Default)]
struct NonCallUses {
    ranges: Vec<SourceRange>,
}

impl<'ast> Visit<'ast> for NonCallUses {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.ranges.push(range(item.self_ty.span()));
        if !item.generics.params.is_empty() {
            self.ranges.push(range(item.generics.span()));
        }
        if let Some(where_clause) = &item.generics.where_clause {
            self.ranges.push(range(where_clause.span()));
        }
        if let Some((_, path, _)) = &item.trait_ {
            self.ranges.push(range(path.span()));
        }
        visit::visit_item_impl(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.ranges.push(range(item.span()));
        visit::visit_item_use(self, item);
    }
}

#[derive(Default)]
struct MacroSyntax {
    ranges: Vec<SourceRange>,
    transparent: Vec<SourceRange>,
    paths: Vec<SourceRange>,
    unqualified_standard: bool,
}

impl<'ast> Visit<'ast> for MacroSyntax {
    fn visit_macro(&mut self, expression: &'ast syn::Macro) {
        let invocation = range(expression.span());
        self.ranges.push(invocation);
        if transparent_macro(expression, self.unqualified_standard) {
            self.transparent.push(invocation);
        }
        self.paths.push(range(expression.path.span()));
        visit::visit_macro(self, expression);
    }
}

const TRANSPARENT_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "dbg",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "eprint",
    "eprintln",
    "format",
    "format_args",
    "matches",
    "panic",
    "print",
    "println",
    "todo",
    "unreachable",
    "vec",
    "write",
    "writeln",
];

const TOKEN_GRAMMAR_STATEMENT_MACROS: &[&str] = &["asm", "naked_asm"];

struct MacroShadowPolicy<'a> {
    shadows_standard: bool,
    reserved_root_rebinding: Option<String>,
    context: &'a ModuleContext,
    relative: &'a Path,
}

impl<'ast> Visit<'ast> for MacroShadowPolicy<'_> {
    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        match &item.ident {
            Some(name) if trusted_unqualified_statement_macro_name(&name.to_string()) => {
                self.shadows_standard = true;
            }
            None if !approved_item_macro_for_shadow_policy(item, self.context, self.relative) => {
                self.shadows_standard = true;
            }
            _ => {}
        }
        visit::visit_item_macro(self, item);
    }

    fn visit_stmt_macro(&mut self, statement: &'ast syn::StmtMacro) {
        if !trusted_standard_statement_macro(&statement.mac) {
            self.shadows_standard = true;
        }
        visit::visit_stmt_macro(self, statement);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        use_tree_shadows_standard_macro(&item.tree, None, &mut self.shadows_standard);
        use_tree_reserved_root_rebinding(&item.tree, None, &mut self.reserved_root_rebinding);
        visit::visit_item_use(self, item);
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        let source = item.ident.to_string();
        let binding = item
            .rename
            .as_ref()
            .map_or_else(|| source.clone(), |(_, rename)| rename.to_string());
        if has_attribute(&item.attrs, "macro_use")
            || reserved_language_root(&binding) && !reserved_language_root(&source)
        {
            self.shadows_standard = true;
        }
        if reserved_language_root(&binding) && binding != source {
            self.reserved_root_rebinding = Some(binding);
        }
        visit::visit_item_extern_crate(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if reserved_language_root(&item.ident.to_string()) {
            self.shadows_standard = true;
            self.reserved_root_rebinding = Some(item.ident.to_string());
        }
        visit::visit_item_mod(self, item);
    }
}

fn discover_macro_policies(
    root: &Path,
    paths: &[PathBuf],
    contexts: &BTreeMap<PathBuf, ModuleContext>,
) -> Result<BTreeMap<PathBuf, bool>> {
    let mut policies: BTreeMap<PathBuf, bool> = BTreeMap::new();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .with_context(|| format!("{} is outside the audit root", path.display()))?;
        let Some(context) = contexts.get(relative) else {
            continue;
        };
        let source = std::fs::read_to_string(path)
            .with_context(|| format!("reading {} for macro resolution", relative.display()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parsing {} for macro resolution", relative.display()))?;
        let mut policy = MacroShadowPolicy {
            shadows_standard: false,
            reserved_root_rebinding: None,
            context,
            relative,
        };
        policy.visit_file(&syntax);
        ensure!(
            policy.reserved_root_rebinding.is_none(),
            "{} rebinds reserved language root `{}` outside the API audit's canonical standard-macro boundary",
            relative.display(),
            policy.reserved_root_rebinding.as_deref().unwrap_or("")
        );
        *policies.entry(context.target_root.clone()).or_default() |= policy.shadows_standard;
    }
    Ok(policies
        .into_iter()
        .map(|(target, shadows_standard)| (target, !shadows_standard))
        .collect())
}

fn approved_item_macro_for_shadow_policy(
    item: &syn::ItemMacro,
    context: &ModuleContext,
    relative: &Path,
) -> bool {
    if context.package == "wost" && exact_path(&item.mac.path, &["core", "arch", "global_asm"]) {
        return true;
    }
    if context.package != crate::paths::KERNEL_PACKAGE
        || context.target_kind != CargoTargetKind::Bin
        || relative != context.target_root
        || relative != Path::new("kernel/src/main.rs")
        || !context.scope.is_empty()
        || !item.attrs.is_empty()
        || !exact_path(&item.mac.path, &["wost", "entry"])
    {
        return false;
    }
    item.mac.parse_body::<syn::Path>().is_ok_and(|handler| {
        handler.leading_colon.is_none()
            && handler.segments.len() == 1
            && handler.segments[0].arguments.is_empty()
            && handler.segments[0].ident == "kernel_main"
    })
}

fn use_tree_shadows_standard_macro(tree: &UseTree, root: Option<&str>, shadows: &mut bool) {
    match tree {
        UseTree::Path(path) => {
            let name = path.ident.to_string();
            use_tree_shadows_standard_macro(&path.tree, root.or(Some(&name)), shadows);
        }
        UseTree::Name(name) => {
            let binding = name.ident.to_string();
            let source_root = root.unwrap_or(&binding);
            if (trusted_unqualified_statement_macro_name(&binding)
                || reserved_language_root(&binding))
                && !reserved_language_root(source_root)
            {
                *shadows = true;
            }
        }
        UseTree::Rename(rename) => {
            let binding = rename.rename.to_string();
            let source = rename.ident.to_string();
            let source_root = root.unwrap_or(&source);
            if (trusted_unqualified_statement_macro_name(&binding)
                || reserved_language_root(&binding))
                && !reserved_language_root(source_root)
            {
                *shadows = true;
            }
        }
        UseTree::Glob(_) => {
            if !root.is_some_and(|root| {
                matches!(root, "self" | "super" | "crate" | "core" | "std" | "alloc")
            }) {
                *shadows = true;
            }
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                use_tree_shadows_standard_macro(tree, root, shadows);
            }
        }
    }
}

fn use_tree_reserved_root_rebinding(
    tree: &UseTree,
    root: Option<&str>,
    rebinding: &mut Option<String>,
) {
    match tree {
        UseTree::Path(path) => {
            let name = path.ident.to_string();
            use_tree_reserved_root_rebinding(&path.tree, root.or(Some(&name)), rebinding);
        }
        UseTree::Name(name) => {
            let binding = name.ident.to_string();
            let source_root = root.unwrap_or(&binding);
            if reserved_language_root(&binding) && source_root != binding {
                *rebinding = Some(binding);
            }
        }
        UseTree::Rename(rename) => {
            let binding = rename.rename.to_string();
            let source = rename.ident.to_string();
            let source_root = root.unwrap_or(&source);
            if reserved_language_root(&binding) && source_root != binding {
                *rebinding = Some(binding);
            }
        }
        UseTree::Group(group) => {
            for tree in &group.items {
                use_tree_reserved_root_rebinding(tree, root, rebinding);
            }
        }
        UseTree::Glob(_) => {}
    }
}

fn reserved_language_root(name: &str) -> bool {
    RESERVED_LANGUAGE_ROOTS.contains(&name)
}

fn transparent_macro_name(name: &str) -> bool {
    TRANSPARENT_MACROS.contains(&name)
}

fn trusted_unqualified_statement_macro_name(name: &str) -> bool {
    transparent_macro_name(name) || TOKEN_GRAMMAR_STATEMENT_MACROS.contains(&name)
}

fn trusted_standard_statement_macro(expression: &syn::Macro) -> bool {
    let segments = expression.path.segments.iter().collect::<Vec<_>>();
    match segments.as_slice() {
        [name]
            if expression.path.leading_colon.is_none()
                && name.arguments.is_empty()
                && trusted_unqualified_statement_macro_name(&name.ident.to_string()) =>
        {
            true
        }
        [root, arch, name]
            if expression.path.leading_colon.is_some()
                && root.arguments.is_empty()
                && arch.arguments.is_empty()
                && name.arguments.is_empty()
                && matches!(root.ident.to_string().as_str(), "core" | "std")
                && arch.ident == "arch"
                && TOKEN_GRAMMAR_STATEMENT_MACROS.contains(&name.ident.to_string().as_str()) =>
        {
            true
        }
        _ => false,
    }
}

fn transparent_macro(expression: &syn::Macro, unqualified_standard: bool) -> bool {
    let segments = expression
        .path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>();
    let name = match segments.as_slice() {
        [name]
            if expression.path.leading_colon.is_none()
                && unqualified_standard
                && transparent_macro_name(name) =>
        {
            name.as_str()
        }
        _ => return false,
    };
    let mut boundary = MacroBoundary {
        unqualified_standard,
        opaque: false,
    };
    if name == "matches" {
        let Ok(arguments) = syn::parse2::<MatchesArguments>(expression.tokens.clone()) else {
            return false;
        };
        <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, &arguments.value);
        <MacroBoundary as Visit<'_>>::visit_pat(&mut boundary, &arguments.pattern);
        if let Some(guard) = &arguments.guard {
            <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, guard);
        }
    } else if name == "vec" {
        if let Ok(arguments) =
            Punctuated::<Expr, Token![,]>::parse_terminated.parse2(expression.tokens.clone())
        {
            for argument in &arguments {
                <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, argument);
            }
        } else if let Ok(arguments) = syn::parse2::<RepeatArguments>(expression.tokens.clone()) {
            <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, &arguments.value);
            <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, &arguments.count);
        } else {
            return false;
        }
    } else {
        let Ok(arguments) =
            Punctuated::<Expr, Token![,]>::parse_terminated.parse2(expression.tokens.clone())
        else {
            return false;
        };
        for argument in &arguments {
            <MacroBoundary as Visit<'_>>::visit_expr(&mut boundary, argument);
        }
    }
    !boundary.opaque
}

struct MatchesArguments {
    value: Expr,
    pattern: syn::Pat,
    guard: Option<Expr>,
}

impl Parse for MatchesArguments {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let value = <Expr as Parse>::parse(input)?;
        <Token![,] as Parse>::parse(input)?;
        let pattern = syn::Pat::parse_multi_with_leading_vert(input)?;
        let guard = if input.peek(Token![if]) {
            <Token![if] as Parse>::parse(input)?;
            Some(<Expr as Parse>::parse(input)?)
        } else {
            None
        };
        if input.peek(Token![,]) {
            <Token![,] as Parse>::parse(input)?;
        }
        ensure_syn_empty(input)?;
        Ok(Self {
            value,
            pattern,
            guard,
        })
    }
}

struct RepeatArguments {
    value: Expr,
    count: Expr,
}

impl Parse for RepeatArguments {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let value = <Expr as Parse>::parse(input)?;
        <Token![;] as Parse>::parse(input)?;
        let count = <Expr as Parse>::parse(input)?;
        ensure_syn_empty(input)?;
        Ok(Self { value, count })
    }
}

fn ensure_syn_empty(input: ParseStream<'_>) -> syn::Result<()> {
    if input.is_empty() {
        Ok(())
    } else {
        Err(input.error("unexpected macro argument tokens"))
    }
}

struct MacroBoundary {
    unqualified_standard: bool,
    opaque: bool,
}

impl<'ast> Visit<'ast> for MacroBoundary {
    fn visit_item(&mut self, _item: &'ast Item) {
        self.opaque = true;
    }

    fn visit_expr_async(&mut self, _expression: &'ast syn::ExprAsync) {
        self.opaque = true;
    }

    fn visit_expr_closure(&mut self, _expression: &'ast syn::ExprClosure) {
        self.opaque = true;
    }

    fn visit_expr_macro(&mut self, expression: &'ast syn::ExprMacro) {
        if !transparent_macro(&expression.mac, self.unqualified_standard) {
            self.opaque = true;
        }
    }

    fn visit_pat(&mut self, pattern: &'ast syn::Pat) {
        if let syn::Pat::Macro(pattern) = pattern
            && !transparent_macro(&pattern.mac, self.unqualified_standard)
        {
            self.opaque = true;
            return;
        }
        visit::visit_pat(self, pattern);
    }
}

#[derive(Default)]
struct DispatchSites {
    sites: Vec<DispatchSite>,
    generic_bounds: Vec<BTreeMap<String, BTreeSet<String>>>,
}

impl DispatchSites {
    fn push_generics(&mut self, generics: &syn::Generics, inherit: bool) {
        let mut bounds = if inherit {
            self.generic_bounds.last().cloned().unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        for parameter in &generics.params {
            let syn::GenericParam::Type(parameter) = parameter else {
                continue;
            };
            bounds.insert(
                parameter.ident.to_string(),
                direct_trait_bounds(&parameter.bounds),
            );
        }
        if let Some(where_clause) = &generics.where_clause {
            for predicate in &where_clause.predicates {
                let syn::WherePredicate::Type(predicate) = predicate else {
                    continue;
                };
                let Some(parameter) = direct_type_parameter(&predicate.bounded_ty) else {
                    continue;
                };
                let Some(bounds) = bounds.get_mut(&parameter) else {
                    continue;
                };
                bounds.extend(direct_trait_bounds(&predicate.bounds));
            }
        }
        self.generic_bounds.push(bounds);
    }

    fn pop_generics(&mut self) {
        self.generic_bounds
            .pop()
            .expect("a visited generic scope has a matching environment");
    }

    fn generic_qself_trait(&self, expression: &syn::ExprPath) -> Option<String> {
        let qself = expression.qself.as_ref()?;
        qself.as_token?;
        if qself.position == 0 || qself.position + 1 != expression.path.segments.len() {
            return None;
        }
        let parameter = direct_type_parameter(&qself.ty)?;
        let trait_path = direct_path_prefix(&expression.path, qself.position)?;
        self.generic_bounds
            .iter()
            .rev()
            .find_map(|bounds| bounds.get(&parameter))
            .is_some_and(|bounds| bounds.contains(&trait_path))
            .then_some(trait_path)
    }
}

fn direct_trait_bounds(bounds: &Punctuated<syn::TypeParamBound, Token![+]>) -> BTreeSet<String> {
    bounds
        .iter()
        .filter_map(|bound| {
            let syn::TypeParamBound::Trait(bound) = bound else {
                return None;
            };
            if bound.paren_token.is_some()
                || bound.lifetimes.is_some()
                || !matches!(bound.modifier, syn::TraitBoundModifier::None)
            {
                return None;
            }
            direct_path(&bound.path)
        })
        .collect()
}

fn qself_trait_path(expression: &syn::ExprPath) -> Option<String> {
    let qself = expression.qself.as_ref()?;
    qself.as_token?;
    if qself.position == 0 || qself.position + 1 != expression.path.segments.len() {
        return None;
    }
    Some(
        expression
            .path
            .segments
            .iter()
            .take(qself.position)
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    )
}

fn direct_type_parameter(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(ty) = ty else {
        return None;
    };
    if ty.qself.is_some() || ty.path.leading_colon.is_some() || ty.path.segments.len() != 1 {
        return None;
    }
    let parameter = ty.path.segments.first()?;
    matches!(parameter.arguments, syn::PathArguments::None).then(|| parameter.ident.to_string())
}

fn direct_path(path: &syn::Path) -> Option<String> {
    direct_path_prefix(path, path.segments.len())
}

fn direct_path_prefix(path: &syn::Path, count: usize) -> Option<String> {
    if path.leading_colon.is_some()
        || count == 0
        || path
            .segments
            .iter()
            .take(count)
            .take(count.saturating_sub(1))
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return None;
    }
    Some(
        path.segments
            .iter()
            .take(count)
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    )
}

impl<'ast> Visit<'ast> for DispatchSites {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.push_generics(&item.sig.generics, false);
        visit::visit_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.push_generics(&item.generics, false);
        visit::visit_item_impl(self, item);
        self.pop_generics();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.push_generics(&item.sig.generics, true);
        visit::visit_impl_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.push_generics(&item.generics, false);
        visit::visit_item_trait(self, item);
        self.pop_generics();
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.push_generics(&item.sig.generics, true);
        visit::visit_trait_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let Expr::Path(path) = &*expression.func
            && let Some(member) = path.path.segments.last()
        {
            let generic_qself_trait = self.generic_qself_trait(path);
            let trait_qualifier = qself_trait_path(path);
            let callee_receiver = if let Some(qself) = &path.qself {
                Some(ExactReceiver {
                    range: range(qself.ty.span()),
                    exact_nominal: exact_nominal_type_path(&qself.ty).is_some(),
                })
            } else {
                path_prefix_receiver(&path.path)
            };
            let argument_receiver = path.qself.is_none().then(|| {
                expression
                    .args
                    .first()
                    .and_then(exact_dispatch_receiver_range)
            });
            let argument_receiver = argument_receiver.flatten();
            self.sites.push(DispatchSite {
                range: range(expression.span()),
                callee_evidence: vec![range(expression.func.span())],
                self_evidence: callee_receiver
                    .iter()
                    .chain(&argument_receiver)
                    .map(|receiver| receiver.range)
                    .collect(),
                exact_self_syntax: argument_receiver.is_some_and(|receiver| receiver.exact_nominal),
                exact_callee_self_syntax: callee_receiver
                    .is_some_and(|receiver| receiver.exact_nominal),
                explicit_qself: path.qself.is_some(),
                generic_qself_trait,
                kind: DispatchKind::Call {
                    member: member.ident.to_string(),
                    trait_qualifier: path.qself.as_ref().and(trait_qualifier),
                },
            });
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let receiver = exact_dispatch_receiver_range(&expression.receiver);
        self.sites.push(DispatchSite {
            range: range(expression.span()),
            callee_evidence: vec![range(expression.method.span())],
            self_evidence: receiver.iter().map(|receiver| receiver.range).collect(),
            exact_self_syntax: receiver.is_some_and(|receiver| receiver.exact_nominal),
            exact_callee_self_syntax: false,
            explicit_qself: false,
            generic_qself_trait: None,
            kind: DispatchKind::Call {
                member: expression.method.to_string(),
                trait_qualifier: None,
            },
        });
        visit::visit_expr_method_call(self, expression);
    }
}

#[derive(Debug, Clone, Copy)]
struct ExactReceiver {
    range: SourceRange,
    exact_nominal: bool,
}

fn path_prefix_receiver(path: &syn::Path) -> Option<ExactReceiver> {
    let prefix = path.segments.len().checked_sub(1)?;
    let receiver = path.segments.iter().nth(prefix.checked_sub(1)?)?;
    Some(ExactReceiver {
        range: range(receiver.ident.span()),
        exact_nominal: path.leading_colon.is_none()
            && path
                .segments
                .iter()
                .take(prefix)
                .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
    })
}

fn exact_dispatch_receiver_range(expression: &Expr) -> Option<ExactReceiver> {
    match expression {
        Expr::Path(path) if path.qself.is_none() => {
            path.path.segments.last().map(|segment| ExactReceiver {
                range: range(segment.ident.span()),
                exact_nominal: path.path.leading_colon.is_none()
                    && path
                        .path
                        .segments
                        .iter()
                        .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
            })
        }
        Expr::Struct(structure) if structure.qself.is_none() => {
            structure.path.segments.last().map(|segment| ExactReceiver {
                range: range(segment.ident.span()),
                exact_nominal: structure.path.leading_colon.is_none()
                    && structure
                        .path
                        .segments
                        .iter()
                        .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
            })
        }
        Expr::Reference(reference) => exact_dispatch_receiver_range(&reference.expr),
        Expr::Paren(parenthesized) => exact_dispatch_receiver_range(&parenthesized.expr),
        Expr::Group(grouped) => exact_dispatch_receiver_range(&grouped.expr),
        Expr::Unary(unary) => exact_dispatch_receiver_range(&unary.expr),
        _ => None,
    }
}

#[derive(Default)]
struct ValueConstructions {
    sites: Vec<ValueConstruction>,
}

impl ValueConstructions {
    fn record(&mut self, path: &syn::Path) {
        if let Some(segment) = path.segments.last() {
            self.sites.push(ValueConstruction {
                range: range(segment.ident.span()),
            });
        }
    }
}

impl<'ast> Visit<'ast> for ValueConstructions {
    fn visit_item_static(&mut self, _item: &'ast syn::ItemStatic) {}

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if expression.qself.is_none() {
            self.record(&expression.path);
        }
        visit::visit_expr_path(self, expression);
    }

    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression.qself.is_none() {
            self.record(&expression.path);
        }
        visit::visit_expr_struct(self, expression);
    }
}

#[derive(Debug, Clone, Copy)]
struct Insertion {
    line: usize,
    column: usize,
    bytes: usize,
}

#[derive(Debug)]
struct Instrumentation {
    insertions: BTreeMap<usize, Vec<Insertion>>,
    ids: BTreeSet<usize>,
}

#[derive(Debug, Clone, Copy)]
struct MarkerObservation {
    file: usize,
    id: usize,
    source_line: usize,
    column: usize,
}

struct Collector<'a> {
    audit: bool,
    file: usize,
    relative: &'a Path,
    nodes: &'a mut Vec<Node>,
    trait_imports: &'a TraitImports,
    reexports: Vec<SourceRange>,
    unsupported_macros: Vec<String>,
    entry_specs: &'a mut Vec<EntrySpec>,
    scope: Vec<String>,
    package: &'a str,
    target_root: &'a Path,
    target_kind: CargoTargetKind,
    language_main: bool,
    lexical_item_depth: usize,
}

struct NestedItems<'collector, 'source> {
    collector: &'collector mut Collector<'source>,
    availability: Availability,
}

impl<'ast> Visit<'ast> for NestedItems<'_, '_> {
    fn visit_item(&mut self, item: &'ast Item) {
        self.collector.item(item, self.availability);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Availability {
    Production,
    Verification,
}

impl Availability {
    fn for_item(self, attributes: &[Attribute]) -> Self {
        if self == Self::Verification || verification_only(attributes) {
            Self::Verification
        } else {
            Self::Production
        }
    }

    fn is_production(self) -> bool {
        self == Self::Production
    }
}

impl Collector<'_> {
    fn item(&mut self, item: &Item, parent: Availability) {
        let availability = parent.for_item(item_attrs(item));

        match item {
            Item::Const(item) => self.record(
                "const",
                &item.ident.to_string(),
                item.span(),
                self.audit && availability.is_production() && public(&item.vis),
                true,
                RootSet::default(),
            ),
            Item::Enum(item) => {
                let report = self.audit && availability.is_production() && public(&item.vis);
                self.record(
                    "enum",
                    &item.ident.to_string(),
                    item.span(),
                    report,
                    true,
                    RootSet::default(),
                );
                self.scope.push(item.ident.to_string());
                if report {
                    for variant in &item.variants {
                        if verification_only(&variant.attrs) {
                            continue;
                        }
                        self.record(
                            "variant",
                            &variant.ident.to_string(),
                            variant.span(),
                            true,
                            true,
                            RootSet::default(),
                        );
                    }
                }
                self.scope.pop();
            }
            Item::Fn(item) => {
                let report = self.audit && availability.is_production() && public(&item.vis);
                self.record(
                    "fn",
                    &item.sig.ident.to_string(),
                    item.span(),
                    report,
                    true,
                    function_roots(
                        &item.sig.ident.to_string(),
                        &item.attrs,
                        availability,
                        self.language_main && self.scope.is_empty() && self.lexical_item_depth == 0,
                    ),
                );
                self.nested_items(&item.block, availability);
            }
            Item::ForeignMod(item) => {
                for foreign in &item.items {
                    match foreign {
                        ForeignItem::Fn(item)
                            if self.audit
                                && availability.is_production()
                                && public(&item.vis)
                                && !verification_only(&item.attrs) =>
                        {
                            self.record(
                                "foreign fn",
                                &item.sig.ident.to_string(),
                                item.span(),
                                true,
                                true,
                                RootSet::default(),
                            )
                        }
                        ForeignItem::Static(item)
                            if self.audit
                                && availability.is_production()
                                && public(&item.vis)
                                && !verification_only(&item.attrs) =>
                        {
                            self.record(
                                "foreign static",
                                &item.ident.to_string(),
                                item.span(),
                                true,
                                true,
                                RootSet::default(),
                            )
                        }
                        ForeignItem::Type(item)
                            if self.audit
                                && availability.is_production()
                                && public(&item.vis)
                                && !verification_only(&item.attrs) =>
                        {
                            self.record(
                                "foreign type",
                                &item.ident.to_string(),
                                item.span(),
                                true,
                                true,
                                RootSet::default(),
                            )
                        }
                        _ => {}
                    }
                }
            }
            Item::Impl(item) => {
                let implemented_trait = item.trait_.as_ref().map(|(_, path, _)| trait_path(path));
                let external_trait = implemented_trait
                    .as_deref()
                    .is_some_and(|path| self.trait_imports.external_trait(&self.scope, path));
                let implicit_trait = implemented_trait.as_deref().and_then(|path| {
                    self.trait_imports
                        .implicit_standard_trait(&self.scope, path)
                });
                let implementing_type = self_type_path(&item.self_ty).map(|path| {
                    let path = self
                        .trait_imports
                        .resolve_path(&self.scope, &path)
                        .unwrap_or(path);
                    if path.contains("::") || self.scope.is_empty() {
                        path
                    } else {
                        format!("{}::{path}", self.scope.join("::"))
                    }
                });
                let exact_self_type = exact_nominal_type_path(&item.self_ty).is_some();
                self.scope.push("impl".to_string());
                for member in &item.items {
                    match member {
                        ImplItem::Const(member) => {
                            let member_availability = availability.for_item(&member.attrs);
                            let mut recorded = self.new_node(
                                "associated const",
                                &member.ident.to_string(),
                                member.span(),
                                self.audit
                                    && member_availability.is_production()
                                    && public(&member.vis),
                                implemented_trait.is_none(),
                                RootSet::default(),
                            );
                            Self::mark_trait_implementation(
                                &mut recorded,
                                implemented_trait.as_deref(),
                                &member.ident.to_string(),
                                implementing_type.as_deref(),
                                exact_self_type,
                                external_trait,
                                implicit_trait,
                            );
                            self.nodes.push(recorded);
                        }
                        ImplItem::Fn(member) => {
                            let member_availability = availability.for_item(&member.attrs);
                            let mut recorded = self.new_node(
                                "method",
                                &member.sig.ident.to_string(),
                                member.span(),
                                self.audit
                                    && member_availability.is_production()
                                    && public(&member.vis),
                                implemented_trait.is_none(),
                                function_roots(
                                    &member.sig.ident.to_string(),
                                    &member.attrs,
                                    member_availability,
                                    false,
                                ),
                            );
                            recorded.implementation_owner = implementing_type.clone();
                            recorded.direct_self_construction =
                                directly_constructs_self(&member.block);
                            recorded.reviewed_drop_producer =
                                implementing_type.as_deref().and_then(|owner| {
                                    reviewed_drop_producer(
                                        self.relative,
                                        owner,
                                        &member.sig.ident.to_string(),
                                        implemented_trait.as_deref(),
                                    )
                                });
                            Self::mark_trait_implementation(
                                &mut recorded,
                                implemented_trait.as_deref(),
                                &member.sig.ident.to_string(),
                                implementing_type.as_deref(),
                                exact_self_type,
                                external_trait,
                                implicit_trait,
                            );
                            self.nodes.push(recorded);
                            self.nested_items(&member.block, member_availability);
                        }
                        ImplItem::Type(member) => {
                            let member_availability = availability.for_item(&member.attrs);
                            let mut recorded = self.new_node(
                                "associated type",
                                &member.ident.to_string(),
                                member.span(),
                                self.audit
                                    && member_availability.is_production()
                                    && public(&member.vis),
                                implemented_trait.is_none(),
                                RootSet::default(),
                            );
                            Self::mark_trait_implementation(
                                &mut recorded,
                                implemented_trait.as_deref(),
                                &member.ident.to_string(),
                                implementing_type.as_deref(),
                                exact_self_type,
                                external_trait,
                                implicit_trait,
                            );
                            self.nodes.push(recorded);
                        }
                        _ => {}
                    }
                }
                self.scope.pop();
            }
            Item::Macro(item) => {
                if let Some(ident) = &item.ident {
                    self.record(
                        "macro",
                        &ident.to_string(),
                        item.span(),
                        self.audit
                            && availability.is_production()
                            && has_attribute(&item.attrs, "macro_export"),
                        true,
                        RootSet::default(),
                    );
                } else {
                    let name = item.mac.path.segments.last().map_or_else(
                        || "<unknown>".to_string(),
                        |segment| segment.ident.to_string(),
                    );
                    let claimed_entry = exact_path(&item.mac.path, &["wost", "entry"]);
                    let runtime = self.runtime_item_macro(item);
                    self.record(
                        "item macro",
                        &name,
                        item.span(),
                        false,
                        false,
                        RootSet {
                            production: availability.is_production() && runtime,
                            verification: availability == Availability::Verification && runtime,
                        },
                    );
                    if (self.audit || claimed_entry) && !runtime {
                        self.unsupported_macros
                            .push(format!("{name} at line {}", item.span().start().line));
                    }
                }
            }
            Item::Mod(item) => {
                if let Some((_, items)) = &item.content {
                    self.scope.push(item.ident.to_string());
                    for item in items {
                        self.item(item, availability);
                    }
                    self.scope.pop();
                }
            }
            Item::Static(item) => {
                let global_allocator = has_semantic_attribute(&item.attrs, "global_allocator");
                let global_allocator_type = global_allocator
                    .then(|| self.resolve_exact_nominal_type(&item.ty))
                    .flatten();
                let mut recorded = self.new_node(
                    "static",
                    &item.ident.to_string(),
                    item.span(),
                    self.audit && availability.is_production() && public(&item.vis),
                    true,
                    static_roots(&item.attrs, availability),
                );
                recorded.global_allocator = global_allocator;
                recorded.global_allocator_type = global_allocator_type;
                self.nodes.push(recorded);
            }
            Item::Struct(item) => self.record(
                "struct",
                &item.ident.to_string(),
                item.span(),
                self.audit && availability.is_production() && public(&item.vis),
                true,
                RootSet::default(),
            ),
            Item::Trait(item) => {
                let report = self.audit && availability.is_production() && public(&item.vis);
                self.record(
                    "trait",
                    &item.ident.to_string(),
                    item.span(),
                    report,
                    true,
                    RootSet::default(),
                );
                self.scope.push(item.ident.to_string());
                for member in &item.items {
                    match member {
                        TraitItem::Const(member) => {
                            let mut recorded = self.new_node(
                                "trait const",
                                &member.ident.to_string(),
                                member.span(),
                                report && !verification_only(&member.attrs),
                                true,
                                RootSet::default(),
                            );
                            self.mark_trait_declaration(&mut recorded, &member.ident.to_string());
                            self.nodes.push(recorded);
                        }
                        TraitItem::Fn(member) => {
                            let mut recorded = self.new_node(
                                "trait method",
                                &member.sig.ident.to_string(),
                                member.span(),
                                report && !verification_only(&member.attrs),
                                true,
                                RootSet::default(),
                            );
                            self.mark_trait_declaration(
                                &mut recorded,
                                &member.sig.ident.to_string(),
                            );
                            recorded.trait_default = member.default.is_some();
                            self.nodes.push(recorded);
                            if let Some(default) = &member.default {
                                self.nested_items(default, availability.for_item(&member.attrs));
                            }
                        }
                        TraitItem::Type(member) => {
                            let mut recorded = self.new_node(
                                "trait type",
                                &member.ident.to_string(),
                                member.span(),
                                report && !verification_only(&member.attrs),
                                true,
                                RootSet::default(),
                            );
                            self.mark_trait_declaration(&mut recorded, &member.ident.to_string());
                            self.nodes.push(recorded);
                        }
                        _ => {}
                    }
                }
                self.scope.pop();
            }
            Item::Type(item) => self.record(
                "type",
                &item.ident.to_string(),
                item.span(),
                self.audit && availability.is_production() && public(&item.vis),
                true,
                RootSet::default(),
            ),
            Item::Union(item) => self.record(
                "union",
                &item.ident.to_string(),
                item.span(),
                self.audit && availability.is_production() && public(&item.vis),
                true,
                RootSet::default(),
            ),
            Item::Use(item) if public(&item.vis) => {
                self.reexports.push(range(item.span()));
            }
            _ => {}
        }
    }

    fn runtime_item_macro(&mut self, item: &syn::ItemMacro) -> bool {
        if self.audit
            && self.package == "wost"
            && exact_path(&item.mac.path, &["core", "arch", "global_asm"])
        {
            return true;
        }
        if self.package != crate::paths::KERNEL_PACKAGE
            || self.target_kind != CargoTargetKind::Bin
            || self.relative != self.target_root
            || self.relative != Path::new("kernel/src/main.rs")
            || !self.scope.is_empty()
            || self.lexical_item_depth != 0
            || !item.attrs.is_empty()
            || !exact_path(&item.mac.path, &["wost", "entry"])
        {
            return false;
        }
        let Ok(handler) = item.mac.parse_body::<syn::Path>() else {
            return false;
        };
        if handler.leading_colon.is_some()
            || handler.segments.len() != 1
            || !handler.segments[0].arguments.is_empty()
            || handler.segments[0].ident != "kernel_main"
        {
            return false;
        }
        self.entry_specs.push(EntrySpec {
            file: self.file,
            invocation: range(item.span()),
            macro_path: range(item.mac.path.span()),
            argument: range(handler.span()),
        });
        true
    }

    fn nested_items(&mut self, block: &syn::Block, availability: Availability) {
        self.lexical_item_depth += 1;
        NestedItems {
            collector: self,
            availability,
        }
        .visit_block(block);
        self.lexical_item_depth -= 1;
    }

    fn mark_trait_declaration(&self, recorded: &mut Node, member: &str) {
        let trait_path = self.scope.join("::");
        recorded.trait_route = Some(TraitRoute::Declaration(TraitMemberKey {
            trait_path,
            member: member.to_string(),
        }));
    }

    fn mark_trait_implementation(
        recorded: &mut Node,
        trait_path: Option<&str>,
        member: &str,
        self_type: Option<&str>,
        exact_self_type: bool,
        external: bool,
        implicit: Option<ImplicitTrait>,
    ) {
        let Some(trait_path) = trait_path else {
            return;
        };
        recorded.trait_route = Some(TraitRoute::Implementation {
            key: TraitMemberKey {
                trait_path: trait_path.to_string(),
                member: member.to_string(),
            },
            self_type: self_type.map(str::to_string),
            exact_self_type,
            external,
            implicit,
        });
    }

    fn resolve_nominal_type(&self, ty: &syn::Type) -> Option<String> {
        let path = self_type_path(ty)?;
        let path = self
            .trait_imports
            .resolve_path(&self.scope, &path)
            .unwrap_or(path);
        if path.contains("::") || self.scope.is_empty() {
            Some(path)
        } else {
            Some(format!("{}::{path}", self.scope.join("::")))
        }
    }

    fn resolve_exact_nominal_type(&self, ty: &syn::Type) -> Option<String> {
        exact_nominal_type_path(ty)?;
        self.resolve_nominal_type(ty)
    }

    fn new_node(
        &self,
        kind: &str,
        name: &str,
        span: Span,
        report: bool,
        instrument: bool,
        roots: RootSet,
    ) -> Node {
        let mut qualified = self.scope.clone();
        qualified.push(name.to_string());
        let qualified = qualified.join("::");
        Node {
            id: 0,
            file: self.file,
            label: format!(
                "{kind} {qualified} at {}:{}",
                self.relative.display(),
                span.start().line
            ),
            symbol: qualified,
            type_declaration: matches!(kind, "enum" | "struct" | "type" | "union"),
            type_alias: kind == "type",
            callable: matches!(kind, "fn" | "foreign fn" | "method" | "trait method"),
            trait_default: false,
            macro_declaration: kind == "macro",
            range: range(span),
            insertion: span.start(),
            code: 0,
            isolation: if report {
                match kind {
                    "variant" => Some(Isolation::Variant),
                    "trait const" | "trait method" | "trait type" => Some(Isolation::TraitMember),
                    _ => None,
                }
            } else {
                None
            },
            report,
            instrument,
            roots,
            trait_route: None,
            implementation_owner: None,
            direct_self_construction: false,
            reviewed_drop_producer: None,
            global_allocator: false,
            global_allocator_type: None,
        }
    }

    fn record(
        &mut self,
        kind: &str,
        name: &str,
        span: Span,
        report: bool,
        instrument: bool,
        roots: RootSet,
    ) {
        let node = self.new_node(kind, name, span, report, instrument, roots);
        self.nodes.push(node);
    }
}

fn trait_path(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

fn self_type_path(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(ty) = ty else {
        return None;
    };
    let mut path = trait_path(&ty.path);
    if let Some(stripped) = path.strip_prefix("crate::") {
        path = stripped.to_string();
    }
    Some(path)
}

fn exact_nominal_type_path(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    if type_path.qself.is_some()
        || type_path
            .path
            .segments
            .iter()
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return None;
    }
    let mut path = trait_path(&type_path.path);
    if let Some(stripped) = path.strip_prefix("crate::") {
        path = stripped.to_string();
    }
    Some(path)
}

fn reviewed_drop_producer(
    relative: &Path,
    owner: &str,
    member: &str,
    implemented_trait: Option<&str>,
) -> Option<usize> {
    REVIEWED_IMPLICIT_DROP_PRODUCERS
        .iter()
        .position(|producer| {
            relative == Path::new(producer.relative)
                && owner == producer.owner
                && member == producer.member
                && implemented_trait == producer.implemented_trait
        })
}

fn directly_constructs_self(block: &syn::Block) -> bool {
    #[derive(Default)]
    struct SelfConstruction {
        found: bool,
    }

    impl<'ast> Visit<'ast> for SelfConstruction {
        fn visit_item(&mut self, _item: &'ast Item) {}

        fn visit_expr_async(&mut self, _expression: &'ast syn::ExprAsync) {}

        fn visit_expr_closure(&mut self, _expression: &'ast syn::ExprClosure) {}

        fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
            if expression.qself.is_none()
                && expression.path.leading_colon.is_none()
                && expression.path.segments.len() == 1
                && expression.path.segments[0].ident == "Self"
                && expression.path.segments[0].arguments.is_empty()
            {
                self.found = true;
                return;
            }
            visit::visit_expr_path(self, expression);
        }

        fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
            if expression.qself.is_none()
                && expression.path.leading_colon.is_none()
                && expression.path.segments.len() == 1
                && expression.path.segments[0].ident == "Self"
                && expression.path.segments[0].arguments.is_empty()
            {
                self.found = true;
                return;
            }
            visit::visit_expr_struct(self, expression);
        }
    }

    let mut construction = SelfConstruction::default();
    construction.visit_block(block);
    construction.found
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn public(visibility: &Visibility) -> bool {
    matches!(visibility, Visibility::Public(_))
}

fn function_roots(
    name: &str,
    attributes: &[Attribute],
    availability: Availability,
    language_main: bool,
) -> RootSet {
    let test_harness = has_attribute(attributes, "test") || has_attribute(attributes, "bench");
    let runtime = language_main && name == "main"
        || [
            "no_mangle",
            "export_name",
            "panic_handler",
            "alloc_error_handler",
        ]
        .iter()
        .any(|name| has_semantic_attribute(attributes, name));
    RootSet {
        production: availability.is_production() && runtime && !test_harness,
        verification: test_harness || availability == Availability::Verification && runtime,
    }
}

fn static_roots(attributes: &[Attribute], availability: Availability) -> RootSet {
    let runtime = ["used", "no_mangle", "export_name", "global_allocator"]
        .iter()
        .any(|name| has_semantic_attribute(attributes, name));
    RootSet {
        production: availability.is_production() && runtime,
        verification: availability == Availability::Verification && runtime,
    }
}

fn has_semantic_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident(name)
            || attribute.path().is_ident("unsafe")
                && attribute
                    .parse_args::<Meta>()
                    .is_ok_and(|meta| meta.path().is_ident(name))
    })
}

fn exact_path(path: &syn::Path, expected: &[&str]) -> bool {
    if path.leading_colon.is_none() {
        return false;
    }
    path.segments.len() == expected.len()
        && path
            .segments
            .iter()
            .zip(expected)
            .all(|(segment, expected)| segment.ident == *expected && segment.arguments.is_empty())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Condition {
    Always,
    Never,
    Maybe,
}

impl Condition {
    fn not(self) -> Self {
        match self {
            Self::Always => Self::Never,
            Self::Never => Self::Always,
            Self::Maybe => Self::Maybe,
        }
    }

    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Never, _) | (_, Self::Never) => Self::Never,
            (Self::Always, Self::Always) => Self::Always,
            _ => Self::Maybe,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => Self::Always,
            (Self::Never, Self::Never) => Self::Never,
            _ => Self::Maybe,
        }
    }
}

fn verification_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(attribute_disables_production)
}

fn attribute_disables_production(attribute: &Attribute) -> bool {
    if attribute.path().is_ident("cfg") {
        return attribute
            .parse_args::<Meta>()
            .is_ok_and(|meta| cfg_condition(&meta) == Condition::Never);
    }
    if !attribute.path().is_ident("cfg_attr") {
        return false;
    }
    let Ok(arguments) = attribute.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return false;
    };
    let mut arguments = arguments.iter();
    let Some(predicate) = core::iter::Iterator::next(&mut arguments) else {
        return false;
    };
    cfg_condition(predicate) == Condition::Always && arguments.any(meta_disables_production)
}

fn meta_disables_production(meta: &Meta) -> bool {
    if meta.path().is_ident("cfg") {
        let Meta::List(list) = meta else {
            return false;
        };
        return list
            .parse_args::<Meta>()
            .is_ok_and(|condition| cfg_condition(&condition) == Condition::Never);
    }
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return false;
    };
    let mut arguments = arguments.iter();
    let Some(predicate) = core::iter::Iterator::next(&mut arguments) else {
        return false;
    };
    cfg_condition(predicate) == Condition::Always && arguments.any(meta_disables_production)
}

fn cfg_condition(meta: &Meta) -> Condition {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Condition::Never,
        Meta::Path(_) => Condition::Maybe,
        Meta::NameValue(value) if value.path.is_ident("feature") => match &value.value {
            Expr::Lit(expression) if matches!(&expression.lit, Lit::Str(feature) if feature.value() == "selftest") => {
                Condition::Never
            }
            _ => Condition::Maybe,
        },
        Meta::NameValue(_) => Condition::Maybe,
        Meta::List(list) if list.path.is_ident("not") => list
            .parse_args::<Meta>()
            .map_or(Condition::Maybe, |condition| {
                cfg_condition(&condition).not()
            }),
        Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let Ok(arguments) =
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return Condition::Maybe;
            };
            if list.path.is_ident("all") {
                arguments.iter().fold(Condition::Always, |value, meta| {
                    value.and(cfg_condition(meta))
                })
            } else {
                arguments.iter().fold(Condition::Never, |value, meta| {
                    value.or(cfg_condition(meta))
                })
            }
        }
        Meta::List(_) => Condition::Maybe,
    }
}

fn reject_deprecation_suppression(root: &Path) -> Result<()> {
    let mut paths = Vec::new();
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        collect_optional_rust_files(&directory, &mut paths)?;
    }
    for path in paths {
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parsing {} for lint suppression", path.display()))?;
        let mut visitor = Suppression::default();
        visitor.visit_file(&syntax);
        let token_suppression = suppression_in_tokens(
            <TokenStream as core::str::FromStr>::from_str(&source).map_err(|reason| {
                anyhow::anyhow!(
                    "tokenizing {} for lint suppression: {reason}",
                    path.display()
                )
            })?,
        );
        if let Some(at) = visitor.at.or(token_suppression) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            bail!(
                "{}:{} suppresses deprecation diagnostics required by the API audit",
                relative.display(),
                at.start().line
            );
        }
    }
    Ok(())
}

fn suppression_in_tokens(stream: TokenStream) -> Option<Span> {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token, TokenTree::Punct(punctuation) if punctuation.as_char() == '#') {
            let mut attribute = index + 1;
            if matches!(tokens.get(attribute), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '!')
            {
                attribute += 1;
            }
            if let Some(TokenTree::Group(group)) = tokens.get(attribute)
                && group.delimiter() == Delimiter::Bracket
                && attribute_may_suppress(group.stream())
            {
                return Some(token.span());
            }
        }
        if let TokenTree::Group(group) = token
            && let Some(at) = suppression_in_tokens(group.stream())
        {
            return Some(at);
        }
    }
    None
}

fn attribute_may_suppress(stream: TokenStream) -> bool {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    if matches!(tokens.first(), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '$')
    {
        return true;
    }
    let Some(TokenTree::Ident(name)) = tokens.first() else {
        return false;
    };
    let Some(TokenTree::Group(arguments)) = tokens.get(1) else {
        return false;
    };
    if arguments.delimiter() != Delimiter::Parenthesis {
        return false;
    }
    if name == "allow" || name == "expect" {
        return lint_arguments_may_suppress(arguments.stream());
    }
    if name == "cfg_attr" {
        return cfg_attr_tokens_may_suppress(arguments.stream());
    }
    false
}

fn cfg_attr_tokens_may_suppress(stream: TokenStream) -> bool {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, TokenTree::Punct(punctuation) if punctuation.as_char() == '$')
            || matches!(token, TokenTree::Ident(name)
                if (name == "allow" || name == "expect")
                    && matches!(tokens.get(index + 1), Some(TokenTree::Group(arguments))
                        if arguments.delimiter() == Delimiter::Parenthesis
                            && lint_arguments_may_suppress(arguments.stream())))
            || matches!(token, TokenTree::Group(group) if cfg_attr_tokens_may_suppress(group.stream()))
    })
}

fn lint_arguments_may_suppress(stream: TokenStream) -> bool {
    stream.into_iter().any(|token| match token {
        TokenTree::Ident(name) => name == "deprecated" || name == "warnings",
        TokenTree::Punct(punctuation) => punctuation.as_char() == '$',
        TokenTree::Group(group) => lint_arguments_may_suppress(group.stream()),
        _ => false,
    })
}

#[derive(Default)]
struct Suppression {
    at: Option<Span>,
}

impl<'ast> Visit<'ast> for Suppression {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if self.at.is_none() && suppresses_deprecation(&attribute.meta) {
            self.at = Some(attribute.span());
        }
        visit::visit_attribute(self, attribute);
    }
}

fn suppresses_deprecation(meta: &Meta) -> bool {
    if meta.path().is_ident("allow") || meta.path().is_ident("expect") {
        let Meta::List(list) = meta else {
            return false;
        };
        return list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|lints| {
                lints.iter().any(|lint| {
                    matches!(lint, Meta::Path(path) if path.is_ident("deprecated") || path.is_ident("warnings"))
                })
            });
    }
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .is_ok_and(|arguments| arguments.iter().skip(1).any(suppresses_deprecation))
}

fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}

fn range(span: Span) -> SourceRange {
    SourceRange {
        start: span.start(),
        end: span.end(),
    }
}

fn collect_rust_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in crate::paths::directory_entries(directory)? {
        let path = entry.path();
        if entry.is_dir() {
            collect_rust_files(&path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            paths.push(path);
        }
    }
    Ok(())
}

fn optional_path_is_present(path: &Path) -> Result<bool> {
    path.try_exists()
        .with_context(|| format!("checking whether {} exists", path.display()))
}

#[cfg(test)]
#[test]
fn optional_source_directory_metadata_errors_are_not_absence() {
    assert!(optional_path_is_present(Path::new("\0")).is_err());
}

fn collect_optional_rust_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    if optional_path_is_present(directory)? {
        collect_rust_files(directory, paths)?;
    }
    Ok(())
}

fn antichain_codes(count: usize) -> Vec<u64> {
    let mut width = 1usize;
    while binomial(width, width.div_ceil(2)) < count {
        width += 1;
    }
    let weight = width.div_ceil(2);
    (1u64..(1u64 << width))
        .filter(|code| code.count_ones() as usize == weight)
        .take(count)
        .collect()
}

fn binomial(n: usize, k: usize) -> usize {
    let k = k.min(n - k);
    (1..=k).fold(1usize, |value, index| value * (n + 1 - index) / index)
}

fn byte_offset(text: &str, at: LineColumn) -> Result<usize> {
    ensure!(at.line > 0, "compiler span has line zero");
    let mut line_start = 0;
    let line = text
        .split_inclusive('\n')
        .enumerate()
        .find_map(|(index, line)| {
            if index + 1 == at.line {
                Some(line)
            } else {
                line_start += line.len();
                None
            }
        })
        .with_context(|| format!("compiler span names missing line {}", at.line))?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let column = line
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(line.len()))
        .nth(at.column)
        .with_context(|| {
            format!(
                "compiler span column {} exceeds line {}",
                at.column, at.line
            )
        })?;
    let offset = line_start + column;
    ensure!(
        text.is_char_boundary(offset),
        "compiler span at {}:{} is not a UTF-8 boundary",
        at.line,
        at.column
    );
    Ok(offset)
}

#[derive(Debug, Default)]
struct Uses {
    roots: BTreeSet<usize>,
    edges: BTreeMap<usize, BTreeSet<usize>>,
    dispatches: BTreeMap<(usize, usize), DispatchEvidence>,
    constructions: BTreeMap<usize, BTreeSet<usize>>,
}

#[derive(Debug)]
struct DispatchEvidence {
    caller: usize,
    nodes: BTreeSet<usize>,
    callee_nodes: BTreeSet<usize>,
    self_nodes: BTreeSet<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DispatchResolution {
    NoCredit,
    DeclarationOnly,
    Implementation(usize),
    Failure(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CargoDiagnosticOrigin {
    package_id: String,
    target_name: String,
    target_kind: CargoTargetKind,
    target_source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkerDiagnostic {
    id: usize,
    origin: CargoDiagnosticOrigin,
    reported: String,
    line: usize,
    column: usize,
    immediate_expansion: Option<DiagnosticExpansion>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticLocation {
    reported: String,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiagnosticExpansion {
    location: DiagnosticLocation,
    terminal: bool,
    macro_decl_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootKind {
    Production,
    Verification,
    TestHarness,
}

fn validate_marker_round(
    id: usize,
    inventory: &Inventory,
    instrumentation: &Instrumentation,
) -> Result<()> {
    ensure!(
        id < inventory.nodes.len(),
        "an API diagnostic names unknown item {id}"
    );
    ensure!(
        instrumentation.ids.contains(&id),
        "an API diagnostic names item {id}, which was not instrumented in this compiler round"
    );
    Ok(())
}

fn corrected_marker_location(
    file: usize,
    source_line: usize,
    column: usize,
    instrumentation: &Instrumentation,
) -> Result<LineColumn> {
    ensure!(
        source_line > 0 && column > 0,
        "an API diagnostic has a zero line or column"
    );
    let mut column = column - 1;
    for insertion in instrumentation
        .insertions
        .get(&file)
        .into_iter()
        .flatten()
        .filter(|insertion| insertion.line == source_line)
    {
        if column >= insertion.column + insertion.bytes {
            column -= insertion.bytes;
        }
    }
    Ok(LineColumn {
        line: source_line,
        column,
    })
}

fn validate_cargo_diagnostic_origin(
    diagnostic: &MarkerDiagnostic,
    inventory: &Inventory,
    root: &Path,
    file: usize,
    attributed_source: &str,
) -> Result<()> {
    let expected = &inventory.files[file].origin;
    let expected_source =
        std::fs::canonicalize(root.join(&expected.relative)).with_context(|| {
            format!(
                "canonicalizing expected Cargo target source {}",
                expected.relative.display()
            )
        })?;
    let reported_source =
        std::fs::canonicalize(&diagnostic.origin.target_source).with_context(|| {
            format!(
                "canonicalizing reported Cargo target source {}",
                diagnostic.origin.target_source.display()
            )
        })?;
    ensure!(
        diagnostic.origin.package_id == expected.package_id
            && diagnostic.origin.target_name == expected.name
            && diagnostic.origin.target_kind == expected.kind
            && reported_source == expected_source,
        "API diagnostic for {} at {} has Cargo origin {}:{}:{}:{}, expected {}:{}:{}:{}",
        inventory
            .nodes
            .get(diagnostic.id)
            .map_or("an unknown item", |node| node.label.as_str()),
        attributed_source,
        diagnostic.origin.package_id,
        diagnostic.origin.target_name,
        diagnostic.origin.target_kind.metadata_name(),
        reported_source.display(),
        expected.package_id,
        expected.name,
        expected.kind.metadata_name(),
        expected_source.display()
    );
    Ok(())
}

fn cargo_diagnostic_origin_matches(
    diagnostic: &MarkerDiagnostic,
    inventory: &Inventory,
    root: &Path,
    file: usize,
) -> Result<bool> {
    let expected = &inventory.files[file].origin;
    let expected_source =
        std::fs::canonicalize(root.join(&expected.relative)).with_context(|| {
            format!(
                "canonicalizing expected Cargo target source {}",
                expected.relative.display()
            )
        })?;
    let reported_source =
        std::fs::canonicalize(&diagnostic.origin.target_source).with_context(|| {
            format!(
                "canonicalizing reported Cargo target source {}",
                diagnostic.origin.target_source.display()
            )
        })?;
    Ok(diagnostic.origin.package_id == expected.package_id
        && diagnostic.origin.target_name == expected.name
        && diagnostic.origin.target_kind == expected.kind
        && reported_source == expected_source)
}

impl Uses {
    fn seed_dispatches(&mut self, inventory: &Inventory) -> Result<()> {
        for (file, source) in inventory.files.iter().enumerate() {
            for (site, dispatch) in source.dispatch_sites.iter().enumerate() {
                let caller = inventory
                    .caller(file, dispatch.range.start)
                    .with_context(|| {
                        format!(
                            "dispatch at {}:{} has no classified caller",
                            source.relative.display(),
                            dispatch.range.start.line
                        )
                    })?;
                let evidence =
                    self.dispatches
                        .entry((file, site))
                        .or_insert_with(|| DispatchEvidence {
                            caller,
                            nodes: BTreeSet::new(),
                            callee_nodes: BTreeSet::new(),
                            self_nodes: BTreeSet::new(),
                        });
                ensure!(
                    evidence.caller == caller,
                    "one dispatch site was attributed to two lexical callers"
                );
            }
        }
        Ok(())
    }

    fn observe_cargo(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        diagnostics: &[MarkerDiagnostic],
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        self.seed_dispatches(inventory)?;
        for diagnostic in diagnostics {
            validate_marker_round(diagnostic.id, inventory, instrumentation)?;
            let Some(file) = inventory.file_index(root, &diagnostic.reported) else {
                let expansion = diagnostic.immediate_expansion.as_ref().with_context(|| {
                    format!(
                        "an API diagnostic for item {} came from unclassified source `{}` without an immediate expansion callsite",
                        diagnostic.id, diagnostic.reported
                    )
                })?;
                ensure!(
                    expansion.terminal,
                    "an API diagnostic for item {} has an external primary span with a nested expansion chain",
                    diagnostic.id
                );
                let expansion_file = inventory
                    .file_index(root, &expansion.location.reported)
                    .with_context(|| {
                        format!(
                            "an API diagnostic for item {} came from unclassified source `{}` and unclassified immediate expansion source `{}`",
                            diagnostic.id, diagnostic.reported, expansion.location.reported
                        )
                    })?;
                validate_cargo_diagnostic_origin(
                    diagnostic,
                    inventory,
                    root,
                    expansion_file,
                    &expansion.location.reported,
                )?;
                let at = corrected_marker_location(
                    expansion_file,
                    expansion.location.line,
                    expansion.location.column,
                    instrumentation,
                )?;
                ensure!(
                    inventory.is_opaque_macro_use(expansion_file, at),
                    "an API diagnostic for item {} has an external primary span whose immediate expansion callsite at {}:{} is not an opaque macro invocation",
                    diagnostic.id,
                    inventory.files[expansion_file].relative.display(),
                    expansion.location.line
                );
                continue;
            };
            let primary_at = corrected_marker_location(
                file,
                diagnostic.line,
                diagnostic.column,
                instrumentation,
            )?;
            let entry_caller =
                if cargo_diagnostic_origin_matches(diagnostic, inventory, root, file)? {
                    None
                } else {
                    let authorized =
                        diagnostic
                            .immediate_expansion
                            .as_ref()
                            .and_then(|expansion| {
                                let expansion_file =
                                    inventory.file_index(root, &expansion.location.reported)?;
                                let expansion_at = corrected_marker_location(
                                    expansion_file,
                                    expansion.location.line,
                                    expansion.location.column,
                                    instrumentation,
                                )
                                .ok()?;
                                inventory
                                    .exact_entry_contract_expansion_caller(
                                        file,
                                        primary_at,
                                        diagnostic.id,
                                        expansion_file,
                                        expansion_at,
                                        expansion,
                                    )
                                    .map(|caller| {
                                        (
                                            caller,
                                            expansion_file,
                                            expansion.location.reported.as_str(),
                                        )
                                    })
                            });
                    let Some((caller, expansion_file, expansion_source)) = authorized else {
                        validate_cargo_diagnostic_origin(
                            diagnostic,
                            inventory,
                            root,
                            file,
                            &diagnostic.reported,
                        )?;
                        bail!("API diagnostic origin changed while its entry route was classified");
                    };
                    validate_cargo_diagnostic_origin(
                        diagnostic,
                        inventory,
                        root,
                        expansion_file,
                        expansion_source,
                    )?;
                    Some(caller)
                };
            self.observe_marker(
                kind,
                inventory,
                MarkerObservation {
                    file,
                    id: diagnostic.id,
                    source_line: diagnostic.line,
                    column: diagnostic.column,
                },
                instrumentation,
                entry_caller,
            )?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn observe(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        output: &str,
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        self.seed_dispatches(inventory)?;
        for line in output.lines().filter(|line| line.contains(MARKER)) {
            let marker = line
                .find(MARKER)
                .context("an API diagnostic lost its marker")?;
            let marker_digits = line[marker + MARKER.len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>();
            let id = <usize as core::str::FromStr>::from_str(&marker_digits)
                .context("an API diagnostic has a malformed marker")?;
            ensure!(
                id < inventory.nodes.len(),
                "an API diagnostic names unknown item {id}"
            );
            ensure!(
                instrumentation.ids.contains(&id),
                "an API diagnostic names item {id}, which was not instrumented in this compiler round"
            );

            let location = &line[..marker];
            let warning = location
                .find(": warning:")
                .context("an API diagnostic is not in short compiler format")?;
            let mut parts = location[..warning].rsplitn(3, ':');
            let column = <usize as core::str::FromStr>::from_str(
                core::iter::Iterator::next(&mut parts)
                    .context("an API diagnostic has no column")?,
            )
            .context("an API diagnostic column is not a number")?;
            let source_line = <usize as core::str::FromStr>::from_str(
                core::iter::Iterator::next(&mut parts).context("an API diagnostic has no line")?,
            )
            .context("an API diagnostic line is not a number")?;
            let reported =
                core::iter::Iterator::next(&mut parts).context("an API diagnostic has no path")?;
            let file = inventory.file_index(root, reported).with_context(|| {
                format!(
                    "an API diagnostic for {} came from unclassified source `{reported}`",
                    inventory.nodes[id].label
                )
            })?;
            self.observe_marker(
                kind,
                inventory,
                MarkerObservation {
                    file,
                    id,
                    source_line,
                    column,
                },
                instrumentation,
                None,
            )?;
        }
        Ok(())
    }

    fn observe_marker(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        observation: MarkerObservation,
        instrumentation: &Instrumentation,
        entry_caller: Option<usize>,
    ) -> Result<()> {
        let MarkerObservation {
            file,
            id,
            source_line,
            column,
        } = observation;
        validate_marker_round(id, inventory, instrumentation)?;
        let at = corrected_marker_location(file, source_line, column, instrumentation)?;
        if inventory.is_reexport(file, at) {
            return Ok(());
        }
        if inventory.is_non_call_use(file, at) {
            return Ok(());
        }
        let entry_caller =
            entry_caller.or_else(|| inventory.exact_entry_invocation_caller(file, at, id));
        let direct_macro_call = inventory.is_direct_macro_call(file, at, id);
        if entry_caller.is_none() && !direct_macro_call && inventory.is_opaque_macro_use(file, at) {
            return Ok(());
        }
        let caller = entry_caller
            .or_else(|| inventory.caller(file, at))
            .with_context(|| {
                format!(
                    "an API diagnostic for {} at {}:{} has no classified caller",
                    inventory.nodes[id].label,
                    inventory.files[file].relative.display(),
                    source_line
                )
            })?;
        if inventory.is_value_construction(file, at, id) {
            self.constructions.entry(caller).or_default().insert(id);
        }
        if let Some(site) = inventory.dispatch_site(file, at) {
            let evidence =
                self.dispatches
                    .entry((file, site))
                    .or_insert_with(|| DispatchEvidence {
                        caller,
                        nodes: BTreeSet::new(),
                        callee_nodes: BTreeSet::new(),
                        self_nodes: BTreeSet::new(),
                    });
            ensure!(
                evidence.caller == caller,
                "one dispatch site was attributed to two lexical callers"
            );
            evidence.nodes.insert(id);
            let site = &inventory.files[file].dispatch_sites[site];
            if site.callee_evidence.iter().any(|range| range.contains(at)) {
                evidence.callee_nodes.insert(id);
            }
            if site.self_evidence.iter().any(|range| range.contains(at)) {
                evidence.self_nodes.insert(id);
            }
        }
        ensure!(
            !inventory.unresolved_trait_implementations.contains(&caller),
            "an API diagnostic for {} originates in unresolved trait implementation {}",
            inventory.nodes[id].label,
            inventory.nodes[caller].label
        );
        if caller != id {
            self.edges.entry(caller).or_default().insert(id);
        }
        if inventory.nodes[caller].roots.contains(kind) {
            self.roots.insert(caller);
        }
        Ok(())
    }

    fn declaration_only_generic_dispatch(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let Some(trait_path) = site.generic_qself_trait.as_deref() else {
            return false;
        };
        if self.exact_self_type(inventory, site, evidence).is_some()
            || evidence.callee_nodes.iter().any(|node| {
                matches!(
                    inventory.nodes[*node].trait_route,
                    Some(TraitRoute::Implementation { .. })
                )
            })
        {
            return false;
        }
        let DispatchKind::Call { member, .. } = &site.kind;
        let declarations = evidence.callee_nodes.iter().filter(|node| {
            let Some(TraitRoute::Declaration(key)) = &inventory.nodes[**node].trait_route else {
                return false;
            };
            key.member == *member
                && (key.trait_path == trait_path
                    || !trait_path.contains("::")
                        && last_path_segment(&key.trait_path) == trait_path)
        });
        declarations.take(2).count() == 1
    }

    fn exact_self_type(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> Option<usize> {
        let callee_self = evidence
            .self_nodes
            .intersection(&evidence.callee_nodes)
            .copied()
            .collect::<BTreeSet<_>>();
        let mut preferred = callee_self.iter().filter(|node| {
            inventory.nodes[**node].type_declaration && !inventory.nodes[**node].type_alias
        });
        if let Some(exact) = preferred.next().copied() {
            return (site.exact_callee_self_syntax && preferred.next().is_none()).then_some(exact);
        }
        if !site.exact_self_syntax {
            return None;
        }
        let mut types = evidence.self_nodes.iter().filter(|node| {
            inventory.nodes[**node].type_declaration && !inventory.nodes[**node].type_alias
        });
        let exact = types.next().copied()?;
        types.next().is_none().then_some(exact)
    }

    fn selected_non_trait_callable(
        &self,
        inventory: &Inventory,
        evidence: &DispatchEvidence,
    ) -> bool {
        let selected = evidence.callee_nodes.iter().any(|node| {
            inventory.nodes[*node].callable && inventory.nodes[*node].trait_route.is_none()
        });
        let trait_callable = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Declaration(_) | TraitRoute::Implementation { .. })
            )
        });
        selected && !trait_callable
    }

    fn local_default_dispatch(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let Some(self_type) = self.exact_self_type(inventory, site, evidence) else {
            return false;
        };
        if evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Implementation { .. })
            )
        }) {
            return false;
        }
        let DispatchKind::Call { member, .. } = &site.kind;
        let mut declarations = evidence.callee_nodes.iter().filter(|node| {
            matches!(
                &inventory.nodes[**node].trait_route,
                Some(TraitRoute::Declaration(key)) if key.member == *member
            )
        });
        let Some(declaration) = declarations.next().copied() else {
            return false;
        };
        if declarations.next().is_some() || !inventory.nodes[declaration].trait_default {
            return false;
        }
        let Some(TraitRoute::Declaration(declaration_key)) =
            &inventory.nodes[declaration].trait_route
        else {
            unreachable!("a selected declaration retains its route")
        };
        let applicable = inventory.trait_activations.iter().any(|activation| {
            activation.declaration == Some(declaration) && activation.self_type == self_type
        });
        let unresolved = inventory
            .unresolved_trait_implementations
            .iter()
            .any(|implementation| {
                matches!(
                    &inventory.nodes[*implementation].trait_route,
                    Some(TraitRoute::Implementation { key, external: false, .. })
                        if key == declaration_key || trait_member_matches(declaration_key, key)
                )
            });
        !applicable && !unresolved
    }

    fn dispatch_matches<'a>(
        &self,
        inventory: &'a Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> Vec<&'a TraitActivation> {
        let exact_self_type = self.exact_self_type(inventory, site, evidence);
        let has_trait_declaration_evidence = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Declaration(_))
            )
        });
        inventory
            .trait_activations
            .iter()
            .filter(|activation| {
                activation.matches(
                    site,
                    &evidence.callee_nodes,
                    exact_self_type,
                    has_trait_declaration_evidence,
                )
            })
            .collect()
    }

    fn dispatch_requires_resolution(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let self_type = self.exact_self_type(inventory, site, evidence);
        let declaration_keys = evidence
            .callee_nodes
            .iter()
            .filter_map(|node| match &inventory.nodes[*node].trait_route {
                Some(TraitRoute::Declaration(key)) => Some(key),
                _ => None,
            })
            .collect::<Vec<_>>();
        let local_implementation = !declaration_keys.is_empty()
            && (inventory.trait_activations.iter().any(|activation| {
                activation
                    .declaration
                    .is_some_and(|declaration| evidence.callee_nodes.contains(&declaration))
            }) || inventory
                .unresolved_trait_implementations
                .iter()
                .any(|implementation| {
                    matches!(
                        &inventory.nodes[*implementation].trait_route,
                        Some(TraitRoute::Implementation { key, external: false, .. })
                            if declaration_keys.iter().any(|declaration| {
                                declaration == &key || trait_member_matches(declaration, key)
                            })
                    )
                }));
        let external_implementation = declaration_keys.is_empty()
            && inventory.trait_activations.iter().any(|activation| {
                activation.declaration.is_none()
                    && activation.call_compatible(site, &evidence.callee_nodes)
                    && (evidence.callee_nodes.contains(&activation.implementation)
                        || self_type.is_some_and(|self_type| {
                            self_type == activation.self_type
                                && matches!(
                                    &site.kind,
                                    DispatchKind::Call {
                                        trait_qualifier: Some(_),
                                        ..
                                    }
                                )
                        }))
            });
        let direct_implementation = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Implementation { .. })
            )
        });
        let unqualified_external_self = self_type.is_some_and(|self_type| {
            matches!(
                &site.kind,
                DispatchKind::Call {
                    trait_qualifier: None,
                    ..
                }
            ) && inventory.trait_activations.iter().any(|activation| {
                activation.declaration.is_none()
                    && activation.self_type == self_type
                    && activation.call_compatible(site, &evidence.callee_nodes)
            })
        });
        let unsupported_explicit_qself = site.explicit_qself
            && site.generic_qself_trait.is_none()
            && self_type.is_none()
            && !declaration_keys.is_empty();
        direct_implementation
            || local_implementation
            || external_implementation
            || unqualified_external_self
            || unsupported_explicit_qself
    }

    fn dispatch_resolution(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> DispatchResolution {
        if self.declaration_only_generic_dispatch(inventory, site, evidence)
            || self.local_default_dispatch(inventory, site, evidence)
        {
            return DispatchResolution::DeclarationOnly;
        }
        if self.selected_non_trait_callable(inventory, evidence) {
            return DispatchResolution::NoCredit;
        }
        let matching = self.dispatch_matches(inventory, site, evidence);
        if !self.dispatch_requires_resolution(inventory, site, evidence) {
            return DispatchResolution::NoCredit;
        }
        match matching.as_slice() {
            [activation] => DispatchResolution::Implementation(activation.implementation),
            _ => DispatchResolution::Failure(matching.len()),
        }
    }

    fn live(&self, inventory: &Inventory) -> BTreeSet<usize> {
        let mut live = self.roots.clone();
        let mut pending: VecDeque<usize> = live.iter().copied().collect();
        loop {
            while let Some(caller) = pending.pop_front() {
                for &used in self.edges.get(&caller).into_iter().flatten() {
                    if live.insert(used) {
                        pending.push_back(used);
                    }
                }
            }
            let mut activated = false;
            for (&(file, site), evidence) in &self.dispatches {
                if !live.contains(&evidence.caller) {
                    continue;
                }
                let site = &inventory.files[file].dispatch_sites[site];
                if let DispatchResolution::Implementation(implementation) =
                    self.dispatch_resolution(inventory, site, evidence)
                    && live.insert(implementation)
                {
                    pending.push_back(implementation);
                    activated = true;
                }
            }
            for activation in &inventory.implicit_trait_activations {
                let triggered = match activation.trigger {
                    ImplicitTraitTrigger::ConstructedType(ty) => self
                        .constructions
                        .iter()
                        .any(|(caller, types)| live.contains(caller) && types.contains(&ty)),
                    ImplicitTraitTrigger::LiveNode(node) => live.contains(&node),
                };
                if triggered && live.insert(activation.implementation) {
                    pending.push_back(activation.implementation);
                    activated = true;
                }
            }
            if !activated {
                break;
            }
        }
        live
    }

    fn dispatch_failures(&self, inventory: &Inventory) -> BTreeSet<DispatchFailure> {
        let live = self.live(inventory);
        let mut failures = BTreeSet::new();
        for (&(file, site), evidence) in &self.dispatches {
            if !live.contains(&evidence.caller) {
                continue;
            }
            let dispatch = &inventory.files[file].dispatch_sites[site];
            if let DispatchResolution::Failure(implementations) =
                self.dispatch_resolution(inventory, dispatch, evidence)
            {
                failures.insert(DispatchFailure {
                    file,
                    site,
                    implementations,
                });
            }
        }
        failures
    }

    #[cfg(test)]
    fn validate_dispatches(&self, inventory: &Inventory) -> Result<()> {
        let failures = self.dispatch_failures(inventory);
        ensure!(
            failures.is_empty(),
            "{}",
            failures
                .iter()
                .map(|failure| dispatch_failure(inventory, *failure))
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(())
    }

    #[cfg(test)]
    fn dead<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Node> {
        let live = self.live(inventory);
        inventory
            .nodes
            .iter()
            .filter(|node| node.report && !live.contains(&node.id))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct DispatchFailure {
    file: usize,
    site: usize,
    implementations: usize,
}

fn dispatch_failure(inventory: &Inventory, failure: DispatchFailure) -> String {
    let dispatch = &inventory.files[failure.file].dispatch_sites[failure.site];
    format!(
        "live dispatch `{}` at {}:{} resolves to {} audited trait implementations",
        match &dispatch.kind {
            DispatchKind::Call { member, .. } => member,
        },
        inventory.files[failure.file].relative.display(),
        dispatch.range.start.line,
        failure.implementations
    )
}

#[derive(Debug, Default)]
struct GraphUses {
    graphs: Vec<Uses>,
}

impl GraphUses {
    fn observe_cargo(
        &mut self,
        graph: usize,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        diagnostics: &[MarkerDiagnostic],
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        if self.graphs.len() <= graph {
            self.graphs.resize_with(graph + 1, Uses::default);
        }
        self.graphs[graph].observe_cargo(kind, inventory, root, diagnostics, instrumentation)
    }

    #[cfg(test)]
    fn observe(
        &mut self,
        graph: usize,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        output: &str,
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        if self.graphs.len() <= graph {
            self.graphs.resize_with(graph + 1, Uses::default);
        }
        self.graphs[graph].observe(kind, inventory, root, output, instrumentation)
    }

    fn dead<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Node> {
        let live = self
            .graphs
            .iter()
            .flat_map(|graph| graph.live(inventory))
            .collect::<BTreeSet<_>>();
        inventory
            .nodes
            .iter()
            .filter(|node| node.report && !live.contains(&node.id))
            .collect()
    }

    fn validate_dispatches(&self, inventory: &Inventory) -> Result<()> {
        let mut failures: BTreeMap<DispatchFailure, BTreeSet<usize>> = BTreeMap::new();
        for (graph_index, graph) in self.graphs.iter().enumerate() {
            for failure in graph.dispatch_failures(inventory) {
                failures.entry(failure).or_default().insert(graph_index);
            }
        }
        ensure!(
            failures.is_empty(),
            "{}",
            failures
                .iter()
                .map(|(failure, graphs)| {
                    format!(
                        "{}; compiler graphs {}",
                        dispatch_failure(inventory, *failure),
                        graphs
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(())
    }
}

struct AuditWorkspace {
    _lock: File,
    source_root: PathBuf,
    root: PathBuf,
    build: PathBuf,
    cargo_home: PathBuf,
    registry_source: PathBuf,
    cargo_inputs: BTreeMap<PathBuf, [u8; 32]>,
    execution_root: PathBuf,
    source: BTreeMap<PathBuf, [u8; 32]>,
}

impl AuditWorkspace {
    fn create(paths: &Paths) -> Result<Self> {
        let source_root = crate::mutation_workspace::canonical_existing_real_directory(
            paths.root(),
            "API audit source repository",
        )?;
        let target = crate::mutation_workspace::prepare_real_directory_tree(
            paths.target_dir(),
            "API audit target directory",
        )?;
        let state = crate::mutation_workspace::prepare_real_child(
            &target,
            &target.join("api-audit"),
            "API audit state directory",
        )?;
        let lock = open_audit_lock(&state)?;

        let root = reset_audit_directory(&state, "workspace")?;
        let build = reset_audit_directory(&state, "build")?;
        let cargo = reset_audit_directory(&state, "cargo")?;
        for file in COPY_FILES {
            copy_file(&source_root.join(file), &root.join(file))?;
        }
        for directory in COPY_DIRECTORIES {
            copy_tree(&source_root.join(directory), &root.join(directory))?;
        }
        let cargo_home = crate::mutation_workspace::prepare_real_child(
            &cargo,
            &cargo.join("home"),
            "API audit private Cargo home",
        )?;
        let registry_source = crate::mutation_workspace::prepare_real_child(
            &cargo,
            &cargo.join("registry-source"),
            "API audit locked registry source",
        )?;
        let config = cargo_home.join("config.toml");
        let mut config_options = OpenOptions::new();
        OpenOptions::create_new(&mut config_options, true);
        OpenOptions::write(&mut config_options, true);
        let mut config_file = OpenOptions::open(&config_options, &config)
            .with_context(|| format!("creating {}", config.display()))?;
        std::io::Write::write_all(&mut config_file, b"\n")
            .with_context(|| format!("writing {}", config.display()))?;
        crate::mutation_workspace::require_direct_file(
            &cargo_home,
            &config,
            "API audit private Cargo configuration",
        )?;
        let execution_root = cargo_execution_root(&source_root)?;
        let compiler = compiler_identity(&source_root)?;
        let copied_source = source_snapshot(&root)?;
        materialize_locked_registry_source(
            &root,
            &cargo_home,
            &registry_source,
            &execution_root,
            &compiler,
        )?;
        ensure!(
            source_snapshot(&root)? == copied_source,
            "Cargo changed the API audit workspace while materializing registry archives"
        );
        let cargo_inputs = cargo_input_snapshot(&cargo, &cargo_home, &registry_source)?;
        let source = source_snapshot(&root)?;

        Ok(Self {
            _lock: lock,
            source_root,
            root,
            build,
            cargo_home,
            registry_source,
            cargo_inputs,
            execution_root,
            source,
        })
    }

    fn require_source_unchanged(&self, source_root: &Path) -> Result<()> {
        let current = source_snapshot(source_root)?;
        if current == self.source {
            return Ok(());
        }
        let changed = self
            .source
            .keys()
            .chain(current.keys())
            .find(|path| self.source.get(*path) != current.get(*path))
            .context("the API audit source manifest changed without naming a file")?;
        bail!(
            "{} changed while the public API audit was running",
            changed.display()
        )
    }

    fn require_copy_unchanged(&self, expected: &BTreeMap<PathBuf, [u8; 32]>) -> Result<()> {
        let current = source_snapshot(&self.root)?;
        if current == *expected {
            return Ok(());
        }
        let changed = expected
            .keys()
            .chain(current.keys())
            .find(|path| expected.get(*path) != current.get(*path))
            .context("the API audit copy changed without naming a file")?;
        bail!(
            "{} changed inside the API audit copy during compilation",
            changed.display()
        )
    }

    fn require_cargo_inputs_unchanged(&self) -> Result<()> {
        let cargo = self
            .cargo_home
            .parent()
            .context("private Cargo home has no audit root")?;
        let current = cargo_input_snapshot(cargo, &self.cargo_home, &self.registry_source)?;
        ensure!(
            current == self.cargo_inputs,
            "the API audit's private Cargo configuration or locked registry source changed"
        );
        Ok(())
    }
}

fn open_audit_lock(state: &Path) -> Result<File> {
    let lock_path = state.join("LOCK");
    crate::mutation_workspace::require_direct_file_if_present(state, &lock_path, "API audit lock")?;
    let mut lock_options = OpenOptions::new();
    OpenOptions::create(&mut lock_options, true);
    OpenOptions::truncate(&mut lock_options, false);
    OpenOptions::read(&mut lock_options, true);
    OpenOptions::write(&mut lock_options, true);
    let lock = OpenOptions::open(&lock_options, &lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    crate::mutation_workspace::require_direct_file(state, &lock_path, "API audit lock")?;
    lock.try_lock()
        .context("another public API audit is already running")?;
    Ok(lock)
}

fn reset_audit_directory(state: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        matches!(name, "workspace" | "build" | "cargo"),
        "refusing to reset unknown API audit directory `{name}`"
    );
    let directory = state.join(name);
    ensure!(
        directory.parent() == Some(state),
        "API audit directory escaped its state root"
    );
    match std::fs::symlink_metadata(&directory) {
        Ok(_) => {
            let canonical = crate::mutation_workspace::canonical_existing_real_directory(
                &directory,
                "API audit working directory",
            )?;
            ensure!(
                canonical.parent() == Some(state),
                "API audit working directory escaped its canonical state root"
            );
            std::fs::remove_dir_all(&canonical)
                .with_context(|| format!("removing stale {}", canonical.display()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("inspecting {}", directory.display()));
        }
    }
    crate::mutation_workspace::prepare_real_child(state, &directory, "API audit working directory")
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(
        source,
        "API audit source directory",
    )?;
    let parent = destination
        .parent()
        .context("API audit copy directory has no parent")?;
    crate::mutation_workspace::prepare_real_child(parent, destination, "API audit copy directory")?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("reading {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", source.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            copy_file(&entry.path(), &target)?;
        } else {
            bail!(
                "API audit source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    let source_parent = source
        .parent()
        .context("API audit source file has no parent")?;
    crate::mutation_workspace::canonical_existing_real_directory(
        source_parent,
        "API audit source-file parent",
    )?;
    crate::mutation_workspace::require_direct_file(source_parent, source, "API audit source file")?;
    let parent = destination
        .parent()
        .context("API audit copied file has no parent")?;
    crate::mutation_workspace::canonical_existing_real_directory(
        parent,
        "API audit copied-file parent",
    )?;
    let mut input = File::open(source)
        .with_context(|| format!("opening API audit source {}", source.display()))?;
    let mut output_options = OpenOptions::new();
    OpenOptions::create_new(&mut output_options, true);
    OpenOptions::write(&mut output_options, true);
    let mut output = OpenOptions::open(&output_options, destination)
        .with_context(|| format!("creating API audit copy {}", destination.display()))?;
    std::io::copy(&mut input, &mut output).with_context(|| {
        format!(
            "copying API audit source {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    std::io::Write::flush(&mut output)
        .with_context(|| format!("flushing API audit copy {}", destination.display()))?;
    crate::mutation_workspace::require_direct_file(parent, destination, "API audit copied file")?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LockedRegistryPackage {
    name: String,
    version: String,
    checksum: String,
}

fn locked_registry_packages(lock: &Path) -> Result<Vec<LockedRegistryPackage>> {
    let text =
        std::fs::read_to_string(lock).with_context(|| format!("reading {}", lock.display()))?;
    ensure!(
        text.lines().any(|line| line == "version = 4"),
        "API audit requires Cargo.lock format 4"
    );
    let mut packages = Vec::new();
    for block in text.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        let mut source = None;
        let mut checksum = None;
        for line in block.lines().map(str::trim) {
            let value = |key: &str| {
                line.strip_prefix(key)
                    .and_then(|rest| rest.strip_prefix(" = \""))
                    .and_then(|rest| rest.strip_suffix('"'))
            };
            if let Some(value) = value("name") {
                name = Some(value.to_string());
            } else if let Some(value) = value("version") {
                version = Some(value.to_string());
            } else if let Some(value) = value("source") {
                source = Some(value.to_string());
            } else if let Some(value) = value("checksum") {
                checksum = Some(value.to_string());
            }
        }
        let Some(source) = source else {
            continue;
        };
        ensure!(
            source == "registry+https://github.com/rust-lang/crates.io-index",
            "API audit has no private source contract for `{source}`"
        );
        let name = name.context("locked registry package has no name")?;
        let version = version.context("locked registry package has no version")?;
        let checksum = checksum.context("locked registry package has no checksum")?;
        for (label, value) in [("name", &name), ("version", &version)] {
            ensure!(
                !value.is_empty()
                    && value.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+')
                    }),
                "locked registry package has unsafe {label} `{value}`"
            );
        }
        ensure!(
            checksum.len() == 64
                && checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
            "locked registry package {name} {version} has malformed checksum"
        );
        packages.push(LockedRegistryPackage {
            name,
            version,
            checksum,
        });
    }
    packages.sort();
    packages.dedup();
    ensure!(!packages.is_empty(), "Cargo.lock has no registry packages");
    Ok(packages)
}

fn ambient_cargo_home() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("CARGO_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .context("the API audit cannot locate Cargo's registry source cache")?;
    Ok(PathBuf::from(home).join(".cargo"))
}

fn materialize_locked_registry_source(
    workspace: &Path,
    cargo_home: &Path,
    destination: &Path,
    execution_root: &Path,
    compiler: &CompilerIdentity,
) -> Result<()> {
    let packages = locked_registry_packages(&workspace.join("Cargo.lock"))?;
    let ambient = ambient_cargo_home()?.join("registry");
    let caches = registry_roots(&ambient.join("cache"), "Cargo registry archive cache")?;
    for package in &packages {
        let archive_name = format!("{}-{}.crate", package.name, package.version);
        let matches = caches
            .iter()
            .map(|root| root.join(&archive_name))
            .filter(|path| path.is_file())
            .collect::<Vec<_>>();
        let [archive] = matches.as_slice() else {
            bail!(
                "locked registry package {} {} has {} cached archives",
                package.name,
                package.version,
                matches.len()
            );
        };
        let bytes = std::fs::read(archive)
            .with_context(|| format!("reading registry archive {}", archive.display()))?;
        ensure!(
            crate::sha256::hex(&crate::sha256::digest(&bytes)) == package.checksum,
            "cached registry archive {} does not match Cargo.lock",
            archive.display()
        );
        let cache_name = archive
            .parent()
            .and_then(Path::file_name)
            .context("registry archive cache has no source identity")?;
        let private_cache = prepare_relative_directory(
            cargo_home,
            Path::new("registry").join("cache").join(cache_name),
            "API audit private registry archive cache",
        )?;
        copy_file(archive, &private_cache.join(&archive_name))?;

        let index = ambient.join("index").join(cache_name);
        let private_index = prepare_relative_directory(
            cargo_home,
            Path::new("registry").join("index").join(cache_name),
            "API audit private registry index",
        )?;
        if !private_index.join("config.json").exists() {
            copy_file(
                &index.join("config.json"),
                &private_index.join("config.json"),
            )?;
        }
        let entry = registry_index_entry(&package.name);
        let private_parent = prepare_relative_directory(
            &private_index,
            Path::new(".cache").join(entry.parent().unwrap_or(Path::new(""))),
            "API audit private registry index entry parent",
        )?;
        let private_entry = private_parent.join(
            entry
                .file_name()
                .context("registry index entry has no name")?,
        );
        if !private_entry.exists() {
            copy_file(&index.join(".cache").join(&entry), &private_entry)?;
        }
    }

    let result = base_cargo_command(execution_root, cargo_home, compiler)
        .args(["vendor", "--offline", "--locked", "--versioned-dirs"])
        .arg(destination)
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .output()
        .context("materializing checksum-verified registry archives for the API audit")?;
    ensure!(
        result.status.success(),
        "materializing the API audit registry source failed ({}):\n{}{}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    compiler.require_unchanged()?;
    validate_materialized_registry_source(destination, &packages)?;
    reset_materialization_cargo_home(cargo_home)?;
    Ok(())
}

fn reset_materialization_cargo_home(cargo_home: &Path) -> Result<()> {
    let parent = cargo_home
        .parent()
        .context("private Cargo home has no audit parent")?;
    let canonical_parent = crate::mutation_workspace::canonical_existing_real_directory(
        parent,
        "API audit Cargo-state root",
    )?;
    let canonical_home = crate::mutation_workspace::canonical_existing_real_directory(
        cargo_home,
        "API audit materialization Cargo home",
    )?;
    ensure!(
        canonical_home.parent() == Some(canonical_parent.as_path()),
        "private Cargo home escaped its audit parent"
    );
    std::fs::remove_dir_all(&canonical_home)
        .with_context(|| format!("removing bootstrap Cargo home {}", canonical_home.display()))?;
    let cargo_home = crate::mutation_workspace::prepare_real_child(
        &canonical_parent,
        &canonical_parent.join("home"),
        "API audit private Cargo home",
    )?;
    let config = cargo_home.join("config.toml");
    std::fs::write(&config, b"\n").with_context(|| format!("writing {}", config.display()))?;
    crate::mutation_workspace::require_direct_file(
        &cargo_home,
        &config,
        "API audit private Cargo configuration",
    )?;
    Ok(())
}

fn registry_roots(root: &Path, label: &str) -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry.with_context(|| format!("reading an entry in {}", root.display()))?;
        if entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?
            .is_dir()
        {
            roots.push(entry.path());
        }
    }
    ensure!(!roots.is_empty(), "{label} is empty at {}", root.display());
    Ok(roots)
}

fn registry_index_entry(name: &str) -> PathBuf {
    match name.len() {
        1 => PathBuf::from("1").join(name),
        2 => PathBuf::from("2").join(name),
        3 => PathBuf::from("3").join(&name[..1]).join(name),
        _ => PathBuf::from(&name[..2]).join(&name[2..4]).join(name),
    }
}

fn prepare_relative_directory(root: &Path, relative: PathBuf, label: &str) -> Result<PathBuf> {
    let mut parent = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            bail!("{label} has unsafe relative path {}", relative.display());
        };
        let child = parent.join(name);
        parent = crate::mutation_workspace::prepare_real_child(&parent, &child, label)?;
    }
    Ok(parent)
}

fn validate_materialized_registry_source(
    source: &Path,
    packages: &[LockedRegistryPackage],
) -> Result<()> {
    let expected = packages
        .iter()
        .map(|package| format!("{}-{}", package.name, package.version))
        .collect::<BTreeSet<_>>();
    let mut actual = BTreeSet::new();
    for entry in std::fs::read_dir(source)
        .with_context(|| format!("reading materialized registry source {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", source.display()))?;
        ensure!(
            entry
                .file_type()
                .with_context(|| format!("reading the type of {}", entry.path().display()))?
                .is_dir(),
            "materialized registry source {} is not a directory",
            entry.path().display()
        );
        actual.insert(entry.file_name().to_string_lossy().into_owned());
    }
    ensure!(
        actual == expected,
        "materialized registry source differs from the exact Cargo.lock package set"
    );
    for package in packages {
        validate_materialized_registry_package(
            &source.join(format!("{}-{}", package.name, package.version)),
            package,
        )?;
    }
    Ok(())
}

fn validate_materialized_registry_package(
    directory: &Path,
    package: &LockedRegistryPackage,
) -> Result<()> {
    let checksum_path = directory.join(".cargo-checksum.json");
    let checksum_bytes = std::fs::read(&checksum_path)
        .with_context(|| format!("reading {}", checksum_path.display()))?;
    let checksum_json = crate::json::parse_json(
        &checksum_bytes,
        "materialized registry package checksum metadata",
    )?;
    let checksum_object = crate::json::json_object(
        &checksum_json,
        "materialized registry package checksum metadata",
    )?;
    crate::json::require_json_fields(
        checksum_object,
        &["$comment", "files", "package"],
        "materialized registry package checksum metadata",
    )?;
    let _comment = crate::json::json_string(
        crate::json::json_field(
            checksum_object,
            "$comment",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package checksum comment",
    )?;
    let recorded = crate::json::json_string(
        crate::json::json_field(
            checksum_object,
            "package",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package checksum",
    )?;
    ensure!(
        recorded == package.checksum,
        "materialized registry package {} {} does not match Cargo.lock",
        package.name,
        package.version
    );
    let files = crate::json::json_object(
        crate::json::json_field(
            checksum_object,
            "files",
            "materialized registry package checksum metadata",
        )?,
        "materialized registry package file checksums",
    )?;
    let mut materialized = BTreeSet::new();
    collect_materialized_files(directory, directory, &mut materialized)?;
    materialized.remove(".cargo-checksum.json");
    let recorded_files = files.keys().cloned().collect::<BTreeSet<_>>();
    ensure!(
        materialized == recorded_files,
        "materialized registry package {} {} file set differs from its authenticated archive",
        package.name,
        package.version
    );
    for (relative, digest) in files {
        ensure!(
            !relative.contains('\\')
                && Path::new(relative)
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
            "materialized registry package has unsafe checksum path `{relative}`"
        );
        let digest = crate::json::json_string(digest, "materialized registry file checksum")?;
        let bytes = std::fs::read(directory.join(relative))
            .with_context(|| format!("reading materialized registry file `{relative}`"))?;
        ensure!(
            crate::sha256::hex(&crate::sha256::digest(&bytes)) == digest,
            "materialized registry file `{relative}` differs from its authenticated archive"
        );
        reject_authority_diagnostic_marker(
            &directory.join(relative),
            &bytes,
            "authenticated registry compiler input",
        )?;
    }
    Ok(())
}

fn collect_materialized_files(
    root: &Path,
    directory: &Path,
    files: &mut BTreeSet<String>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            collect_materialized_files(root, &entry.path(), files)?;
        } else if kind.is_file() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .with_context(|| {
                    format!(
                        "making {} relative to materialized registry root {}",
                        path.display(),
                        root.display()
                    )
                })?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                files.insert(relative.clone()),
                "duplicate registry file `{relative}`"
            );
        } else {
            bail!(
                "materialized registry source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn cargo_input_snapshot(
    cargo_root: &Path,
    cargo_home: &Path,
    registry_source: &Path,
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutation_workspace::canonical_existing_real_directory(
        cargo_root,
        "API audit Cargo-state root",
    )?;
    crate::mutation_workspace::canonical_existing_real_directory(
        cargo_home,
        "API audit private Cargo home",
    )?;
    crate::mutation_workspace::canonical_existing_real_directory(
        registry_source,
        "API audit private registry source",
    )?;
    let mut snapshot = BTreeMap::new();
    snapshot_file(cargo_root, &cargo_home.join("config.toml"), &mut snapshot)?;
    snapshot_tree(cargo_root, registry_source, &mut snapshot)?;
    Ok(snapshot)
}

fn cargo_execution_root(repository: &Path) -> Result<PathBuf> {
    let repository = std::fs::canonicalize(repository)
        .with_context(|| format!("canonicalizing {}", repository.display()))?;
    let execution_root = repository
        .ancestors()
        .last()
        .context("the API audit repository has no filesystem root")?;
    let config = execution_root.join(".cargo");
    for name in ["config", "config.toml"] {
        let path = config.join(name);
        match std::fs::symlink_metadata(&path) {
            Ok(_) => bail!(
                "the API audit refuses Cargo configuration at filesystem root {}",
                path.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspecting {}", path.display()));
            }
        }
    }
    crate::mutation_workspace::canonical_existing_real_directory(
        execution_root,
        "API audit Cargo execution root",
    )
}

fn source_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutation_workspace::canonical_existing_real_directory(root, "API audit snapshot root")?;
    let mut snapshot = BTreeMap::new();
    for relative in COPY_FILES {
        snapshot_file(root, &root.join(relative), &mut snapshot)?;
    }
    for relative in COPY_DIRECTORIES {
        snapshot_tree(root, &root.join(relative), &mut snapshot)?;
    }
    Ok(snapshot)
}

fn snapshot_tree(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(
        directory,
        "API audit snapshot directory",
    )?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            snapshot_tree(root, &entry.path(), snapshot)?;
        } else if kind.is_file() {
            snapshot_file(root, &entry.path(), snapshot)?;
        } else {
            bail!(
                "API audit source {} is not a regular file",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn snapshot_file(
    root: &Path,
    path: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    let parent = path.parent().context("snapshot file has no parent")?;
    crate::mutation_workspace::require_direct_file(parent, path, "API audit snapshot file")?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the source snapshot", path.display()))?
        .to_path_buf();
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(
        snapshot
            .insert(relative.clone(), crate::sha256::digest(&bytes))
            .is_none(),
        "{} occurs twice in the source snapshot",
        relative.display()
    );
    Ok(())
}

fn snapshot_marker_free_file(
    root: &Path,
    path: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    let parent = path.parent().context("snapshot file has no parent")?;
    crate::mutation_workspace::require_direct_file(parent, path, label)?;
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("{} is outside the {label} snapshot", path.display()))?
        .to_path_buf();
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    reject_authority_diagnostic_marker(path, &bytes, label)?;
    ensure!(
        snapshot
            .insert(relative.clone(), crate::sha256::digest(&bytes))
            .is_none(),
        "{} occurs twice in the {label} snapshot",
        relative.display()
    );
    Ok(())
}

fn snapshot_marker_free_tree(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(directory, label)?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            snapshot_marker_free_tree(root, &entry.path(), snapshot, label)?;
        } else if kind.is_file() {
            snapshot_marker_free_file(root, &entry.path(), snapshot, label)?;
        } else {
            bail!("{label} {} is not a regular file", entry.path().display());
        }
    }
    Ok(())
}

fn rejected_audit_environment_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    if name.starts_with("CARGO_") {
        return !matches!(
            name.as_str(),
            "CARGO_BIN_NAME"
                | "CARGO_CRATE_NAME"
                | "CARGO_HOME"
                | "CARGO_LOG"
                | "CARGO_MAKEFLAGS"
                | "CARGO_MANIFEST_DIR"
                | "CARGO_MANIFEST_PATH"
                | "CARGO_PRIMARY_PACKAGE"
                | "CARGO_TARGET_DIR"
        ) && !name.starts_with("CARGO_PKG_")
            && !name.starts_with("CARGO_TERM_");
    }
    matches!(
        name.as_str(),
        "RUSTC"
            | "RUSTC_WRAPPER"
            | "RUSTC_WORKSPACE_WRAPPER"
            | "RUSTC_BOOTSTRAP"
            | "RUSTFLAGS"
            | "RUSTDOC"
            | "RUSTDOCFLAGS"
            | "RUST_TARGET_PATH"
    ) || name.starts_with("RUSTUP_")
        && !matches!(
            name.as_str(),
            "RUSTUP_HOME" | "RUSTUP_TOOLCHAIN" | "RUSTUP_TOOLCHAIN_SOURCE"
        )
}

fn remove_ambient_cargo_semantics(
    command: &mut Command,
    names: impl IntoIterator<Item = std::ffi::OsString>,
) {
    for name in names {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("CARGO_")
            || normalized.starts_with("RUST")
            || normalized == "TARGET"
        {
            command.env_remove(name);
        }
    }
}

fn validate_api_audit_environment() -> Result<()> {
    let mut rejected = Vec::new();
    for (name, _) in std::env::vars_os() {
        let name = name.into_string().map_err(|_| {
            anyhow::anyhow!("the API audit environment has a non-UTF-8 variable name")
        })?;
        if rejected_audit_environment_variable(&name) {
            rejected.push(name);
        }
    }
    rejected.sort();
    ensure!(
        rejected.is_empty(),
        "the API audit refuses compiler-semantic environment variable(s): {}",
        rejected.join(", ")
    );
    Ok(())
}

/// Compile every public `libs/` and `wost` item against its resolved callers.
pub fn check(paths: &Paths) -> Result<()> {
    validate_api_audit_environment()?;
    let workspace = AuditWorkspace::create(paths)?;
    reject_deprecation_suppression(&workspace.root)?;
    let plan = AuditPlan::discover(&workspace, &workspace.source)?;
    let inventory = Inventory::discover_from_catalog(&workspace.root, &plan.catalog)?;
    let mut uses = GraphUses::default();
    println!(
        "[xtask] auditing {} public API items in {} compiler rounds",
        inventory.report_count(),
        inventory.rounds
    );
    let pristine_source = source_snapshot(&workspace.root)?;
    let _pristine = compile_graphs(&workspace, 0, &pristine_source, &plan, None)?;
    workspace.require_cargo_inputs_unchanged()?;
    plan.compiler.require_input_authority_unchanged()?;
    let pristine_build_authority = (0..plan.compilations.len())
        .map(|graph| {
            build_script_authority_snapshot(&graph_build_directory(&workspace.build, graph)?)
        })
        .collect::<Result<Vec<_>>>()?;
    for round in 0..inventory.rounds {
        let insertions = inventory.instrument(&workspace.root, round)?;
        let instrumented_source = source_snapshot(&workspace.root)?;
        for compilation in compile_graphs(
            &workspace,
            round,
            &instrumented_source,
            &plan,
            Some(&pristine_build_authority),
        )? {
            uses.observe_cargo(
                compilation.graph,
                compilation.kind,
                &inventory,
                &workspace.root,
                &compilation.diagnostics,
                &insertions,
            )?;
        }
        workspace.require_cargo_inputs_unchanged()?;
        plan.compiler.require_target_libraries_unchanged()?;
    }
    workspace.require_source_unchanged(paths.root())?;
    plan.compiler.require_input_authority_unchanged()?;

    uses.validate_dispatches(&inventory)?;
    let dead = uses.dead(&inventory);
    if dead.is_empty() {
        println!(
            "[xtask] all {} public API items have compiler-resolved callers",
            inventory.report_count()
        );
        return Ok(());
    }
    let report = dead
        .iter()
        .map(|node| format!("\n  {}", node.label))
        .collect::<String>();
    bail!("{} public API item(s) have no caller:{report}", dead.len());
}

pub(crate) fn report_names_exact_dead_item(report: &str, wanted: &str) -> bool {
    let mut lines = report.lines();
    let Some(header) = core::iter::Iterator::next(&mut lines) else {
        return false;
    };
    let Some((count, summary)) = header.split_once(' ') else {
        return false;
    };
    let Ok(count) = <usize as core::str::FromStr>::from_str(count) else {
        return false;
    };
    if count != 1 || summary != "public API item(s) have no caller:" {
        return false;
    }
    let Some(row) = core::iter::Iterator::next(&mut lines).and_then(|line| line.strip_prefix("  "))
    else {
        return false;
    };
    if core::iter::Iterator::next(&mut lines).is_some() {
        return false;
    }
    let Some((identity, location)) = row.rsplit_once(" at ") else {
        return false;
    };
    identity == wanted
        && location.rsplit_once(':').is_some_and(|(path, line)| {
            !path.is_empty()
                && <usize as core::str::FromStr>::from_str(line).is_ok_and(|line| line > 0)
        })
}

struct Compilation {
    graph: usize,
    kind: RootKind,
    diagnostics: Vec<MarkerDiagnostic>,
}

#[derive(Debug, Clone)]
enum CompilationMode {
    RegisteredPass,
    Example(CargoTarget),
    TestHarness(CargoTarget),
}

#[derive(Debug, Clone)]
struct CompilationPlan {
    pass: gate::CargoPass,
    target: String,
    kind: RootKind,
    mode: CompilationMode,
}

#[derive(Debug)]
struct AuditPlan {
    catalog: CargoCatalog,
    compiler: CompilerIdentity,
    compilations: Vec<CompilationPlan>,
}

impl AuditPlan {
    fn discover(
        workspace: &AuditWorkspace,
        expected: &BTreeMap<PathBuf, [u8; 32]>,
    ) -> Result<Self> {
        let compiler = compiler_identity(&workspace.source_root)?;
        let mut catalog: Option<CargoCatalog> = None;
        for &pass in gate::REGISTRY.cargo_passes() {
            let current = CargoCatalog::discover(workspace, expected, pass, &compiler)?;
            if let Some(catalog) = &catalog {
                ensure!(
                    catalog.packages == current.packages,
                    "Cargo target metadata changes across registered compiler graphs"
                );
            } else {
                catalog = Some(current);
            }
        }
        let catalog = catalog.context("the API audit has no registered Cargo graph")?;
        compiler.require_input_authority_unchanged()?;
        Self::from_catalog(catalog, compiler)
    }

    fn from_catalog(catalog: CargoCatalog, compiler: CompilerIdentity) -> Result<Self> {
        let mut compilations = Vec::new();
        for &pass in gate::REGISTRY.cargo_passes() {
            let compiler_target = pass.target.unwrap_or(&compiler.host_target).to_string();
            compilations.push(CompilationPlan {
                pass,
                target: compiler_target.clone(),
                kind: if pass.features.is_some() {
                    RootKind::Verification
                } else {
                    RootKind::Production
                },
                mode: CompilationMode::RegisteredPass,
            });
            if pass.target.is_some() {
                continue;
            }
            for package in pass.packages {
                for target in catalog.package(package)? {
                    if target.kind == CargoTargetKind::Example {
                        compilations.push(CompilationPlan {
                            pass,
                            target: compiler_target.clone(),
                            kind: RootKind::Production,
                            mode: CompilationMode::Example(target.clone()),
                        });
                    }
                    let standard_harness = match target.kind {
                        CargoTargetKind::Test | CargoTargetKind::Bench => true,
                        CargoTargetKind::Library
                        | CargoTargetKind::Bin
                        | CargoTargetKind::Example => target.test,
                        CargoTargetKind::CustomBuild => false,
                    };
                    if standard_harness {
                        compilations.push(CompilationPlan {
                            pass,
                            target: compiler_target.clone(),
                            kind: RootKind::TestHarness,
                            mode: CompilationMode::TestHarness(target.clone()),
                        });
                    }
                }
            }
        }
        Ok(Self {
            catalog,
            compiler,
            compilations,
        })
    }
}

#[derive(Debug, Clone)]
struct CompilerIdentity {
    path: PathBuf,
    host_target: String,
    host_linker: PathBuf,
    tool_path: std::ffi::OsString,
    compiler_digest: [u8; 32],
    linker_digest: [u8; 32],
    input_authority: CompilerInputAuthority,
}

#[derive(Debug, Clone)]
struct CompilerInputAuthority {
    sysroot: PathBuf,
    cargo: PathBuf,
    compiler: PathBuf,
    targets: Vec<String>,
    runtime_snapshot: BTreeMap<PathBuf, [u8; 32]>,
    target_library_snapshot: BTreeMap<PathBuf, [u8; 32]>,
}

fn compiler_identity(source_root: &Path) -> Result<CompilerIdentity> {
    let cargo = std::fs::canonicalize(env!("CARGO"))
        .with_context(|| format!("resolving API audit Cargo executable {}", env!("CARGO")))?;
    let compiler_name = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    let compiler = cargo
        .parent()
        .context("the API audit Cargo executable has no toolchain directory")?
        .join(compiler_name);
    let compiler = std::fs::canonicalize(&compiler).with_context(|| {
        format!(
            "resolving the API audit compiler beside Cargo at {}",
            compiler.display()
        )
    })?;
    let result = Command::new(&compiler)
        .current_dir(source_root)
        .arg("-vV")
        .output()
        .context("querying the API audit compiler host target")?;
    ensure!(
        result.status.success(),
        "querying the API audit compiler host target failed ({}): {}",
        result.status,
        String::from_utf8_lossy(&result.stderr)
    );
    let version = std::str::from_utf8(&result.stdout)
        .context("the API audit compiler version is not UTF-8")?;
    let targets = version
        .lines()
        .filter_map(|line| line.strip_prefix("host: "))
        .collect::<Vec<_>>();
    let [target] = targets.as_slice() else {
        bail!("the API audit compiler did not report exactly one host target");
    };
    ensure!(
        !target.is_empty()
            && target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "the API audit compiler returned malformed host target `{target}`"
    );
    let sysroot = Command::new(&compiler)
        .current_dir(source_root)
        .args(["--print", "sysroot"])
        .output()
        .context("querying the API audit compiler sysroot")?;
    ensure!(
        sysroot.status.success(),
        "querying the API audit compiler sysroot failed ({}): {}",
        sysroot.status,
        String::from_utf8_lossy(&sysroot.stderr)
    );
    let sysroot = std::str::from_utf8(&sysroot.stdout)
        .context("the API audit compiler sysroot is not UTF-8")?
        .trim();
    ensure!(
        !sysroot.is_empty(),
        "the API audit compiler returned an empty sysroot"
    );
    let sysroot = std::fs::canonicalize(sysroot)
        .with_context(|| format!("canonicalizing compiler sysroot `{sysroot}`"))?;
    let toolchain = compiler
        .parent()
        .and_then(Path::parent)
        .context("the API audit compiler is outside a toolchain root")?;
    ensure!(
        sysroot == toolchain,
        "the API audit compiler reported a sysroot outside its exact toolchain"
    );
    let linker_name = if cfg!(windows) {
        "rust-lld.exe"
    } else {
        "rust-lld"
    };
    let host_linker = std::fs::canonicalize(
        sysroot
            .join("lib/rustlib")
            .join(*target)
            .join("bin")
            .join(linker_name),
    )
    .context("resolving the API audit host linker inside the compiler sysroot")?;
    ensure!(
        host_linker.starts_with(&sysroot),
        "the API audit host linker escaped the compiler sysroot"
    );
    let host_linker = crate::cargo::child_process_path(&host_linker);
    let tool_path = std::env::join_paths([compiler
        .parent()
        .context("the API audit compiler has no binary directory")?])
    .context("constructing the API audit tool PATH")?;
    let compiler_digest = crate::sha256::digest(
        &std::fs::read(&compiler)
            .with_context(|| format!("reading compiler {}", compiler.display()))?,
    );
    let linker_digest = crate::sha256::digest(
        &std::fs::read(&host_linker)
            .with_context(|| format!("reading linker {}", host_linker.display()))?,
    );
    let input_authority = CompilerInputAuthority::capture(&sysroot, &cargo, &compiler, target)?;
    Ok(CompilerIdentity {
        path: compiler,
        host_target: target.to_string(),
        host_linker,
        tool_path,
        compiler_digest,
        linker_digest,
        input_authority,
    })
}

impl CompilerInputAuthority {
    fn capture(sysroot: &Path, cargo: &Path, compiler: &Path, host_target: &str) -> Result<Self> {
        let sysroot = std::fs::canonicalize(sysroot)
            .with_context(|| format!("canonicalizing compiler sysroot {}", sysroot.display()))?;
        let cargo = std::fs::canonicalize(cargo)
            .with_context(|| format!("canonicalizing Cargo executable {}", cargo.display()))?;
        let compiler = std::fs::canonicalize(compiler).with_context(|| {
            format!("canonicalizing compiler executable {}", compiler.display())
        })?;
        ensure!(
            cargo.starts_with(&sysroot) && compiler.starts_with(&sysroot),
            "Cargo or rustc escaped the exact compiler sysroot"
        );
        let mut targets = [host_target, crate::paths::KERNEL_TARGET]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        targets.sort();
        targets.dedup();
        let runtime_snapshot = compiler_runtime_snapshot(&sysroot, &cargo, &compiler, &targets)?;
        let target_library_snapshot = compiler_target_library_snapshot(&sysroot, &targets)?;
        Ok(Self {
            sysroot,
            cargo,
            compiler,
            targets,
            runtime_snapshot,
            target_library_snapshot,
        })
    }

    fn require_unchanged(&self) -> Result<()> {
        self.require_runtime_unchanged()?;
        self.require_target_libraries_unchanged()
    }

    fn require_runtime_unchanged(&self) -> Result<()> {
        let current =
            compiler_runtime_snapshot(&self.sysroot, &self.cargo, &self.compiler, &self.targets)?;
        require_compiler_snapshot_unchanged(&self.runtime_snapshot, &current)
    }

    fn require_target_libraries_unchanged(&self) -> Result<()> {
        let current = compiler_target_library_snapshot(&self.sysroot, &self.targets)?;
        require_compiler_snapshot_unchanged(&self.target_library_snapshot, &current)
    }
}

fn require_compiler_snapshot_unchanged(
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    current: &BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    if current == expected {
        return Ok(());
    }
    let changed = expected
        .keys()
        .chain(current.keys())
        .find(|path| expected.get(*path) != current.get(*path))
        .context("compiler authority changed without naming an input")?;
    bail!(
        "compiler input {} changed while the API audit was running",
        changed.display()
    )
}

#[cfg(test)]
fn compiler_input_snapshot(
    sysroot: &Path,
    cargo: &Path,
    compiler: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    let mut snapshot = compiler_runtime_snapshot(sysroot, cargo, compiler, targets)?;
    for (path, digest) in compiler_target_library_snapshot(sysroot, targets)? {
        ensure!(
            snapshot.insert(path.clone(), digest).is_none(),
            "{} occurs twice in the compiler input snapshot",
            path.display()
        );
    }
    Ok(snapshot)
}

fn compiler_runtime_snapshot(
    sysroot: &Path,
    cargo: &Path,
    compiler: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutation_workspace::canonical_existing_real_directory(
        sysroot,
        "API audit compiler sysroot",
    )?;
    let mut snapshot = BTreeMap::new();
    for executable in [cargo, compiler] {
        ensure!(
            executable.starts_with(sysroot),
            "compiler executable {} escaped the exact sysroot",
            executable.display()
        );
        snapshot_marker_free_file(
            sysroot,
            executable,
            &mut snapshot,
            "compiler executable authority",
        )?;
    }

    let bin = sysroot.join("bin");
    crate::mutation_workspace::canonical_existing_real_directory(
        &bin,
        "API audit compiler runtime directory",
    )?;
    for entry in std::fs::read_dir(&bin)
        .with_context(|| format!("reading compiler runtime directory {}", bin.display()))?
    {
        let entry = entry.with_context(|| format!("reading an entry in {}", bin.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            continue;
        }
        let runtime_library = entry.path().extension().is_some_and(|extension| {
            matches!(
                extension.to_string_lossy().to_ascii_lowercase().as_str(),
                "dll" | "dylib" | "so"
            )
        });
        ensure!(
            kind.is_file() || !runtime_library,
            "compiler runtime authority {} is not a regular file",
            entry.path().display()
        );
        if runtime_library {
            snapshot_marker_free_file(
                sysroot,
                &entry.path(),
                &mut snapshot,
                "compiler runtime authority",
            )?;
        }
    }

    let top_level_lib = sysroot.join("lib");
    crate::mutation_workspace::canonical_existing_real_directory(
        &top_level_lib,
        "API audit compiler library directory",
    )?;
    for entry in std::fs::read_dir(&top_level_lib).with_context(|| {
        format!(
            "reading compiler library directory {}",
            top_level_lib.display()
        )
    })? {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", top_level_lib.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_file() {
            snapshot_marker_free_file(
                sysroot,
                &entry.path(),
                &mut snapshot,
                "compiler runtime authority",
            )?;
        } else {
            ensure!(
                kind.is_dir(),
                "compiler runtime authority {} is not a regular file or directory",
                entry.path().display()
            );
        }
    }

    snapshot_compiler_target_runtime(sysroot, targets, &mut snapshot)?;
    Ok(snapshot)
}

fn snapshot_compiler_target_runtime(
    sysroot: &Path,
    targets: &[String],
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    for target in targets {
        let target_root = sysroot.join("lib/rustlib").join(target);
        crate::mutation_workspace::canonical_existing_real_directory(
            &target_root,
            "API audit compiler target root",
        )?;
        for entry in std::fs::read_dir(&target_root)
            .with_context(|| format!("reading compiler target root {}", target_root.display()))?
        {
            let entry =
                entry.with_context(|| format!("reading an entry in {}", target_root.display()))?;
            if entry.file_name() == "lib" {
                continue;
            }
            let kind = entry
                .file_type()
                .with_context(|| format!("reading the type of {}", entry.path().display()))?;
            if kind.is_dir() {
                snapshot_marker_free_tree(
                    sysroot,
                    &entry.path(),
                    snapshot,
                    "compiler target runtime authority",
                )?;
            } else if kind.is_file() {
                snapshot_marker_free_file(
                    sysroot,
                    &entry.path(),
                    snapshot,
                    "compiler target runtime authority",
                )?;
            } else {
                bail!(
                    "compiler target runtime authority {} is not a regular file or directory",
                    entry.path().display()
                );
            }
        }
    }
    Ok(())
}

fn compiler_target_library_snapshot(
    sysroot: &Path,
    targets: &[String],
) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutation_workspace::canonical_existing_real_directory(
        sysroot,
        "API audit compiler sysroot",
    )?;
    let mut snapshot = BTreeMap::new();
    for target in targets {
        let target_lib = sysroot.join("lib/rustlib").join(target).join("lib");
        snapshot_marker_free_tree(
            sysroot,
            &target_lib,
            &mut snapshot,
            "compiler target library authority",
        )?;
    }
    Ok(snapshot)
}

impl CompilerIdentity {
    fn require_unchanged(&self) -> Result<()> {
        ensure!(
            crate::sha256::digest(
                &std::fs::read(&self.path)
                    .with_context(|| format!("reading compiler {}", self.path.display()))?
            ) == self.compiler_digest,
            "the API audit compiler changed while the audit was running"
        );
        ensure!(
            crate::sha256::digest(
                &std::fs::read(&self.host_linker)
                    .with_context(|| format!("reading linker {}", self.host_linker.display()))?
            ) == self.linker_digest,
            "the API audit host linker changed while the audit was running"
        );
        Ok(())
    }

    fn require_input_authority_unchanged(&self) -> Result<()> {
        self.input_authority.require_unchanged()
    }

    fn require_target_libraries_unchanged(&self) -> Result<()> {
        self.input_authority.require_target_libraries_unchanged()
    }
}

fn cargo_encoded_rustflags(target: Option<&str>) -> Result<&'static str> {
    match target {
        None => Ok(""),
        Some(crate::paths::KERNEL_TARGET) => {
            Ok("-C\u{1f}code-model=kernel\u{1f}-C\u{1f}relocation-model=static")
        }
        Some(target) => bail!("API audit has no exact rustflags contract for target `{target}`"),
    }
}

fn audited_cargo_command(
    execution_root: &Path,
    cargo_home: &Path,
    registry_source: &Path,
    compiler: &CompilerIdentity,
) -> Result<Command> {
    let mut command = base_cargo_command(execution_root, cargo_home, compiler);
    let registry = format!("{:?}", registry_source.to_string_lossy());
    command
        .args([
            "--config",
            "source.crates-io.replace-with=\"api-audit-registry\"",
            "--config",
        ])
        .arg(format!("source.api-audit-registry.directory={registry}"));
    Ok(command)
}

fn base_cargo_command(
    execution_root: &Path,
    cargo_home: &Path,
    compiler: &CompilerIdentity,
) -> Command {
    let mut command = Command::new(env!("CARGO"));
    remove_ambient_cargo_semantics(&mut command, std::env::vars_os().map(|(name, _)| name));
    command
        .current_dir(execution_root)
        .env("CARGO_HOME", cargo_home)
        .env("PATH", &compiler.tool_path)
        .env("RUSTC", &compiler.path)
        .env("RUSTC_WRAPPER", "")
        .env("RUSTC_WORKSPACE_WRAPPER", "")
        .args(["--config"])
        .arg(format!(
            "target.{}.linker={:?}",
            compiler.host_target,
            compiler.host_linker.to_string_lossy()
        ));
    command
}

fn compile_graphs(
    workspace: &AuditWorkspace,
    round: usize,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    plan: &AuditPlan,
    pristine_build_authority: Option<&[BTreeMap<PathBuf, [u8; 32]>]>,
) -> Result<Vec<Compilation>> {
    let mut compilations = Vec::new();
    for (graph, compilation) in plan.compilations.iter().enumerate() {
        let compiled = compile_graph(
            workspace,
            round,
            graph,
            &plan.compiler,
            compilation,
            expected,
        )?;
        if let Some(pristine) = pristine_build_authority {
            let expected = pristine
                .get(graph)
                .context("a compiler graph has no pristine build-script baseline")?;
            require_build_script_authority_unchanged(
                &graph_build_directory(&workspace.build, graph)?,
                expected,
                &compilation_label(compilation),
            )?;
        }
        compilations.push(compiled);
    }
    Ok(compilations)
}

fn compile_graph(
    workspace: &AuditWorkspace,
    round: usize,
    graph: usize,
    compiler: &CompilerIdentity,
    plan: &CompilationPlan,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
) -> Result<Compilation> {
    let args = cargo_arguments(plan);
    let build = graph_build_directory(&workspace.build, graph)?;
    let result = audited_cargo_command(
        &workspace.execution_root,
        &workspace.cargo_home,
        &workspace.registry_source,
        compiler,
    )?
    .env("CARGO_TARGET_DIR", build)
    .env(
        "CARGO_ENCODED_RUSTFLAGS",
        cargo_encoded_rustflags(plan.pass.target)?,
    )
    .env("CARGO_BUILD_WARNINGS", "warn")
    .args(&args)
    .arg("--manifest-path")
    .arg(workspace.root.join("Cargo.toml"))
    .arg("--color=never")
    .output()
    .with_context(|| {
        format!(
            "spawning API audit round {round} for {}",
            compilation_label(plan)
        )
    })?;
    workspace.require_copy_unchanged(expected)?;
    workspace.require_source_unchanged(&workspace.source_root)?;
    compiler.require_unchanged()?;
    if !result.status.success() {
        bail!(
            "API audit round {round} failed to compile {} ({})\n{}{}",
            compilation_label(plan),
            result.status,
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let diagnostics = cargo_compilation_output(
        &result.stdout,
        match &plan.mode {
            CompilationMode::TestHarness(target) => Some(target),
            CompilationMode::RegisteredPass | CompilationMode::Example(_) => None,
        },
    )?;
    Ok(Compilation {
        graph,
        kind: plan.kind,
        diagnostics,
    })
}

fn cargo_arguments(plan: &CompilationPlan) -> Vec<String> {
    let (command, selected) = match &plan.mode {
        CompilationMode::RegisteredPass => ("check", None),
        CompilationMode::Example(target) => ("check", Some(target)),
        CompilationMode::TestHarness(target) => ("test", Some(target)),
    };
    let mut args = vec![command.to_string()];
    if matches!(plan.mode, CompilationMode::TestHarness(_)) {
        args.push("--no-run".to_string());
    }
    args.extend([
        "--offline".to_string(),
        "--locked".to_string(),
        "--message-format=json".to_string(),
    ]);
    if let Some(target) = selected {
        args.extend(["--package".to_string(), target.package.clone()]);
        args.extend(target_selector(target));
    } else {
        args.extend(crate::cargo::package_args(plan.pass.packages));
    }
    args.extend(["--target".to_string(), plan.target.clone()]);
    if let Some(features) = plan.pass.features {
        args.extend(["--features".to_string(), features.to_string()]);
    }
    args
}

fn target_selector(target: &CargoTarget) -> Vec<String> {
    match target.kind {
        CargoTargetKind::Library => vec!["--lib".to_string()],
        CargoTargetKind::Bin => vec!["--bin".to_string(), target.name.clone()],
        CargoTargetKind::Example => vec!["--example".to_string(), target.name.clone()],
        CargoTargetKind::Test => vec!["--test".to_string(), target.name.clone()],
        CargoTargetKind::Bench => vec!["--bench".to_string(), target.name.clone()],
        CargoTargetKind::CustomBuild => unreachable!("build scripts have no Cargo selector"),
    }
}

fn compilation_label(plan: &CompilationPlan) -> String {
    match &plan.mode {
        CompilationMode::RegisteredPass => plan.pass.label(),
        CompilationMode::Example(target) => {
            format!("{} example {}", target.package, target.name)
        }
        CompilationMode::TestHarness(target) => format!(
            "{} {} {} test harness",
            target.package,
            target.kind.metadata_name(),
            target.name
        ),
    }
}

fn cargo_compilation_output(
    bytes: &[u8],
    selected_test: Option<&CargoTarget>,
) -> Result<Vec<MarkerDiagnostic>> {
    let text = std::str::from_utf8(bytes).context("Cargo compiler JSON is not UTF-8")?;
    let mut diagnostics = Vec::new();
    let mut selected_artifact = false;
    let mut test_artifact = false;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let value = crate::json::parse_json(line.as_bytes(), "Cargo compiler JSON message")?;
        let message = crate::json::json_object(&value, "Cargo compiler JSON message")?;
        let reason = crate::json::json_string(
            crate::json::json_field(message, "reason", "Cargo compiler JSON message")?,
            "Cargo compiler JSON `reason`",
        )?;
        if reason == "compiler-message" {
            let package_id = crate::json::json_string(
                crate::json::json_field(message, "package_id", "Cargo compiler message")?,
                "Cargo compiler message `package_id`",
            )?;
            let target = crate::json::json_object(
                crate::json::json_field(message, "target", "Cargo compiler message")?,
                "Cargo compiler message target",
            )?;
            let compiler_message = crate::json::json_object(
                crate::json::json_field(message, "message", "Cargo compiler message")?,
                "Cargo compiler message",
            )?;
            let diagnostic_message = crate::json::json_string(
                crate::json::json_field(compiler_message, "message", "Cargo compiler message")?,
                "Cargo compiler diagnostic `message`",
            )?;
            let Some(id) = diagnostic_marker_id(diagnostic_message)? else {
                continue;
            };
            let level = crate::json::json_string(
                crate::json::json_field(compiler_message, "level", "Cargo compiler message")?,
                "Cargo compiler diagnostic `level`",
            )?;
            ensure!(
                level == "warning",
                "API marker diagnostic has compiler level `{level}` instead of `warning`"
            );
            let code = crate::json::json_object(
                crate::json::json_field(compiler_message, "code", "Cargo compiler message")?,
                "API marker diagnostic code",
            )?;
            let code = crate::json::json_string(
                crate::json::json_field(code, "code", "API marker diagnostic code")?,
                "API marker diagnostic code name",
            )?;
            ensure!(
                code == "deprecated",
                "API marker diagnostic has compiler code `{code}` instead of `deprecated`"
            );
            let spans = crate::json::json_array(
                crate::json::json_field(compiler_message, "spans", "Cargo compiler message")?,
                "Cargo compiler diagnostic `spans`",
            )?;
            let primary = spans
                .iter()
                .map(|span| {
                    let span = crate::json::json_object(span, "Cargo compiler diagnostic span")?;
                    let primary = match crate::json::json_field(
                        span,
                        "is_primary",
                        "Cargo compiler diagnostic span",
                    )? {
                        crate::json::JsonValue::Bool(primary) => *primary,
                        _ => bail!("Cargo compiler diagnostic `is_primary` is not a Boolean"),
                    };
                    Ok(primary.then_some(span))
                })
                .collect::<Result<Vec<_>>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            let [primary] = primary.as_slice() else {
                bail!(
                    "API marker diagnostic has {} primary spans instead of exactly one",
                    primary.len()
                );
            };
            let primary_location =
                cargo_diagnostic_location(primary, "primary API diagnostic span")?;
            let immediate_expansion = match crate::json::json_field(
                primary,
                "expansion",
                "primary API diagnostic span",
            )? {
                crate::json::JsonValue::Null => None,
                expansion @ crate::json::JsonValue::Object(_) => {
                    let expansion =
                        crate::json::json_object(expansion, "primary API diagnostic expansion")?;
                    let macro_decl_name = crate::json::json_string(
                        crate::json::json_field(
                            expansion,
                            "macro_decl_name",
                            "primary API diagnostic expansion",
                        )?,
                        "primary API diagnostic expansion `macro_decl_name`",
                    )?;
                    let span = crate::json::json_object(
                        crate::json::json_field(
                            expansion,
                            "span",
                            "primary API diagnostic expansion",
                        )?,
                        "primary API diagnostic expansion span",
                    )?;
                    let terminal = match crate::json::json_field(
                        span,
                        "expansion",
                        "primary API diagnostic expansion span",
                    )? {
                        crate::json::JsonValue::Null => true,
                        crate::json::JsonValue::Object(_) => false,
                        _ => bail!(
                            "primary API diagnostic expansion span `expansion` is neither null nor an object"
                        ),
                    };
                    Some(DiagnosticExpansion {
                        location: cargo_diagnostic_location(
                            span,
                            "primary API diagnostic expansion span",
                        )?,
                        terminal,
                        macro_decl_name: macro_decl_name.to_string(),
                    })
                }
                _ => bail!("primary API diagnostic `expansion` is neither null nor an object"),
            };
            let target_name = crate::json::json_string(
                crate::json::json_field(target, "name", "Cargo compiler message target")?,
                "Cargo compiler message target `name`",
            )?;
            let kinds = crate::json::json_string_array(
                crate::json::json_field(target, "kind", "Cargo compiler message target")?,
                "Cargo compiler message target `kind`",
            )?;
            let target_source = PathBuf::from(crate::json::json_string(
                crate::json::json_field(target, "src_path", "Cargo compiler message target")?,
                "Cargo compiler message target `src_path`",
            )?);
            ensure!(
                target_source.is_absolute(),
                "Cargo compiler message target source is not absolute"
            );
            diagnostics.push(MarkerDiagnostic {
                id,
                origin: CargoDiagnosticOrigin {
                    package_id: package_id.to_string(),
                    target_name: target_name.to_string(),
                    target_kind: CargoTargetKind::parse(&kinds)?,
                    target_source,
                },
                reported: primary_location.reported,
                line: primary_location.line,
                column: primary_location.column,
                immediate_expansion,
            });
            continue;
        }
        if reason != "compiler-artifact" {
            continue;
        }
        let Some(selected) = selected_test else {
            continue;
        };
        let package_id = crate::json::json_string(
            crate::json::json_field(message, "package_id", "Cargo compiler artifact")?,
            "Cargo compiler artifact `package_id`",
        )?;
        let target = crate::json::json_object(
            crate::json::json_field(message, "target", "Cargo compiler artifact")?,
            "Cargo compiler artifact target",
        )?;
        let target_name = crate::json::json_string(
            crate::json::json_field(target, "name", "Cargo compiler artifact target")?,
            "Cargo compiler artifact target `name`",
        )?;
        let kinds = crate::json::json_string_array(
            crate::json::json_field(target, "kind", "Cargo compiler artifact target")?,
            "Cargo compiler artifact target `kind`",
        )?;
        if package_id != selected.package_id
            || target_name != selected.name
            || CargoTargetKind::parse(&kinds)? != selected.kind
        {
            continue;
        }
        selected_artifact = true;
        let profile = crate::json::json_object(
            crate::json::json_field(message, "profile", "Cargo compiler artifact")?,
            "Cargo compiler artifact profile",
        )?;
        let test =
            match crate::json::json_field(profile, "test", "Cargo compiler artifact profile")? {
                crate::json::JsonValue::Bool(test) => *test,
                _ => bail!("Cargo compiler artifact profile `test` is not a Boolean"),
            };
        test_artifact |= test;
    }
    if let Some(selected) = selected_test {
        ensure!(
            selected_artifact,
            "Cargo emitted no artifact for selected target `{}:{}`",
            selected.package,
            selected.name
        );
        ensure!(
            test_artifact,
            "Cargo target `{}:{}` uses a custom test harness, which the API audit cannot classify",
            selected.package,
            selected.name
        );
    }
    Ok(diagnostics)
}

fn cargo_diagnostic_location(
    span: &BTreeMap<String, crate::json::JsonValue>,
    label: &str,
) -> Result<DiagnosticLocation> {
    let reported = crate::json::json_string(
        crate::json::json_field(span, "file_name", label)?,
        &format!("{label} `file_name`"),
    )?;
    let line = usize::try_from(crate::json::json_u64(
        crate::json::json_field(span, "line_start", label)?,
        &format!("{label} `line_start`"),
    )?)
    .with_context(|| format!("{label} line does not fit usize"))?;
    let column = usize::try_from(crate::json::json_u64(
        crate::json::json_field(span, "column_start", label)?,
        &format!("{label} `column_start`"),
    )?)
    .with_context(|| format!("{label} column does not fit usize"))?;
    ensure!(line > 0 && column > 0, "{label} has a zero line or column");
    Ok(DiagnosticLocation {
        reported: reported.to_string(),
        line,
        column,
    })
}

#[cfg(test)]
fn cargo_test_output(bytes: &[u8], selected: &CargoTarget) -> Result<Vec<MarkerDiagnostic>> {
    cargo_compilation_output(bytes, Some(selected))
}

fn diagnostic_marker_id(message: &str) -> Result<Option<usize>> {
    let markers = message.match_indices(MARKER).collect::<Vec<_>>();
    let [] = markers.as_slice() else {
        ensure!(
            markers.len() == 1,
            "compiler diagnostic contains {} API markers",
            markers.len()
        );
        let start = markers[0].0 + MARKER.len();
        let digits = message[start..]
            .chars()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>();
        ensure!(
            !digits.is_empty(),
            "compiler diagnostic has a malformed API marker"
        );
        return Ok(Some(
            <usize as core::str::FromStr>::from_str(&digits)
                .context("compiler diagnostic API marker does not fit usize")?,
        ));
    };
    Ok(None)
}

fn graph_build_directory(build: &Path, graph: usize) -> Result<PathBuf> {
    let directory = build.join(format!("graph-{graph}"));
    let directory = crate::mutation_workspace::prepare_real_child(
        build,
        &directory,
        "API audit graph build directory",
    )?;
    Ok(crate::cargo::child_process_path(&directory))
}

fn build_script_authority_snapshot(root: &Path) -> Result<BTreeMap<PathBuf, [u8; 32]>> {
    crate::mutation_workspace::canonical_existing_real_directory(
        root,
        "API audit compiler-graph build directory",
    )?;
    let mut snapshot = BTreeMap::new();
    collect_build_script_authority(root, root, &mut snapshot)?;
    Ok(snapshot)
}

fn require_build_script_authority_unchanged(
    root: &Path,
    expected: &BTreeMap<PathBuf, [u8; 32]>,
    label: &str,
) -> Result<()> {
    ensure!(
        build_script_authority_snapshot(root)? == *expected,
        "{label} changed build-script output or OUT_DIR authority after source instrumentation"
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildScriptAuthorityPath {
    Output,
    OutRoot,
    OutEntry,
}

fn collect_build_script_authority(
    root: &Path,
    directory: &Path,
    snapshot: &mut BTreeMap<PathBuf, [u8; 32]>,
) -> Result<()> {
    crate::mutation_workspace::canonical_existing_real_directory(
        directory,
        "API audit build-script authority directory",
    )?;
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        let relative = entry
            .path()
            .strip_prefix(root)
            .context("build-script authority escaped its compiler graph")?
            .to_path_buf();
        let authority = build_script_authority_path(&relative);
        if kind.is_dir() {
            ensure!(
                authority != Some(BuildScriptAuthorityPath::Output),
                "build-script output {} is not a regular file",
                entry.path().display()
            );
            collect_build_script_authority(root, &entry.path(), snapshot)?;
        } else if kind.is_file() {
            ensure!(
                authority != Some(BuildScriptAuthorityPath::OutRoot),
                "build-script OUT_DIR {} is not a directory",
                entry.path().display()
            );
            if authority.is_some() {
                snapshot_marker_free_file(
                    root,
                    &entry.path(),
                    snapshot,
                    "build-script compiler input",
                )?;
            }
        } else if authority.is_some() {
            bail!(
                "build-script authority {} is not a regular file or directory",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn build_script_authority_path(relative: &Path) -> Option<BuildScriptAuthorityPath> {
    let components = relative
        .components()
        .map(|component| component.as_os_str())
        .collect::<Vec<_>>();
    components
        .iter()
        .enumerate()
        .find_map(|(index, component)| {
            if component != &"build" || index + 2 >= components.len() {
                return None;
            }
            let authority = &components[index + 2];
            if authority == &"output" && index + 3 == components.len() {
                Some(BuildScriptAuthorityPath::Output)
            } else if authority == &"out" && index + 3 == components.len() {
                Some(BuildScriptAuthorityPath::OutRoot)
            } else if authority == &"out" {
                Some(BuildScriptAuthorityPath::OutEntry)
            } else {
                None
            }
        })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn isolated_test_cargo_command(root: &Path, compiler: &CompilerIdentity) -> Command {
        let cargo = root.join("audit-cargo");
        let home = cargo.join("home");
        let registry = cargo.join("registry-source");
        std::fs::create_dir_all(&home).expect("private test Cargo home");
        std::fs::create_dir_all(&registry).expect("private test Cargo registry");
        std::fs::write(home.join("config.toml"), "[net]\noffline = true\n")
            .expect("private test Cargo config");
        audited_cargo_command(
            &cargo_execution_root(root).expect("isolated Cargo execution root"),
            &home,
            &registry,
            compiler,
        )
        .expect("isolated Cargo command")
    }

    fn validated_entry_fixture_source() -> &'static str {
        r#"pub struct BootInfo;

pub mod limine_glue {
    pub struct Request;
    impl Request { pub const fn new() -> Self { Self } }
    pub struct BaseRevision;
    pub type RequestsStartMarker = Request;
    pub type FramebufferRequest = Request;
    pub type MemoryMapRequest = Request;
    pub type HhdmRequest = Request;
    pub type ExecutableAddressRequest = Request;
    pub type RsdpRequest = Request;
    pub type ModuleRequest = Request;
    pub type RequestsEndMarker = Request;
    pub struct Requests {
        pub base_revision: &'static BaseRevision,
        pub framebuffer: &'static FramebufferRequest,
        pub memory_map: &'static MemoryMapRequest,
        pub hhdm: &'static HhdmRequest,
        pub image: &'static ExecutableAddressRequest,
        pub rsdp: &'static RsdpRequest,
        pub modules: &'static ModuleRequest,
    }
    pub const fn base_revision() -> BaseRevision { BaseRevision }
    pub fn enter(_: Requests, handler: fn(super::BootInfo) -> !) -> ! {
        handler(super::BootInfo)
    }
}

pub mod __macro {
    pub use super::limine_glue::{
        BaseRevision, ExecutableAddressRequest, FramebufferRequest, HhdmRequest,
        MemoryMapRequest, ModuleRequest, Requests, RequestsEndMarker,
        RequestsStartMarker, RsdpRequest, base_revision,
    };
}

pub use limine_glue::enter;

#[macro_export]
macro_rules! entry {
    ($path:path) => {
        const __WOST_ENTRY_SIGNATURE: fn($crate::boot::BootInfo) -> ! = $path;

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_start_marker")]
        static __WOST_REQUESTS_START: $crate::boot::__macro::RequestsStartMarker =
            $crate::boot::__macro::RequestsStartMarker::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_BASE_REVISION: $crate::boot::__macro::BaseRevision =
            $crate::boot::__macro::base_revision();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_FRAMEBUFFER: $crate::boot::__macro::FramebufferRequest =
            $crate::boot::__macro::FramebufferRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MEMORY_MAP: $crate::boot::__macro::MemoryMapRequest =
            $crate::boot::__macro::MemoryMapRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_DIRECT_MAP: $crate::boot::__macro::HhdmRequest =
            $crate::boot::__macro::HhdmRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_IMAGE: $crate::boot::__macro::ExecutableAddressRequest =
            $crate::boot::__macro::ExecutableAddressRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_RSDP: $crate::boot::__macro::RsdpRequest =
            $crate::boot::__macro::RsdpRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests")]
        static __WOST_MODULES: $crate::boot::__macro::ModuleRequest =
            $crate::boot::__macro::ModuleRequest::new();

        #[used]
        #[allow(unsafe_code)]
        #[unsafe(link_section = ".requests_end_marker")]
        static __WOST_REQUESTS_END: $crate::boot::__macro::RequestsEndMarker =
            $crate::boot::__macro::RequestsEndMarker::new();

        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        extern "C" fn kmain() -> ! {
            $crate::boot::enter(
                $crate::boot::__macro::Requests {
                    base_revision: &__WOST_BASE_REVISION,
                    framebuffer: &__WOST_FRAMEBUFFER,
                    memory_map: &__WOST_MEMORY_MAP,
                    hhdm: &__WOST_DIRECT_MAP,
                    image: &__WOST_IMAGE,
                    rsdp: &__WOST_RSDP,
                    modules: &__WOST_MODULES,
                },
                __WOST_ENTRY_SIGNATURE,
            )
        }
    };
}
"#
    }

    #[test]
    fn compiler_semantic_environment_is_refused_before_the_audit() {
        for name in [
            "RUSTC",
            "rustflags",
            "RUSTC_WORKSPACE_WRAPPER",
            "CARGO_BUILD_TARGET",
            "cargo_encoded_rustflags",
            "CARGO_PROFILE_DEV_DEBUG",
            "CARGO_TARGET_X86_64_UNKNOWN_NONE_RUSTFLAGS",
        ] {
            assert!(
                rejected_audit_environment_variable(name),
                "{name} escaped the compiler environment boundary"
            );
        }
        for name in [
            "PATH",
            "RUST_LOG",
            "RUST_BACKTRACE",
            "RUSTUP_HOME",
            "RUSTUP_TOOLCHAIN",
            "CARGO_HOME",
            "CARGO_TARGET_DIR",
            "CARGO_TERM_COLOR",
            "CARGO_BIN_NAME",
            "CARGO_CRATE_NAME",
            "CARGO_MANIFEST_DIR",
            "CARGO_MANIFEST_PATH",
            "CARGO_PKG_NAME",
            "CARGO_PRIMARY_PACKAGE",
        ] {
            assert!(
                !rejected_audit_environment_variable(name),
                "{name} was rejected despite not changing compiled semantics"
            );
        }
    }

    #[test]
    fn cargo_commands_remove_ambient_rust_and_cargo_semantics_before_pinning_their_own() {
        let mut command = Command::new("cargo");
        for name in [
            "RUSTFLAGS",
            "RUSTC_WRAPPER",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
            "TARGET",
        ] {
            command.env(name, "hostile");
        }
        remove_ambient_cargo_semantics(
            &mut command,
            [
                "RUSTFLAGS",
                "RUSTC_WRAPPER",
                "CARGO_ENCODED_RUSTFLAGS",
                "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
                "TARGET",
            ]
            .into_iter()
            .map(std::ffi::OsString::from),
        );
        let environment = command.get_envs().collect::<BTreeMap<_, _>>();
        for name in [
            "RUSTFLAGS",
            "RUSTC_WRAPPER",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_TARGET_X86_64_UNKNOWN_NONE_LINKER",
            "TARGET",
        ] {
            assert_eq!(environment.get(std::ffi::OsStr::new(name)), Some(&None));
        }
    }

    #[test]
    fn every_cargo_subprocess_pins_the_adjacent_compiler_and_disables_wrappers() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let cargo = std::fs::canonicalize(env!("CARGO")).expect("canonical Cargo executable");
        let expected_compiler =
            std::fs::canonicalize(cargo.parent().unwrap().join(if cfg!(windows) {
                "rustc.exe"
            } else {
                "rustc"
            }))
            .expect("canonical adjacent compiler");
        assert_eq!(compiler.path, expected_compiler);
        let private_home = repository.join("private-cargo-home");
        let private_registry = repository.join("private-registry");
        let mut command =
            audited_cargo_command(repository, &private_home, &private_registry, &compiler)
                .expect("audited Cargo command");
        let kernel_rustflags = cargo_encoded_rustflags(Some(crate::paths::KERNEL_TARGET))
            .expect("kernel rustflags contract");
        assert_eq!(cargo_encoded_rustflags(None).unwrap(), "");
        command.env("CARGO_ENCODED_RUSTFLAGS", kernel_rustflags);
        let environment = command
            .get_envs()
            .map(|(name, value)| (name.to_os_string(), value.map(ToOwned::to_owned)))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            environment.get(std::ffi::OsStr::new("RUSTC")),
            Some(&Some(compiler.path.clone().into_os_string()))
        );
        for wrapper in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
            assert_eq!(
                environment.get(std::ffi::OsStr::new(wrapper)),
                Some(&Some(std::ffi::OsString::new()))
            );
        }
        assert_eq!(
            environment.get(std::ffi::OsStr::new("CARGO_HOME")),
            Some(&Some(private_home.into_os_string()))
        );
        assert_eq!(
            environment.get(std::ffi::OsStr::new("PATH")),
            Some(&Some(compiler.tool_path.clone()))
        );
        assert!(command.get_args().any(|argument| {
            argument.to_string_lossy().contains(".linker=")
                && argument.to_string_lossy().contains("rust-lld")
        }));
        assert_eq!(
            environment.get(std::ffi::OsStr::new("CARGO_ENCODED_RUSTFLAGS")),
            Some(&Some(std::ffi::OsString::from(kernel_rustflags)))
        );
        assert!(command.get_args().any(|argument| {
            argument
                .to_string_lossy()
                .starts_with("source.api-audit-registry.directory=")
        }));
    }

    #[test]
    fn checksum_verified_archives_form_an_immutable_isolated_cargo_source() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-private-cargo-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let cargo = root.join("audit-cargo");
        let home = cargo.join("home");
        let registry = cargo.join("registry-source");
        std::fs::create_dir_all(root.join("src")).expect("private-Cargo fixture source");
        std::fs::create_dir_all(&home).expect("private-Cargo home");
        std::fs::create_dir_all(&registry).expect("private-Cargo directory source");
        std::fs::write(home.join("config.toml"), "\n").expect("empty private config");
        std::fs::create_dir_all(root.join(".cargo")).expect("hostile ancestor config directory");
        std::fs::write(
            root.join(".cargo/config.toml"),
            "[build]\nrustflags = [\"--cfg\", \"manufactured\"]\n",
        )
        .expect("hostile ancestor config");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nanyhow = \"=1.0.104\"\n",
        )
        .expect("private-Cargo fixture manifest");
        std::fs::write(
            root.join("Cargo.lock"),
            r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470"

[[package]]
name = "fixture"
version = "0.1.0"
dependencies = [
 "anyhow",
]
"#,
        )
        .expect("private-Cargo fixture lockfile");
        std::fs::write(
            root.join("src/lib.rs"),
            "#[cfg(manufactured)] compile_error!(\"ancestor Cargo config escaped isolation\");\npub fn live() {}\n",
        )
        .expect("private-Cargo fixture library");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let execution = cargo_execution_root(&root).expect("filesystem-root execution boundary");
        materialize_locked_registry_source(&root, &home, &registry, &execution, &compiler)
            .expect("materialize verified archive");
        let baseline = cargo_input_snapshot(&cargo, &home, &registry)
            .expect("private Cargo authority baseline");
        let result = audited_cargo_command(&execution, &home, &registry, &compiler)
            .expect("isolated Cargo check")
            .env("CARGO_TARGET_DIR", root.join("target"))
            .env("CARGO_ENCODED_RUSTFLAGS", "")
            .args(["check", "--offline", "--locked"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("run isolated Cargo check");
        assert!(
            result.status.success(),
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            cargo_input_snapshot(&cargo, &home, &registry)
                .expect("unchanged private Cargo authority"),
            baseline
        );
        let package = locked_registry_packages(&root.join("Cargo.lock"))
            .expect("fixture locked package")
            .pop()
            .expect("one registry package");
        let library = registry.join("anyhow-1.0.104").join("src/lib.rs");
        let mut tampered = std::fs::read_to_string(&library).expect("materialized library");
        tampered.push_str("\n// tampered\n");
        std::fs::write(&library, tampered).expect("tamper materialized source");
        assert!(
            validate_materialized_registry_package(&registry.join("anyhow-1.0.104"), &package)
                .is_err()
        );
        std::fs::remove_dir_all(root).expect("remove private-Cargo fixture");
    }

    #[test]
    fn authenticated_registry_source_cannot_mint_a_diagnostic_marker() {
        for (label, source) in [
            (
                "raw marker",
                format!("#[deprecated(note = \"{MARKER}0\")]\npub fn forged() {{}}\n"),
            ),
            (
                "decoded marker",
                "#[deprecated(note = \"wizard-api-audit:\\x30\")]\npub fn forged() {}\n"
                    .to_string(),
            ),
        ] {
            let root = std::env::temp_dir().join(format!(
                "wizard-api-registry-marker-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let package_root = root.join("fixture-1.0.0");
            std::fs::create_dir_all(package_root.join("src"))
                .expect("registry marker fixture directory");
            let manifest = b"[package]\nname = \"fixture\"\nversion = \"1.0.0\"\n";
            std::fs::write(package_root.join("Cargo.toml"), manifest)
                .expect("registry marker manifest");
            std::fs::write(package_root.join("src/lib.rs"), source.as_bytes())
                .expect("registry marker source");
            let manifest_digest = crate::sha256::hex(&crate::sha256::digest(manifest));
            let source_digest = crate::sha256::hex(&crate::sha256::digest(source.as_bytes()));
            let package_digest = "00".repeat(32);
            std::fs::write(
                package_root.join(".cargo-checksum.json"),
                format!(
                    "{{\"$comment\":\"fixture\",\"files\":{{\"Cargo.toml\":\"{manifest_digest}\",\"src/lib.rs\":\"{source_digest}\"}},\"package\":\"{package_digest}\"}}"
                ),
            )
            .expect("registry checksum metadata");
            let package = LockedRegistryPackage {
                name: "fixture".to_string(),
                version: "1.0.0".to_string(),
                checksum: package_digest,
            };

            let error =
                validate_materialized_registry_package(&package_root, &package).expect_err(label);
            assert!(
                format!("{error:#}").contains("reserved diagnostic marker"),
                "{label}: {error:#}"
            );
            std::fs::remove_dir_all(root).expect("remove registry marker fixture");
        }
    }

    #[test]
    fn compiler_graphs_have_distinct_stable_build_directories() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-graph-build-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let build = root.join("build");
        std::fs::create_dir_all(&build).expect("graph build root");
        let graph_zero = graph_build_directory(&build, 0).expect("graph zero build");
        let graph_one = graph_build_directory(&build, 1).expect("graph one build");
        let graph_one_again = graph_build_directory(&build, 1).expect("stable graph one build");
        std::fs::remove_dir_all(root).expect("remove graph build fixture");

        assert!(
            graph_zero != graph_one,
            "distinct compiler graphs shared incremental state"
        );
        assert!(
            graph_one == graph_one_again,
            "one compiler graph changed build identity between rounds"
        );
    }

    #[test]
    fn instrumented_rounds_cannot_change_build_script_output_authority() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-build-authority-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let package = root.join("debug/build/fixture-build-id");
        let out = package.join("out");
        std::fs::create_dir_all(&out).expect("build-script OUT_DIR fixture");
        std::fs::write(package.join("output"), "cargo:rerun-if-changed=fixed\n")
            .expect("build-script output fixture");
        let generated = out.join("generated.rs");
        std::fs::write(&generated, "pub const VALUE: usize = 1;\n")
            .expect("generated source fixture");

        let baseline = build_script_authority_snapshot(&root).expect("pristine authority");
        require_build_script_authority_unchanged(&root, &baseline, "fixture graph")
            .expect("unchanged authority");
        std::fs::write(&generated, "pub const VALUE: usize = 2;\n")
            .expect("marker-sensitive generated source");
        let error = require_build_script_authority_unchanged(&root, &baseline, "fixture graph")
            .expect_err("changed OUT_DIR source must be refused");
        assert!(format!("{error:#}").contains("changed build-script output"));

        std::fs::remove_dir_all(root).expect("remove build-authority fixture");
    }

    #[test]
    fn pristine_generated_source_cannot_mint_a_diagnostic_marker() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-generated-marker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let out = root.join("debug/build/fixture-build-id/out");
        std::fs::create_dir_all(&out).expect("generated marker OUT_DIR");
        std::fs::write(
            out.join("generated.rs"),
            "#[deprecated(note = \"wizard-api-audit:\\x30\")]\npub fn forged() {}\n",
        )
        .expect("marker-sensitive generated source");

        let error = build_script_authority_snapshot(&root)
            .expect_err("decoded marker in pristine OUT_DIR must be refused");
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove generated marker fixture");
    }

    #[test]
    fn compiler_binaries_and_target_metadata_cannot_mint_a_diagnostic_marker() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-sysroot-marker-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let bin = root.join("bin");
        let host_lib = root.join("lib/rustlib/fixture-host/lib");
        let kernel_lib = root.join("lib/rustlib/fixture-kernel/lib");
        let codegen = root.join("lib/rustlib/fixture-host/codegen-backends/fixture.dll");
        std::fs::create_dir_all(&bin).expect("compiler fixture bin");
        std::fs::create_dir_all(&host_lib).expect("host sysroot fixture");
        std::fs::create_dir_all(&kernel_lib).expect("kernel sysroot fixture");
        std::fs::create_dir_all(codegen.parent().expect("codegen backend parent"))
            .expect("codegen backend fixture directory");
        let cargo = bin.join(if cfg!(windows) { "cargo.exe" } else { "cargo" });
        let compiler = bin.join(if cfg!(windows) { "rustc.exe" } else { "rustc" });
        let metadata = host_lib.join("libcore-fixture.rmeta");
        std::fs::write(&cargo, b"fixed cargo").expect("fixture Cargo");
        std::fs::write(&compiler, b"fixed rustc").expect("fixture rustc");
        std::fs::write(&metadata, b"fixed metadata").expect("fixture host metadata");
        std::fs::write(&codegen, b"fixed codegen backend").expect("fixture codegen backend");
        std::fs::write(
            kernel_lib.join("libcore-fixture.rlib"),
            b"fixed kernel library",
        )
        .expect("fixture kernel library");
        let targets = vec!["fixture-host".to_string(), "fixture-kernel".to_string()];
        let baseline = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
            .expect("marker-free compiler authority");

        for (label, path, original) in [
            (
                "compiler binary",
                compiler.as_path(),
                b"fixed rustc".as_slice(),
            ),
            (
                "precompiled metadata",
                metadata.as_path(),
                b"fixed metadata".as_slice(),
            ),
            (
                "codegen backend",
                codegen.as_path(),
                b"fixed codegen backend".as_slice(),
            ),
        ] {
            std::fs::write(path, format!("hostile {MARKER}0")).expect("inject compiler marker");
            let error =
                compiler_input_snapshot(&root, &cargo, &compiler, &targets).expect_err(label);
            assert!(
                format!("{error:#}").contains("reserved diagnostic marker"),
                "{label}: {error:#}"
            );
            std::fs::write(path, original).expect("restore compiler input");
        }

        std::fs::write(&codegen, b"changed codegen backend").expect("change codegen backend");
        let current = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
            .expect("changed marker-free codegen authority");
        let error = require_compiler_snapshot_unchanged(&baseline, &current)
            .expect_err("changed codegen backend must be refused");
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains("codegen-backends/fixture.dll"),
            "{error:#}"
        );
        std::fs::write(&codegen, b"fixed codegen backend").expect("restore codegen backend");

        std::fs::write(&metadata, b"changed metadata").expect("change compiler input");
        let current = compiler_input_snapshot(&root, &cargo, &compiler, &targets)
            .expect("changed marker-free compiler authority");
        let error = require_compiler_snapshot_unchanged(&baseline, &current)
            .expect_err("changed compiler input must be refused");
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains("libcore-fixture.rmeta"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove compiler marker fixture");
    }

    #[test]
    fn compiler_graphs_are_offline_and_lockfile_frozen() {
        const PACKAGES: &[&str] = &["fixture"];
        let pass = gate::CargoPass {
            name: "fixture",
            packages: PACKAGES,
            target: None,
            features: None,
        };
        let plan = CompilationPlan {
            pass,
            target: "fixture-host".to_string(),
            kind: RootKind::Production,
            mode: CompilationMode::RegisteredPass,
        };
        let args = cargo_arguments(&plan);

        assert_eq!(
            &args[..4],
            ["check", "--offline", "--locked", "--message-format=json"]
        );
        assert_eq!(args.iter().filter(|arg| *arg == "--locked").count(), 1);
        assert!(
            args.windows(2)
                .any(|args| args == ["--target", "fixture-host"])
        );
    }

    #[test]
    fn compiler_graphs_never_merge_distinct_cargo_targets() {
        let source = include_str!("api.rs");
        let compilation = source
            .split_once("fn compile_graphs(")
            .and_then(|(_, source)| source.split_once("#[cfg(test)]"))
            .map(|(source, _)| source)
            .expect("API compilation implementation");
        assert!(
            !compilation.contains("--all-targets"),
            "mutually exclusive Cargo targets shared one evidence graph"
        );
    }

    #[test]
    fn cargo_metadata_not_a_conventional_filename_defines_a_target_root() {
        let root = fixture("#![warn(deprecated)]\npub fn orphan() {}\nfn main() { orphan(); }\n");
        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture source");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let metadata = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture_core\",\"kind\":[\"lib\"],\"crate_types\":[\"lib\"],\"src_path\":\"{}\",\"test\":true}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            escaped(&source)
        );
        let catalog = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
            .expect("fixture metadata");
        let inventory =
            Inventory::discover_from_catalog(&root, &catalog).expect("metadata inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &["--crate-type", "lib"]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read library diagnostics");
        }
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "orphan"),
            "a library at src/main.rs acquired an executable root"
        );
        std::fs::remove_dir_all(root).expect("remove metadata fixture");
    }

    #[test]
    fn active_procedural_macros_are_outside_the_audit_trust_boundary() {
        let root = fixture("fn main() {}\n");
        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture source");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let packages = format!(
            "[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"derive-id\",\"name\":\"untrusted-derive\",\"targets\":[{{\"name\":\"untrusted_derive\",\"kind\":[\"proc-macro\"],\"crate_types\":[\"proc-macro\"],\"src_path\":\"outside.rs\",\"test\":false}}]}}]",
            escaped(&source)
        );
        let inactive = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            packages
        );
        CargoCatalog::parse(&root, &root, inactive.as_bytes(), &["fixture"], true)
            .expect("an unresolved procedural macro is inactive");
        let active = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"derive-id\"],\"deps\":[{{\"name\":\"untrusted_derive\",\"pkg\":\"derive-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"derive-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            packages
        );
        let error = CargoCatalog::parse(&root, &root, active.as_bytes(), &["fixture"], true)
            .expect_err("active procedural macro must be refused");
        assert!(format!("{error:#}").contains("untrusted-derive"));
        std::fs::remove_dir_all(root).expect("remove procedural-macro fixture");
    }

    #[test]
    fn unreviewed_dependency_build_scripts_are_outside_the_audit_authority() {
        let root = fixture("fn main() {}\n");
        let build = root.join("evil-build.rs");
        std::fs::write(
            &build,
            r#"fn main() {
    let source = std::fs::read_to_string("libs/fixture/src/main.rs").unwrap();
    let generated = std::env::temp_dir().join("api-audit-generated.rs");
    std::fs::write(generated, source).unwrap();
    println!("cargo:rerun-if-changed=fixed");
}"#,
        )
        .expect("hostile dependency build script");
        let compiled = Command::new("rustc")
            .current_dir(&root)
            .args([
                "--edition=2024",
                "evil-build.rs",
                "--emit=metadata",
                "-o",
                "evil-build.rmeta",
            ])
            .output()
            .expect("compile hostile dependency build script");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        std::fs::write(
            root.join("Cargo.lock"),
            r#"version = 4

[[package]]
name = "anyhow"
version = "1.0.104"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "330a5ed07fa54e4702c9d6c4174f74427fc0ef6e214bbd677ae50a5099946470"
"#,
        )
        .expect("fixture lockfile");

        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture source");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let metadata = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"evil-id\",\"name\":\"evil-build\",\"version\":\"0.1.0\",\"source\":null,\"targets\":[{{\"name\":\"build-script-build\",\"kind\":[\"custom-build\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":false}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"evil-id\"],\"deps\":[{{\"name\":\"evil_build\",\"pkg\":\"evil-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"evil-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            escaped(&source),
            escaped(&build),
        );
        let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
            .expect_err("unreviewed dependency build script must be refused");
        assert!(format!("{error:#}").contains("not a checksum-bound crates.io package"));
        std::fs::remove_dir_all(root).expect("remove dependency-build fixture");
    }

    #[test]
    fn active_path_packages_are_outside_the_compiler_input_authority() {
        let root = fixture("fn main() {}\n");
        let external = root.with_extension("external-package");
        std::fs::create_dir_all(external.join("src")).expect("external package directory");
        std::fs::write(
            external.join("Cargo.toml"),
            "[package]\nname = \"external\"\nversion = \"0.1.0\"\n",
        )
        .expect("external package manifest");
        std::fs::write(external.join("src/lib.rs"), "pub fn forged() {}\n")
            .expect("external package source");
        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture source");
        let external_manifest = std::fs::canonicalize(external.join("Cargo.toml"))
            .expect("canonical external manifest");
        let external_source =
            std::fs::canonicalize(external.join("src/lib.rs")).expect("canonical external source");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let metadata = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"external-id\",\"name\":\"external\",\"version\":\"0.1.0\",\"source\":null,\"manifest_path\":\"{}\",\"targets\":[{{\"name\":\"external\",\"kind\":[\"lib\"],\"crate_types\":[\"lib\"],\"src_path\":\"{}\",\"test\":true}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"external-id\"],\"deps\":[{{\"name\":\"external\",\"pkg\":\"external-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"external-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            escaped(&source),
            escaped(&external_manifest),
            escaped(&external_source),
        );

        let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
            .expect_err("active path package must be refused");
        assert!(
            format!("{error:#}").contains("not a checksum-bound crates.io package"),
            "{error:#}"
        );
        std::fs::remove_dir_all(external).expect("remove external package fixture");
        std::fs::remove_dir_all(root).expect("remove path-package fixture");
    }

    #[test]
    fn active_dependencies_cannot_replace_language_crate_roots() {
        let root = fixture("fn main() {}\n");
        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let source = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture source");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let packages = format!(
            "[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}}]}},{{\"id\":\"evil-id\",\"name\":\"evil\",\"targets\":[]}}]",
            escaped(&source)
        );
        for alias in RESERVED_LANGUAGE_ROOTS {
            let metadata = format!(
                "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":{},\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[\"evil-id\"],\"deps\":[{{\"name\":\"{}\",\"pkg\":\"evil-id\",\"dep_kinds\":[{{\"kind\":null,\"target\":null}}]}}]}},{{\"id\":\"evil-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
                escaped(&canonical),
                packages,
                alias
            );
            let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
                .expect_err("reserved dependency alias must be refused");
            assert!(format!("{error:#}").contains(alias), "{error:#}");
        }
        std::fs::remove_dir_all(root).expect("remove reserved-root metadata fixture");
    }

    #[test]
    fn only_the_exact_kernel_linker_build_script_is_trusted() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let kernel = CargoTarget {
            package: crate::paths::KERNEL_PACKAGE.to_string(),
            package_id: "kernel-id".to_string(),
            name: "build-script-build".to_string(),
            relative: PathBuf::from("kernel/build.rs"),
            kind: CargoTargetKind::CustomBuild,
            test: false,
            executable: true,
        };
        validate_workspace_build_script(repository, crate::paths::KERNEL_PACKAGE, &kernel)
            .expect("current kernel build-script contract");

        let root = fixture("#[cfg(manufactured)] pub fn hidden() {}\nfn main() {}\n");
        let build_script = r#"fn main() {
    let source = std::fs::read_to_string("libs/fixture/src/main.rs").unwrap();
    if source.contains("wizard-api-audit:") {
        println!("cargo:rustc-cfg=manufactured");
    }
}
"#;
        std::fs::write(root.join("build.rs"), build_script).expect("hostile build script");
        let compiled = Command::new("rustc")
            .current_dir(&root)
            .args([
                "--edition=2024",
                "build.rs",
                "--emit=metadata",
                "-o",
                "build-script.rmeta",
            ])
            .output()
            .expect("compile hostile build script");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let canonical = std::fs::canonicalize(&root).expect("canonical fixture root");
        let main = std::fs::canonicalize(root.join("libs/fixture/src/main.rs"))
            .expect("canonical fixture main");
        let build =
            std::fs::canonicalize(root.join("build.rs")).expect("canonical fixture build script");
        let escaped = |path: &Path| {
            path.to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
        };
        let metadata = format!(
            "{{\"version\":1,\"workspace_root\":\"{}\",\"workspace_members\":[\"fixture-id\"],\"packages\":[{{\"id\":\"fixture-id\",\"name\":\"fixture\",\"targets\":[{{\"name\":\"fixture\",\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":true}},{{\"name\":\"build-script-build\",\"kind\":[\"custom-build\"],\"crate_types\":[\"bin\"],\"src_path\":\"{}\",\"test\":false}}]}}],\"resolve\":{{\"nodes\":[{{\"id\":\"fixture-id\",\"dependencies\":[],\"deps\":[]}}]}}}}",
            escaped(&canonical),
            escaped(&main),
            escaped(&build)
        );
        let error = CargoCatalog::parse(&root, &root, metadata.as_bytes(), &["fixture"], true)
            .expect_err("marker-sensitive workspace build script must be refused");
        assert!(format!("{error:#}").contains("exact build-script contract"));
        std::fs::remove_dir_all(root).expect("remove build-script fixture");
    }

    #[test]
    fn procedural_attribute_and_derive_syntax_is_refused_before_inventory() {
        for source in [
            "#[generate_api]\npub struct Seed;\nfn main() {}\n",
            "#[derive(tool::GenerateApi)]\npub struct Seed;\nfn main() {}\n",
        ] {
            let root = fixture(source);
            let error = Inventory::discover(&root)
                .expect_err("procedural attribute syntax must be refused");
            assert!(
                format!("{error:#}").contains("built-in trust boundary"),
                "{error:#}"
            );
            std::fs::remove_dir_all(root).expect("remove attribute fixture");
        }
    }

    #[test]
    fn cfg_attr_cannot_hide_a_semantic_runtime_root() {
        for attribute in [
            "unsafe(no_mangle)",
            "unsafe(export_name = \"exported\")",
            "used",
            "global_allocator",
        ] {
            let declaration = if attribute == "used" || attribute == "global_allocator" {
                format!("#[cfg_attr(all(), {attribute})]\npub static EXPORTED: usize = 0;\n")
            } else {
                format!(
                    "#[cfg_attr(all(), {attribute})]\npub extern \"C\" fn exported() {{ api(); }}\n"
                )
            };
            let root = fixture(&format!(
                "#![warn(deprecated)]\npub fn api() {{}}\n{declaration}fn main() {{}}\n"
            ));
            let compiled = Command::new("rustc")
                .current_dir(&root)
                .args([
                    "--edition=2024",
                    "libs/fixture/src/main.rs",
                    "--emit=metadata",
                    "-o",
                    "fixture.rmeta",
                ])
                .output()
                .expect("compile conditional-root fixture");
            if attribute != "global_allocator" {
                assert!(
                    compiled.status.success(),
                    "{}",
                    String::from_utf8_lossy(&compiled.stderr)
                );
            }
            let error = Inventory::discover(&root)
                .expect_err("a conditional semantic runtime root must be refused");
            assert!(
                format!("{error:#}").contains("conditionally controls a semantic runtime root"),
                "{error:#}"
            );
            std::fs::remove_dir_all(root).expect("remove conditional-root fixture");
        }
    }

    #[test]
    fn test_harness_graphs_do_not_root_the_target_program_main() {
        assert!(
            !RootSet {
                production: true,
                verification: false,
            }
            .contains(RootKind::TestHarness)
        );
        assert!(
            RootSet {
                production: false,
                verification: true,
            }
            .contains(RootKind::TestHarness)
        );
    }

    #[test]
    fn cargo_test_artifacts_must_prove_the_standard_harness() {
        let selected = CargoTarget {
            package: "fixture".to_string(),
            package_id: "fixture-id".to_string(),
            name: "fixture_core".to_string(),
            relative: PathBuf::from("src/lib.rs"),
            kind: CargoTargetKind::Library,
            test: true,
            executable: false,
        };
        let artifact = |test| {
            format!(
                "{{\"reason\":\"compiler-artifact\",\"package_id\":\"fixture-id\",\"target\":{{\"name\":\"fixture_core\",\"kind\":[\"lib\"]}},\"profile\":{{\"test\":{test}}}}}\n"
            )
        };
        cargo_test_output(artifact("true").as_bytes(), &selected)
            .expect("standard harness artifact");
        let error = cargo_test_output(artifact("false").as_bytes(), &selected)
            .expect_err("custom harness must be refused");
        assert!(format!("{error:#}").contains("custom test harness"));
    }

    #[test]
    fn cargo_diagnostic_origins_bind_markers_to_the_exact_package_and_target() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-diagnostic-origin-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = root.join("libs/fixture/src/main.rs");
        std::fs::create_dir_all(source.parent().unwrap()).expect("first-party source directory");
        std::fs::create_dir_all(root.join("tools/evil/src")).expect("sibling package source");
        std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"libs/fixture\", \"tools/evil\"]\nresolver = \"3\"\n",
        )
        .expect("origin fixture workspace manifest");
        std::fs::write(
            root.join("libs/fixture/Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nevil = { path = \"../../tools/evil\" }\n",
        )
        .expect("origin fixture package manifest");
        std::fs::write(
            root.join("tools/evil/Cargo.toml"),
            "[package]\nname = \"evil\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("sibling package manifest");
        std::fs::write(
            &source,
            "pub fn called() {}\npub fn caller() { called(); }\nfn main() { caller(); }\n",
        )
        .expect("first-party origin fixture source");
        std::fs::write(
            root.join("tools/evil/src/lib.rs"),
            format!("include!({:?});\n", source.to_string_lossy()),
        )
        .expect("sibling package include");

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let metadata = isolated_test_cargo_command(&root, &compiler)
            .args(["metadata", "--format-version=1", "--offline"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("origin fixture Cargo metadata");
        assert!(
            metadata.status.success(),
            "{}{}",
            String::from_utf8_lossy(&metadata.stdout),
            String::from_utf8_lossy(&metadata.stderr)
        );
        let catalog = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
            .expect("origin fixture catalog");
        let inventory =
            Inventory::discover_from_catalog(&root, &catalog).expect("origin fixture inventory");
        let called = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "called")
            .expect("called API")
            .id;
        let expected_package = inventory.files[inventory.nodes[called].file]
            .origin
            .package_id
            .clone();
        let mut legitimate = Uses::default();
        let mut observed_hostile = false;
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument origin fixture");
            let result = isolated_test_cargo_command(&root, &compiler)
                .env("CARGO_TARGET_DIR", root.join("target"))
                .env("CARGO_ENCODED_RUSTFLAGS", "")
                .args([
                    "check",
                    "--offline",
                    "--locked",
                    "--message-format=json",
                    "--package",
                    "fixture",
                ])
                .arg("--manifest-path")
                .arg(root.join("Cargo.toml"))
                .output()
                .expect("compile origin fixture");
            assert!(
                result.status.success(),
                "{}{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
            let diagnostics = cargo_compilation_output(&result.stdout, None)
                .expect("structured origin diagnostics");
            let matching = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.origin.package_id == expected_package)
                .cloned()
                .collect::<Vec<_>>();
            let hostile = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.origin.package_id != expected_package)
                .cloned()
                .collect::<Vec<_>>();
            if !hostile.is_empty() {
                observed_hostile = true;
                let error = Uses::default()
                    .observe_cargo(
                        RootKind::Production,
                        &inventory,
                        &root,
                        &hostile,
                        &instrumentation,
                    )
                    .expect_err("wrong-package diagnostic must be refused");
                assert!(format!("{error:#}").contains("Cargo origin"), "{error:#}");
            }
            legitimate
                .observe_cargo(
                    RootKind::Production,
                    &inventory,
                    &root,
                    &matching,
                    &instrumentation,
                )
                .expect("matching-package diagnostics remain authoritative");
        }
        assert!(
            observed_hostile,
            "sibling package include emitted no hostile marker diagnostic"
        );
        assert!(
            !legitimate
                .dead(&inventory)
                .iter()
                .any(|node| node.id == called),
            "matching-origin diagnostics did not preserve legitimate liveness"
        );
        std::fs::remove_dir_all(root).expect("remove diagnostic-origin fixture");
    }

    #[test]
    fn cross_crate_entry_diagnostics_require_the_exact_compiler_origin_bridge() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-entry-origin-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("libs")).expect("empty entry fixture libs directory");
        std::fs::create_dir_all(root.join("wost/src/boot"))
            .expect("entry fixture wost source directory");
        std::fs::create_dir_all(root.join("kernel/src"))
            .expect("entry fixture kernel source directory");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"wost\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .expect("entry fixture workspace manifest");
        std::fs::write(
            root.join("wost/Cargo.toml"),
            "[package]\nname = \"wost\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("entry fixture wost manifest");
        std::fs::write(
            root.join("kernel/Cargo.toml"),
            "[package]\nname = \"wizard-kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nwost = { path = \"../wost\" }\n",
        )
        .expect("entry fixture kernel manifest");
        std::fs::write(root.join("wost/src/lib.rs"), "pub mod boot;\n")
            .expect("entry fixture wost root");
        std::fs::write(
            root.join("wost/src/boot/mod.rs"),
            validated_entry_fixture_source(),
        )
        .expect("entry fixture macro source");
        std::fs::write(
            root.join("kernel/src/main.rs"),
            "::wost::entry!(kernel_main);\nmod other;\nfn kernel_main(_: wost::boot::BootInfo) -> ! { loop {} }\nfn main() {}\n",
        )
        .expect("entry fixture kernel source");
        std::fs::write(root.join("kernel/src/other.rs"), "pub fn dormant() {}\n")
            .expect("entry fixture secondary kernel source");

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let metadata = isolated_test_cargo_command(&root, &compiler)
            .args(["metadata", "--format-version=1", "--offline"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("entry fixture Cargo metadata");
        assert!(
            metadata.status.success(),
            "{}{}",
            String::from_utf8_lossy(&metadata.stdout),
            String::from_utf8_lossy(&metadata.stderr)
        );
        let catalog = CargoCatalog::parse(
            &root,
            &root,
            &metadata.stdout,
            &["wost", crate::paths::KERNEL_PACKAGE],
            true,
        )
        .expect("entry fixture Cargo catalog");
        let targets = catalog.targets().cloned().collect::<Vec<_>>();
        let untrusted_inventory = Inventory::discover_with_targets(&root, &targets)
            .expect("entry fixture inventory without authority");
        assert!(untrusted_inventory.entry.is_none());
        let inventory = Inventory::discover_from_catalog(&root, &catalog)
            .expect("entry fixture catalog-authorized inventory");
        let entry = inventory.entry.expect("entry fixture link");
        let kernel_origin = &inventory.files[entry.file].origin;
        let declaration = &inventory.nodes[entry.macro_declaration];
        let contracts = [
            "boot::BootInfo",
            "boot::limine_glue::base_revision",
            "boot::limine_glue::enter",
            "boot::limine_glue::Requests",
        ]
        .map(|symbol| {
            let id = inventory
                .nodes
                .iter()
                .find(|node| node.symbol == symbol)
                .unwrap_or_else(|| panic!("entry fixture has no `{symbol}` node"))
                .id;
            assert!(
                entry
                    .contract_nodes
                    .iter()
                    .any(|contract| contract.id == id)
            );
            (symbol, id)
        });
        let mut enter_evidence = None;
        for (symbol, id) in contracts {
            let instrumentation = (0..inventory.antichain_rounds)
                .find_map(|round| {
                    let instrumentation = inventory
                        .instrument(&root, round)
                        .expect("instrument entry fixture");
                    instrumentation.ids.contains(&id).then_some(instrumentation)
                })
                .unwrap_or_else(|| panic!("entry fixture has no `{symbol}` instrumentation round"));
            let compiled = isolated_test_cargo_command(&root, &compiler)
                .env("CARGO_TARGET_DIR", root.join("target"))
                .env("CARGO_ENCODED_RUSTFLAGS", "")
                .env("CARGO_BUILD_WARNINGS", "warn")
                .args([
                    "check",
                    "--offline",
                    "--locked",
                    "--message-format=json",
                    "--package",
                    crate::paths::KERNEL_PACKAGE,
                    "--bin",
                    crate::paths::KERNEL_PACKAGE,
                    "--color=never",
                ])
                .arg("--manifest-path")
                .arg(root.join("Cargo.toml"))
                .output()
                .expect("compile entry fixture");
            assert!(
                compiled.status.success(),
                "{symbol}: {}{}",
                String::from_utf8_lossy(&compiled.stdout),
                String::from_utf8_lossy(&compiled.stderr)
            );
            let diagnostics = cargo_compilation_output(&compiled.stdout, None)
                .expect("structured entry fixture diagnostics");
            let crosses = diagnostics
                .iter()
                .filter(|diagnostic| {
                    diagnostic.id == id
                        && diagnostic.origin.package_id == kernel_origin.package_id
                        && inventory.file_index(&root, &diagnostic.reported)
                            == Some(declaration.file)
                })
                .cloned()
                .collect::<Vec<_>>();
            assert!(
                !crosses.is_empty(),
                "entry fixture emitted no kernel-origin `{symbol}` definition diagnostics"
            );
            for cross in &crosses {
                let primary_file = inventory
                    .file_index(&root, &cross.reported)
                    .expect("classified entry definition span");
                let primary_at = corrected_marker_location(
                    primary_file,
                    cross.line,
                    cross.column,
                    &instrumentation,
                )
                .expect("corrected entry definition span");
                assert_eq!(primary_file, declaration.file);
                assert!(declaration.range.contains(primary_at));
                assert!(
                    entry
                        .contract_nodes
                        .iter()
                        .find(|contract| contract.id == id)
                        .is_some_and(|contract| contract.primary.contains(primary_at)),
                    "`{symbol}` diagnostic escaped its validated macro use site"
                );
                let expansion = cross
                    .immediate_expansion
                    .as_ref()
                    .expect("entry diagnostic immediate expansion");
                assert!(expansion.terminal);
                assert_eq!(expansion.macro_decl_name, "::wost::entry!");
                let expansion_file = inventory
                    .file_index(&root, &expansion.location.reported)
                    .expect("classified entry invocation span");
                let expansion_at = corrected_marker_location(
                    expansion_file,
                    expansion.location.line,
                    expansion.location.column,
                    &instrumentation,
                )
                .expect("corrected entry invocation span");
                assert_eq!(expansion_file, entry.file);
                assert_eq!(expansion_at, entry.invocation.start);
                assert!(
                    !cargo_diagnostic_origin_matches(cross, &inventory, &root, primary_file)
                        .expect("entry primary origin comparison")
                );
                assert!(
                    cargo_diagnostic_origin_matches(cross, &inventory, &root, expansion_file)
                        .expect("entry expansion origin comparison")
                );

                let no_authority = Uses::default()
                    .observe_cargo(
                        RootKind::Production,
                        &untrusted_inventory,
                        &root,
                        std::slice::from_ref(cross),
                        &instrumentation,
                    )
                    .expect_err("raw target discovery must not carry entry authority");
                assert!(
                    format!("{no_authority:#}").contains("Cargo origin"),
                    "{symbol}: {no_authority:#}"
                );
            }

            let mut uses = Uses::default();
            uses.observe_cargo(
                RootKind::Production,
                &inventory,
                &root,
                &crosses,
                &instrumentation,
            )
            .expect("exact entry origin bridge");
            assert!(
                uses.edges
                    .get(&entry.invocation_node)
                    .is_some_and(|used| used.contains(&id))
            );
            assert!(uses.live(&inventory).contains(&id));
            if symbol == "boot::limine_glue::enter" {
                let [cross] = crosses.as_slice() else {
                    panic!("entry fixture enter emitted {} diagnostics", crosses.len());
                };
                enter_evidence = Some((cross.clone(), instrumentation));
            }
        }
        let (cross, instrumentation) = enter_evidence.expect("entry fixture enter evidence");
        let enter = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "boot::limine_glue::enter")
            .expect("entry fixture enter node")
            .id;

        let refuse = |label: &str, diagnostic: MarkerDiagnostic, evidence: &Instrumentation| {
            let error = Uses::default()
                .observe_cargo(
                    RootKind::Production,
                    &inventory,
                    &root,
                    &[diagnostic],
                    evidence,
                )
                .expect_err(label);
            assert!(
                format!("{error:#}").contains("Cargo origin"),
                "{label}: {error:#}"
            );
        };

        let mut missing = cross.clone();
        missing.immediate_expansion = None;
        refuse("missing entry expansion", missing, &instrumentation);

        let mut nonterminal = cross.clone();
        nonterminal
            .immediate_expansion
            .as_mut()
            .expect("entry expansion")
            .terminal = false;
        refuse("nonterminal entry expansion", nonterminal, &instrumentation);

        let mut wrong_macro = cross.clone();
        wrong_macro
            .immediate_expansion
            .as_mut()
            .expect("entry expansion")
            .macro_decl_name = "evil::entry!".to_string();
        refuse("wrong entry macro identity", wrong_macro, &instrumentation);

        let mut wrong_callsite = cross.clone();
        wrong_callsite
            .immediate_expansion
            .as_mut()
            .expect("entry expansion")
            .location
            .line += 1;
        refuse(
            "wrong entry expansion callsite",
            wrong_callsite,
            &instrumentation,
        );

        let mut wrong_expansion_file = cross.clone();
        wrong_expansion_file
            .immediate_expansion
            .as_mut()
            .expect("entry expansion")
            .location
            .reported = root.join("kernel/src/other.rs").display().to_string();
        refuse(
            "wrong entry expansion source",
            wrong_expansion_file,
            &instrumentation,
        );

        let mut wrong_primary = cross.clone();
        wrong_primary.line = 1;
        wrong_primary.column = 1;
        refuse(
            "entry primary outside its declaration",
            wrong_primary,
            &instrumentation,
        );

        let other_site = entry
            .contract_nodes
            .iter()
            .find(|contract| contract.id != enter)
            .expect("another validated entry-contract site")
            .primary
            .start;
        let inserted_bytes = instrumentation
            .insertions
            .get(&declaration.file)
            .into_iter()
            .flatten()
            .filter(|insertion| {
                insertion.line == other_site.line && insertion.column <= other_site.column
            })
            .map(|insertion| insertion.bytes)
            .sum::<usize>();
        let mut wrong_contract_site = cross.clone();
        wrong_contract_site.line = other_site.line;
        wrong_contract_site.column = other_site.column + inserted_bytes + 1;
        refuse(
            "entry contract identity at another validated site",
            wrong_contract_site,
            &instrumentation,
        );

        let mut non_contract = cross.clone();
        non_contract.id = entry.function_node;
        let mut non_contract_ids = instrumentation.ids.clone();
        non_contract_ids.insert(entry.function_node);
        let non_contract_instrumentation = Instrumentation {
            insertions: instrumentation.insertions.clone(),
            ids: non_contract_ids,
        };
        refuse(
            "non-contract entry diagnostic",
            non_contract,
            &non_contract_instrumentation,
        );

        let wrong_origins = [
            ("wrong entry package", {
                let mut diagnostic = cross.clone();
                diagnostic.origin.package_id = "wrong-package-id".to_string();
                diagnostic
            }),
            ("wrong entry target name", {
                let mut diagnostic = cross.clone();
                diagnostic.origin.target_name = "wrong-target".to_string();
                diagnostic
            }),
            ("wrong entry target kind", {
                let mut diagnostic = cross.clone();
                diagnostic.origin.target_kind = CargoTargetKind::Library;
                diagnostic
            }),
            ("wrong entry target source", {
                let mut diagnostic = cross.clone();
                diagnostic.origin.target_source = root.join("wost/src/lib.rs");
                diagnostic
            }),
        ];
        for (label, diagnostic) in wrong_origins {
            refuse(label, diagnostic, &instrumentation);
        }

        let wost_origin = &inventory.files[declaration.file].origin;
        let mut local = cross;
        local.origin = CargoDiagnosticOrigin {
            package_id: wost_origin.package_id.clone(),
            target_name: wost_origin.name.clone(),
            target_kind: wost_origin.kind,
            target_source: root.join(&wost_origin.relative),
        };
        let mut local_uses = Uses::default();
        local_uses
            .observe_cargo(
                RootKind::Production,
                &inventory,
                &root,
                &[local],
                &instrumentation,
            )
            .expect("same-wost macro definition diagnostic is ordinary opaque evidence");
        assert!(local_uses.edges.is_empty());
        assert!(local_uses.roots.is_empty());
        assert!(
            local_uses
                .dead(&inventory)
                .iter()
                .any(|node| node.id == enter)
        );

        std::fs::remove_dir_all(root).expect("remove entry origin fixture");
    }

    #[test]
    fn core_offset_of_diagnostics_are_discarded_at_the_terminal_opaque_callsite() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-offset-of-expansion-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("libs/fixture/src"))
            .expect("offset_of fixture source directory");
        std::fs::create_dir_all(root.join("wost/src"))
            .expect("empty offset_of fixture wost source");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"libs/fixture\"]\nresolver = \"3\"\n",
        )
        .expect("offset_of fixture workspace manifest");
        std::fs::write(
            root.join("libs/fixture/Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("offset_of fixture package manifest");
        std::fs::write(
            root.join("libs/fixture/src/lib.rs"),
            r#"#![warn(deprecated)]
#[repr(C)]
pub struct Gate { pub field: u8 }

#[cfg(test)]
mod tests {
    use super::Gate;

    macro_rules! inner_offset {
        () => { core::mem::offset_of!(Gate, field) };
    }
    macro_rules! nested_offset {
        () => { inner_offset!() };
    }

    #[test]
    fn layout() {
        let _ = core::mem::offset_of!(Gate, field);
        let _ = nested_offset!();
    }
}
"#,
        )
        .expect("offset_of fixture source");

        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let metadata = isolated_test_cargo_command(&root, &compiler)
            .args(["metadata", "--format-version=1", "--offline"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("offset_of fixture Cargo metadata");
        assert!(
            metadata.status.success(),
            "{}{}",
            String::from_utf8_lossy(&metadata.stdout),
            String::from_utf8_lossy(&metadata.stderr)
        );
        let catalog = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
            .expect("offset_of fixture catalog");
        let selected = catalog
            .package("fixture")
            .expect("offset_of fixture package")
            .iter()
            .find(|target| target.kind == CargoTargetKind::Library)
            .expect("offset_of fixture library target")
            .clone();
        let inventory =
            Inventory::discover_from_catalog(&root, &catalog).expect("offset_of inventory");
        let gate = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "Gate")
            .expect("Gate API")
            .id;
        let instrumentation = (0..inventory.antichain_rounds)
            .find_map(|round| {
                let instrumentation = inventory
                    .instrument(&root, round)
                    .expect("instrument offset_of fixture");
                instrumentation
                    .ids
                    .contains(&gate)
                    .then_some(instrumentation)
            })
            .expect("Gate instrumentation round");
        let compile = || {
            isolated_test_cargo_command(&root, &compiler)
                .env("CARGO_TARGET_DIR", root.join("target"))
                .env("CARGO_ENCODED_RUSTFLAGS", "")
                .env("CARGO_BUILD_WARNINGS", "warn")
                .args([
                    "test",
                    "--no-run",
                    "--offline",
                    "--locked",
                    "--message-format=json",
                    "--package",
                    "fixture",
                    "--lib",
                    "--target",
                ])
                .arg(&compiler.host_target)
                .args(["--color=never"])
                .arg("--manifest-path")
                .arg(root.join("Cargo.toml"))
                .output()
                .expect("compile offset_of fixture")
        };
        let compiled = compile();
        assert!(
            compiled.status.success(),
            "{}{}",
            String::from_utf8_lossy(&compiled.stdout),
            String::from_utf8_lossy(&compiled.stderr)
        );
        let diagnostics = cargo_test_output(&compiled.stdout, &selected)
            .expect("structured offset_of diagnostics");
        let external = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.id == gate
                    && inventory.file_index(&root, &diagnostic.reported).is_none()
                    && diagnostic
                        .immediate_expansion
                        .as_ref()
                        .is_some_and(|expansion| expansion.terminal)
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !external.is_empty(),
            "offset_of emitted no Gate marker with an external primary span"
        );
        for diagnostic in &external {
            assert!(
                Path::new(&diagnostic.reported).ends_with(Path::new("library/core/src/mem/mod.rs")),
                "unexpected external primary span: {}",
                diagnostic.reported
            );
            let expansion = diagnostic
                .immediate_expansion
                .as_ref()
                .expect("offset_of diagnostic immediate expansion");
            assert_eq!(expansion.macro_decl_name, "core::mem::offset_of!");
            let file = inventory
                .file_index(&root, &expansion.location.reported)
                .expect("classified offset_of invocation");
            let at = corrected_marker_location(
                file,
                expansion.location.line,
                expansion.location.column,
                &instrumentation,
            )
            .expect("corrected offset_of invocation");
            assert!(
                inventory.is_opaque_macro_use(file, at),
                "offset_of expansion did not identify the opaque invocation"
            );
        }
        let mut uses = Uses::default();
        uses.observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &external,
            &instrumentation,
        )
        .expect("terminal opaque expansion diagnostics are discarded");
        assert!(uses.roots.is_empty());
        assert!(uses.edges.is_empty());
        assert!(uses.constructions.is_empty());
        assert!(uses.dead(&inventory).iter().any(|node| node.id == gate));

        let mut non_opaque = external[0].clone();
        let expansion = non_opaque
            .immediate_expansion
            .as_mut()
            .expect("offset_of diagnostic immediate expansion");
        expansion.location.line = 4;
        expansion.location.column = 1;
        let error = Uses::default()
            .observe_cargo(
                RootKind::TestHarness,
                &inventory,
                &root,
                &[non_opaque],
                &instrumentation,
            )
            .expect_err("an external primary cannot use a non-macro expansion callsite");
        assert!(
            format!("{error:#}").contains("not an opaque macro invocation"),
            "{error:#}"
        );

        let mut unclassified = external[0].clone();
        unclassified
            .immediate_expansion
            .as_mut()
            .expect("offset_of diagnostic immediate expansion")
            .location
            .reported = root.join("external-expansion.rs").display().to_string();
        let error = Uses::default()
            .observe_cargo(
                RootKind::TestHarness,
                &inventory,
                &root,
                &[unclassified],
                &instrumentation,
            )
            .expect_err("an external primary cannot use an unclassified expansion callsite");
        assert!(
            format!("{error:#}").contains("unclassified immediate expansion source"),
            "{error:#}"
        );

        let mut missing = external[0].clone();
        missing.immediate_expansion = None;
        let error = Uses::default()
            .observe_cargo(
                RootKind::TestHarness,
                &inventory,
                &root,
                &[missing],
                &instrumentation,
            )
            .expect_err("an external primary must identify its immediate expansion callsite");
        assert!(
            format!("{error:#}").contains("without an immediate expansion callsite"),
            "{error:#}"
        );

        let classified_nested = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.id == gate
                    && inventory.file_index(&root, &diagnostic.reported).is_some()
                    && diagnostic
                        .immediate_expansion
                        .as_ref()
                        .is_some_and(|expansion| !expansion.terminal)
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !classified_nested.is_empty(),
            "nested offset_of emitted no classified nonterminal Gate diagnostic"
        );
        let mut uses = Uses::default();
        uses.observe_cargo(
            RootKind::TestHarness,
            &inventory,
            &root,
            &classified_nested,
            &instrumentation,
        )
        .expect("a classified primary retains its lexical opaque-macro policy");
        assert!(uses.roots.is_empty());
        assert!(uses.edges.is_empty());
        assert!(uses.constructions.is_empty());
        assert!(uses.dead(&inventory).iter().any(|node| node.id == gate));

        let external_nested = diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.id == gate
                    && inventory.file_index(&root, &diagnostic.reported).is_none()
                    && diagnostic
                        .immediate_expansion
                        .as_ref()
                        .is_some_and(|expansion| !expansion.terminal)
            })
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            !external_nested.is_empty(),
            "nested offset_of emitted no external nonterminal Gate diagnostic"
        );
        let error = Uses::default()
            .observe_cargo(
                RootKind::TestHarness,
                &inventory,
                &root,
                &external_nested,
                &instrumentation,
            )
            .expect_err("an external primary cannot traverse a nested expansion chain");
        assert!(
            format!("{error:#}").contains("nested expansion chain"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove offset_of fixture");
    }

    #[test]
    fn a_macro_generated_custom_harness_main_is_refused_from_its_manifest() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-custom-harness-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("libs/fixture/tests"))
            .expect("custom harness fixture directory");
        std::fs::create_dir_all(root.join("wost/src")).expect("empty custom harness wost source");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"libs/fixture\"]\nresolver = \"3\"\n",
        )
        .expect("custom harness workspace manifest");
        std::fs::write(
            root.join("libs/fixture/Cargo.toml"),
            r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
autobins = false

[[test]]
name = "hostile"
path = "tests/hostile.rs"
harness = false
"#,
        )
        .expect("custom harness package manifest");
        std::fs::write(
            root.join("libs/fixture/tests/hostile.rs"),
            "pub fn hidden() {}\nmacro_rules! make_main { () => { fn main() { hidden(); } } }\nmake_main!();\n",
        )
        .expect("custom harness source");
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        let compiler = compiler_identity(repository).expect("adjacent compiler identity");
        let compiled = isolated_test_cargo_command(&root, &compiler)
            .args([
                "test",
                "--offline",
                "--no-run",
                "--package",
                "fixture",
                "--test",
                "hostile",
            ])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("compile custom harness fixture");
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let metadata = isolated_test_cargo_command(&root, &compiler)
            .args(["metadata", "--format-version=1", "--offline"])
            .arg("--manifest-path")
            .arg(root.join("Cargo.toml"))
            .output()
            .expect("read custom harness metadata");
        assert!(
            metadata.status.success(),
            "{}",
            String::from_utf8_lossy(&metadata.stderr)
        );
        let error = CargoCatalog::parse(&root, &root, &metadata.stdout, &["fixture"], true)
            .expect_err("a custom harness manifest must be refused");
        assert!(format!("{error:#}").contains("custom-harness"), "{error:#}");
        std::fs::remove_dir_all(root).expect("remove custom harness fixture");
    }

    #[test]
    fn every_host_target_mode_receives_its_own_compiler_graph() {
        let mut packages = crate::paths::HOST_PACKAGES
            .iter()
            .map(|package| ((*package).to_string(), Vec::new()))
            .collect::<BTreeMap<_, _>>();
        let package = crate::paths::HOST_PACKAGES[0];
        let target = |name: &str, kind, test| CargoTarget {
            package: package.to_string(),
            package_id: format!("{package}-id"),
            name: name.to_string(),
            relative: PathBuf::from(format!("libs/{package}/{name}.rs")),
            kind,
            test,
            executable: matches!(kind, CargoTargetKind::Bin | CargoTargetKind::Example),
        };
        packages.insert(
            package.to_string(),
            vec![
                target("core", CargoTargetKind::Library, true),
                target("tool", CargoTargetKind::Bin, true),
                target("demo", CargoTargetKind::Example, true),
                target("integration", CargoTargetKind::Test, false),
                target("benchmark", CargoTargetKind::Bench, false),
                target("build-script-build", CargoTargetKind::CustomBuild, false),
            ],
        );
        let plan = AuditPlan::from_catalog(
            CargoCatalog {
                packages,
                resolved: BTreeMap::new(),
            },
            compiler_identity(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .expect("repository root"),
            )
            .expect("fixture compiler identity"),
        )
        .expect("fixture plan");
        let identities = plan
            .compilations
            .iter()
            .filter_map(|compilation| match &compilation.mode {
                CompilationMode::Example(target) if target.package == package => {
                    Some(format!("normal:{}", target.name))
                }
                CompilationMode::TestHarness(target) if target.package == package => {
                    Some(format!("test:{}", target.name))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let distinct = identities.iter().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            identities.len(),
            distinct.len(),
            "two target modes shared one graph plan"
        );
        assert_eq!(
            distinct,
            [
                "normal:demo",
                "test:benchmark",
                "test:core",
                "test:demo",
                "test:integration",
                "test:tool",
            ]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
        );
    }

    #[cfg(unix)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating API directory symlink");
    }

    #[cfg(windows)]
    fn create_directory_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_dir(target, link).expect("creating API directory symlink");
    }

    #[cfg(unix)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::unix::fs::symlink(target, link).expect("creating API file symlink");
    }

    #[cfg(windows)]
    fn create_file_symlink(target: &Path, link: &Path) {
        std::os::windows::fs::symlink_file(target, link).expect("creating API file symlink");
    }

    #[test]
    fn redirected_api_audit_paths_are_refused_without_touching_outside_bytes() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-path-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let outside = root.join("outside");
        let target = root.join("target-link");
        std::fs::create_dir_all(&outside).unwrap();
        create_directory_symlink(&outside, &target);
        let paths = Paths::new(root.join("repository"), Some(&target));
        assert!(AuditWorkspace::create(&paths).is_err());
        assert!(!outside.join("api-audit").exists());
        std::fs::remove_dir_all(&root).unwrap();

        for redirected in ["workspace", "build"] {
            let root = std::env::temp_dir().join(format!(
                "wizard-api-{redirected}-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let target = root.join("target");
            let state = target.join("api-audit");
            let outside = root.join("outside");
            std::fs::create_dir_all(&state).unwrap();
            std::fs::create_dir_all(&outside).unwrap();
            std::fs::write(outside.join("keep"), "outside").unwrap();
            create_directory_symlink(&outside, &state.join(redirected));

            assert!(reset_audit_directory(&state, redirected).is_err());
            assert_eq!(
                std::fs::read_to_string(outside.join("keep")).unwrap(),
                "outside"
            );
            std::fs::remove_dir_all(&root).unwrap();
        }

        let root = std::env::temp_dir().join(format!(
            "wizard-api-lock-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let target = root.join("target");
        let state = target.join("api-audit");
        let outside = root.join("outside-lock");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::write(&outside, "outside").unwrap();
        create_file_symlink(&outside, &state.join("LOCK"));

        assert!(open_audit_lock(&state).is_err());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "outside");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn api_audit_copy_never_writes_through_an_external_link() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-copy-link-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let source = root.join("source");
        let workspace = root.join("workspace");
        let sentinel = root.join("outside-sentinel");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(source.join("input"), b"copied source").unwrap();
        std::fs::write(&sentinel, b"outside authority").unwrap();

        let copied = workspace.join("copied");
        std::fs::hard_link(&sentinel, &copied).unwrap();
        let hardlink = copy_file(&source.join("input"), &copied);
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"outside authority",
            "API audit copy truncated an external hardlink"
        );
        assert!(hardlink.is_err(), "an existing copied file was accepted");
        std::fs::remove_file(&copied).unwrap();

        create_file_symlink(&sentinel, &copied);
        let symlink = copy_file(&source.join("input"), &copied);
        assert_eq!(
            std::fs::read(&sentinel).unwrap(),
            b"outside authority",
            "API audit copy followed an external symlink"
        );
        assert!(symlink.is_err(), "a redirected copied file was accepted");
        std::fs::remove_file(&copied).unwrap();

        let redirected_source = source.join("redirected-source");
        let copied_redirected_source = workspace.join("copied-redirected-source");
        create_file_symlink(&sentinel, &redirected_source);
        assert!(
            copy_file(&redirected_source, &copied_redirected_source).is_err(),
            "a top-level redirected source file was accepted"
        );
        assert!(!copied_redirected_source.exists());
        std::fs::remove_file(&redirected_source).unwrap();

        let outside_directory = root.join("outside-directory");
        let redirected_directory = workspace.join("redirected");
        std::fs::create_dir(&outside_directory).unwrap();

        let redirected_source_directory = source.join("redirected-directory");
        let copied_redirected_directory = workspace.join("copied-redirected-directory");
        create_directory_symlink(&outside_directory, &redirected_source_directory);
        assert!(
            copy_tree(&redirected_source_directory, &copied_redirected_directory).is_err(),
            "a top-level redirected source directory was accepted"
        );
        assert!(!copied_redirected_directory.exists());
        std::fs::remove_dir(&redirected_source_directory).unwrap();

        create_directory_symlink(&outside_directory, &redirected_directory);
        assert!(copy_tree(&source, &redirected_directory).is_err());
        assert!(
            !outside_directory.join("input").exists(),
            "API audit recursion wrote through a redirected directory"
        );
        std::fs::remove_dir(&redirected_directory).unwrap();

        copy_file(&source.join("input"), &copied).unwrap();
        assert_eq!(std::fs::read(&copied).unwrap(), b"copied source");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn fixture(source: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("libs/fixture/src")).expect("fixture directory");
        std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
        std::fs::write(root.join("libs/fixture/src/main.rs"), source).expect("fixture source");
        root
    }

    fn compile_fixture(root: &Path, extra: &[&str]) -> String {
        let mut command = Command::new("rustc");
        command.current_dir(root).args([
            "--edition=2024",
            "libs/fixture/src/main.rs",
            "--emit=metadata",
            "--error-format=short",
            "-o",
            "fixture.rmeta",
        ]);
        command.args(extra);
        let result = command.output().expect("compile fixture");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stderr).expect("rustc diagnostics are UTF-8")
    }

    fn compile_wost_fixture(root: &Path) -> String {
        let result = Command::new("rustc")
            .current_dir(root)
            .args([
                "--edition=2024",
                "--crate-name",
                "wost",
                "wost/src/lib.rs",
                "--crate-type",
                "lib",
                "--emit=metadata",
                "--error-format=short",
                "-o",
                "wost.rmeta",
            ])
            .output()
            .expect("compile wost fixture");
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        String::from_utf8(result.stderr).expect("rustc diagnostics are UTF-8")
    }

    #[test]
    fn unicode_columns_instrument_the_exact_nested_item() {
        assert_eq!(
            byte_offset("a🧙z", LineColumn { line: 1, column: 2 }).expect("Unicode byte offset"),
            5
        );
        let root = fixture(
            "#![warn(deprecated)]\npub fn api() { let _ = \"🧙\"; fn nested() {} nested(); }\nfn main() { api(); }\n",
        );
        let inventory = Inventory::discover(&root).expect("Unicode fixture inventory");
        let nested = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "nested")
            .map(|node| node.id)
            .expect("nested item node");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument Unicode fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read Unicode fixture diagnostics");
        }
        assert!(uses.live(&inventory).contains(&nested));
        std::fs::remove_dir_all(root).expect("remove Unicode fixture");
    }

    #[test]
    fn an_adjacent_call_is_outside_a_half_open_nested_item_span() {
        let root = fixture(
            "#![warn(deprecated)]\npub fn api() {}\nfn outer(){fn inner(){}api();}\nfn main(){outer();}\n",
        );
        let inventory = Inventory::discover(&root).expect("adjacent-span fixture inventory");
        let api = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "api")
            .map(|node| node.id)
            .expect("fixture API node");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument adjacent-span fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read adjacent-span diagnostics");
        }
        assert!(!uses.dead(&inventory).iter().any(|node| node.id == api));
        std::fs::remove_dir_all(root).expect("remove adjacent-span fixture");
    }

    #[test]
    fn reexports_docs_and_same_names_are_resolved_by_the_compiler() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("libs/fixture/src")).expect("fixture directory");
        std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
        let source = r#"#![warn(deprecated)]
mod api {
    pub fn direct() {}
    pub fn reexported() {}
    pub fn reexport_only() {}
    pub fn import_only() {}
    pub fn documented() {}
    pub fn same_name() {}
    pub mod nested { pub fn same_name() {} }
}
pub use api::{reexport_only, reexported as public_name};
use api::import_only as unused_import;
/// Mentions [`api::documented`] but does not call it.
fn docs_only() {}
fn main() {
    api::direct();
    public_name();
    api::nested::same_name();
    docs_only();
}
"#;
        std::fs::write(root.join("libs/fixture/src/main.rs"), source).expect("fixture source");

        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let result = Command::new("rustc")
                .current_dir(&root)
                .args([
                    "--edition=2024",
                    "libs/fixture/src/main.rs",
                    "--emit=metadata",
                    "--error-format=short",
                    "-o",
                    "fixture.rmeta",
                ])
                .output()
                .expect("compile fixture");
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let output = String::from_utf8_lossy(&result.stderr);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &insertions,
            )
            .expect("read fixture diagnostics");
        }
        let dead: Vec<&str> = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.label.as_str())
            .collect();
        assert!(dead.iter().any(|label| label.contains("reexport_only")));
        assert!(dead.iter().any(|label| label.contains("import_only")));
        assert!(dead.iter().any(|label| label.contains("documented")));
        assert!(dead.iter().any(|label| label.contains("fn api::same_name")));
        assert!(!dead.iter().any(|label| label.contains("reexported")));
        assert!(!dead.iter().any(|label| label.contains("nested::same_name")));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn host_tests_are_legitimate_compiler_resolved_callers() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn production() {}
pub fn test_only() {}
fn main() { production(); }
#[cfg(test)]
mod tests {
    #[test]
    fn observer() { super::test_only(); }
}
"#,
        );
        std::fs::create_dir_all(root.join("libs/fixture/tests"))
            .expect("integration fixture directory");
        std::fs::write(
            root.join("libs/fixture/tests/public.rs"),
            "pub fn integration_only() {}\n",
        )
        .expect("integration fixture source");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        assert!(
            !inventory
                .nodes
                .iter()
                .any(|node| node.report && node.label.contains("integration_only"))
        );
        let test_only = inventory
            .nodes
            .iter()
            .find(|node| node.label.contains("fn test_only"))
            .expect("test-only item")
            .id;
        let mut uses = Uses::default();
        let mut compiled = GraphUses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let production = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &production,
                &insertions,
            )
            .expect("read production diagnostics");
            compiled
                .observe(
                    0,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &production,
                    &insertions,
                )
                .expect("read production diagnostics for complete graph");
            let verification = compile_fixture(&root, &["--test"]);
            if verification.contains(&format!("{MARKER}{test_only}")) {
                compiled
                    .observe(
                        1,
                        RootKind::Verification,
                        &inventory,
                        &root,
                        &verification,
                        &insertions,
                    )
                    .expect("read verification diagnostics for classification");
            }
        }
        let dead = uses.dead(&inventory);
        assert!(dead.iter().any(|node| node.id == test_only));
        assert!(
            !compiled
                .dead(&inventory)
                .iter()
                .any(|node| node.id == test_only)
        );
        assert!(!dead.iter().any(|node| node.label.contains("fn production")));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn authoritative_verdict_includes_every_compiled_graph() {
        let source = include_str!("api.rs");
        let check = source
            .split_once("pub fn check(paths: &Paths) -> Result<()> {")
            .and_then(|(_, source)| source.split_once("struct Compilation"))
            .map(|(source, _)| source)
            .expect("API-audit check body");
        assert!(check.contains("let mut uses = GraphUses::default();"));
        assert_eq!(check.matches("uses.observe_cargo(").count(), 1);
        assert!(!check.contains("if compilation.kind == RootKind::Production"));
        assert!(check.contains("uses.validate_dispatches(&inventory)?;"));
        assert!(check.contains("let dead = uses.dead(&inventory);"));
    }

    #[test]
    fn private_dormant_call_chains_do_not_become_production_roots() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn dormant_api() {}
pub fn reachable_api() {}
pub fn nested_dormant_api() {}
pub fn nested_reachable_api() {}
fn dormant_leaf() { dormant_api(); }
fn dormant_middle() { dormant_leaf(); }
fn reachable_leaf() { reachable_api(); }
fn reachable_middle() {
    fn nested_dormant() { nested_dormant_api(); }
    fn nested_reachable() { nested_reachable_api(); }
    nested_reachable();
    reachable_leaf();
}
fn main() { reachable_middle(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(dead.iter().all(|node| node.report));
        assert!(
            dead.iter()
                .any(|node| node.label.contains("fn dormant_api"))
        );
        assert!(
            dead.iter()
                .any(|node| node.label.contains("fn nested_dormant_api"))
        );
        assert!(
            !dead
                .iter()
                .any(|node| node.label.contains("fn reachable_api"))
        );
        assert!(
            !dead
                .iter()
                .any(|node| node.label.contains("fn nested_reachable_api"))
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn a_module_local_main_is_not_a_runtime_root() {
        let root = fixture("#![warn(deprecated)]\nmod dormant;\nfn main() {}\n");
        std::fs::write(
            root.join("libs/fixture/src/dormant.rs"),
            "pub fn target() {}\nfn main() { target(); }\n",
        )
        .expect("out-of-line module source");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }

        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "dormant::target"),
            "a module-local function named main became a runtime root"
        );
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn out_of_line_modules_inherit_scope_and_parent_availability() {
        let root = fixture(
            "#![warn(deprecated)]\nmod production;\n#[cfg(test)]\nmod verification;\nfn main() {}\n",
        );
        std::fs::write(
            root.join("libs/fixture/src/production.rs"),
            "pub fn called() {}\n",
        )
        .expect("production module source");
        std::fs::create_dir_all(root.join("libs/fixture/src/verification"))
            .expect("verification module directory");
        std::fs::write(
            root.join("libs/fixture/src/verification.rs"),
            "pub fn only() {}\npub mod child;\n",
        )
        .expect("verification module source");
        std::fs::write(
            root.join("libs/fixture/src/verification/child.rs"),
            "pub fn nested_only() {}\n",
        )
        .expect("nested verification module source");

        let inventory = Inventory::discover(&root).expect("fixture inventory");
        for (symbol, report) in [
            ("production::called", true),
            ("verification::only", false),
            ("verification::child::nested_only", false),
        ] {
            let node = inventory
                .nodes
                .iter()
                .find(|node| node.symbol == symbol)
                .unwrap_or_else(|| panic!("missing {symbol}"));
            assert_eq!(node.report, report, "wrong availability for {symbol}");
        }

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn literal_path_modules_use_the_declared_scope() {
        let root = fixture(
            "#![warn(deprecated)]\n#[path = \"alternate/implementation.rs\"]\nmod routed;\nfn main() {}\n",
        );
        std::fs::create_dir_all(root.join("libs/fixture/src/alternate"))
            .expect("path module directory");
        std::fs::write(
            root.join("libs/fixture/src/alternate/implementation.rs"),
            "pub fn target() {}\n",
        )
        .expect("path module source");

        let inventory = Inventory::discover(&root).expect("fixture inventory");
        assert!(
            inventory
                .nodes
                .iter()
                .any(|node| node.symbol == "routed::target" && node.report)
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn one_source_file_cannot_have_conflicting_module_contexts() {
        let root = fixture(
            "#![warn(deprecated)]\n#[path = \"shared.rs\"]\nmod first;\n#[path = \"shared.rs\"]\nmod second;\nfn main() {}\n",
        );
        std::fs::write(
            root.join("libs/fixture/src/shared.rs"),
            "pub fn target() {}\n",
        )
        .expect("shared module source");

        let error = Inventory::discover(&root).expect_err("conflicting contexts must be refused");
        assert!(format!("{error:#}").contains("conflicting module contexts"));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn ambiguous_default_module_routes_are_refused() {
        let root = fixture("#![warn(deprecated)]\nmod split;\nfn main() {}\n");
        std::fs::write(
            root.join("libs/fixture/src/split.rs"),
            "pub fn first() {}\n",
        )
        .expect("first module candidate");
        std::fs::create_dir_all(root.join("libs/fixture/src/split"))
            .expect("second module directory");
        std::fs::write(
            root.join("libs/fixture/src/split/mod.rs"),
            "pub fn second() {}\n",
        )
        .expect("second module candidate");

        let error = Inventory::discover(&root).expect_err("ambiguous route must be refused");
        assert!(format!("{error:#}").contains("ambiguous module route"));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn unsafe_or_nonliteral_path_routes_are_refused() {
        for declaration in [
            "#[path = concat!(\"route\", \".rs\")] mod routed;",
            "#[path = \"../../../../outside.rs\"] mod routed;",
        ] {
            let root = fixture(&format!(
                "#![warn(deprecated)]\npub fn exposed() {{}}\n{declaration}\nfn main() {{}}\n"
            ));
            let error = Inventory::discover(&root).expect_err("unsafe route must be refused");
            assert!(format!("{error:#}").contains("module path"));
            std::fs::remove_dir_all(&root).expect("remove fixture");
        }
    }

    #[test]
    fn only_a_top_level_target_main_is_a_language_root() {
        let root = fixture(
            "#![warn(deprecated)]\npub fn exposed() {}\nfn enclosing() { fn main() {} main(); }\nfn main() {}\n",
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let main_roots = inventory
            .nodes
            .iter()
            .filter(|node| node.symbol == "main" && node.roots.production)
            .count();
        assert_eq!(main_roots, 1, "a nested main became a runtime root");

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn explicit_runtime_and_trait_callers_reach_only_their_called_surface() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_runtime() {}
pub fn from_called_trait() {}
pub fn from_dormant_trait() {}
#[unsafe(no_mangle)]
extern "C" fn runtime_entry() { from_runtime(); }
trait Called { fn call(&self); }
struct Active;
impl Called for Active { fn call(&self) { from_called_trait(); } }
trait Dormant { fn call(&self); }
struct Installed;
impl Dormant for Installed { fn call(&self) { from_dormant_trait(); } }
fn main() { Active.call(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(
            !dead
                .iter()
                .any(|node| node.label.contains("fn from_runtime"))
        );
        assert!(
            !dead
                .iter()
                .any(|node| node.label.contains("fn from_called_trait"))
        );
        assert!(
            dead.iter()
                .any(|node| node.label.contains("fn from_dormant_trait"))
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn one_called_trait_implementation_does_not_activate_its_dormant_sibling() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_dormant() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Dormant;
impl Action for Dormant { fn act(&self) { from_dormant(); } }
fn main() { Active.act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(!dead.iter().any(|node| node.symbol == "from_active"));
        assert!(dead.iter().any(|node| node.symbol == "from_dormant"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn ambiguous_receiver_types_cannot_activate_multiple_trait_implementations() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_dormant() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Dormant;
impl Action for Dormant { fn act(&self) { from_dormant(); } }
fn pick(_: Active, _: Dormant) -> Active { Active }
fn main() { pick(Active, Dormant).act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("ambiguous-dispatch inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument ambiguous dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read ambiguous-dispatch diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("ambiguous receiver evidence must be refused")
            .to_string();
        assert!(reason.contains("resolves to 2 audited trait implementations"));
        std::fs::remove_dir_all(root).expect("remove ambiguous-dispatch fixture");
    }

    #[test]
    fn every_invalid_dispatch_in_one_graph_is_reported() {
        let root = fixture(
            r#"#![warn(deprecated)]
trait First { fn first(&self); }
pub fn from_first_a() {}
struct FirstA;
impl First for FirstA { fn first(&self) { from_first_a(); } }
pub fn from_first_b() {}
struct FirstB;
impl First for FirstB { fn first(&self) { from_first_b(); } }
fn choose_first(_: FirstA, _: FirstB) -> FirstA { FirstA }
trait Second { fn second(&self); }
pub fn from_second_a() {}
struct SecondA;
impl Second for SecondA { fn second(&self) { from_second_a(); } }
pub fn from_second_b() {}
struct SecondB;
impl Second for SecondB { fn second(&self) { from_second_b(); } }
fn choose_second(_: SecondA, _: SecondB) -> SecondA { SecondA }
fn main() {
    choose_first(FirstA, FirstB).first();
    choose_second(SecondA, SecondB).second();
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("multiple-dispatch inventory");
        let mut graphs = GraphUses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument multiple dispatches");
            let output = compile_fixture(&root, &[]);
            graphs
                .observe(
                    0,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &instrumentation,
                )
                .expect("read multiple-dispatch diagnostics");
        }

        let reason = graphs
            .validate_dispatches(&inventory)
            .expect_err("every ambiguous dispatch must be refused")
            .to_string();
        let first = reason
            .lines()
            .find(|line| line.contains("dispatch `first`"))
            .expect("first dispatch refusal");
        let second = reason
            .lines()
            .find(|line| line.contains("dispatch `second`"))
            .expect("second dispatch refusal");
        for refusal in [first, second] {
            assert!(refusal.contains("resolves to 2 audited trait implementations"));
            assert!(refusal.contains("compiler graphs 0"));
        }
        std::fs::remove_dir_all(root).expect("remove multiple-dispatch fixture");
    }

    #[test]
    fn a_later_compiler_graph_cannot_hide_its_invalid_dispatch() {
        let root = fixture(
            r#"#![warn(deprecated)]
trait Early { fn early(&self); }
pub fn from_early_a() {}
struct EarlyA;
impl Early for EarlyA { fn early(&self) { from_early_a(); } }
pub fn from_early_b() {}
struct EarlyB;
impl Early for EarlyB { fn early(&self) { from_early_b(); } }
fn choose_early(_: EarlyA, _: EarlyB) -> EarlyA { EarlyA }
trait Late { fn late(&self); }
pub fn from_late_a() {}
struct LateA;
impl Late for LateA { fn late(&self) { from_late_a(); } }
pub fn from_late_b() {}
struct LateB;
impl Late for LateB { fn late(&self) { from_late_b(); } }
fn choose_late(_: LateA, _: LateB) -> LateA { LateA }
#[cfg(audit_early)]
fn main() { choose_early(EarlyA, EarlyB).early(); }
#[cfg(not(audit_early))]
fn main() { choose_late(LateA, LateB).late(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("cross-graph dispatch inventory");
        let mut graphs = GraphUses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument cross-graph dispatches");
            for (graph, options) in [&["--cfg", "audit_early"][..], &[][..]]
                .into_iter()
                .enumerate()
            {
                let output = compile_fixture(&root, options);
                graphs
                    .observe(
                        graph,
                        RootKind::Production,
                        &inventory,
                        &root,
                        &output,
                        &instrumentation,
                    )
                    .expect("read cross-graph dispatch diagnostics");
            }
        }

        let reason = graphs
            .validate_dispatches(&inventory)
            .expect_err("invalid dispatches in every graph must be refused")
            .to_string();
        let early = reason
            .lines()
            .find(|line| line.contains("dispatch `early`"))
            .expect("early-graph dispatch refusal");
        let late = reason
            .lines()
            .find(|line| line.contains("dispatch `late`"))
            .expect("later-graph dispatch refusal");
        assert!(early.contains("compiler graphs 0"));
        assert!(late.contains("compiler graphs 1"));
        std::fs::remove_dir_all(root).expect("remove cross-graph dispatch fixture");
    }

    #[test]
    fn a_unique_local_trait_declaration_resolves_variable_receiver_dispatch() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn reached() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { reached(); } }
fn main() { let active = Active; active.act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("variable-dispatch inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument variable dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read variable-dispatch diagnostics");
        }
        assert!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "reached"),
            "unique declaration-identified variable dispatch was not activated"
        );
        std::fs::remove_dir_all(root).expect("remove variable-dispatch fixture");
    }

    #[test]
    fn ambiguous_variable_receiver_dispatch_fails_closed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_active() {}
pub fn from_other() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) { from_active(); } }
struct Other;
impl Action for Other { fn act(&self) { from_other(); } }
fn main() { let active = Active; active.act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("ambiguous-variable inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument ambiguous variable dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read ambiguous-variable diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("ambiguous variable dispatch must be refused")
            .to_string();
        assert!(reason.contains("resolves to 2 audited trait implementations"));
        std::fs::remove_dir_all(root).expect("remove ambiguous-variable fixture");
    }

    #[test]
    fn an_explicit_external_variable_dispatch_activates_its_implementation() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let subject = Subject; let _ = <Subject as Clone>::clone(&subject); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("external-variable inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument external variable dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read external-variable diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("explicit external dispatch");
        assert!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "cloned"),
            "explicit external dispatch did not activate its implementation"
        );
        std::fs::remove_dir_all(root).expect("remove external-variable fixture");
    }

    #[test]
    fn generic_trait_arguments_preserve_the_full_explicit_dispatch_qualifier() {
        let expression =
            syn::parse_str::<Expr>("<Subject as module::External<'_>>::same(&subject)")
                .expect("qualified call expression");
        let mut sites = DispatchSites::default();
        sites.visit_expr(&expression);
        let [site] = sites.sites.as_slice() else {
            panic!("qualified call did not produce exactly one dispatch site");
        };
        let DispatchKind::Call {
            member,
            trait_qualifier,
        } = &site.kind;
        assert_eq!(member, "same");
        assert_eq!(trait_qualifier.as_deref(), Some("module::External"));
    }

    #[test]
    fn qualified_dispatch_distinguishes_same_named_traits() {
        let site = DispatchSite {
            range: SourceRange {
                start: LineColumn { line: 1, column: 0 },
                end: LineColumn { line: 1, column: 1 },
            },
            callee_evidence: Vec::new(),
            self_evidence: Vec::new(),
            exact_self_syntax: false,
            exact_callee_self_syntax: false,
            explicit_qself: true,
            generic_qself_trait: None,
            kind: DispatchKind::Call {
                member: "same".to_string(),
                trait_qualifier: Some("selected::Action".to_string()),
            },
        };
        let activation = |trait_path: &str| TraitActivation {
            self_type: 0,
            exact_self_type: true,
            declaration: None,
            key: TraitMemberKey {
                trait_path: trait_path.to_string(),
                member: "same".to_string(),
            },
            implementation: 1,
        };
        let evidence = BTreeSet::new();

        assert!(activation("selected::Action").call_compatible(&site, &evidence));
        assert!(!activation("other::Action").call_compatible(&site, &evidence));
    }

    #[test]
    fn untyped_external_method_dispatch_does_not_guess_an_implementation() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let subject = Subject; let _ = subject.clone(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("external-variable inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument external variable dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read external-variable diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("untyped external dispatch stays conservative");
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "cloned"),
            "untyped method syntax guessed its external implementation"
        );
        std::fs::remove_dir_all(root).expect("remove external-variable fixture");
    }

    #[test]
    fn an_unqualified_external_dispatch_with_exact_self_fails_closed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn cloned() {}
struct Subject;
impl Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
fn main() { let _ = Subject.clone(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("exact external Self inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument exact external Self dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read exact external Self diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("an unqualified external dispatch must not use Self alone")
            .to_string();
        assert!(reason.contains("resolves to 0 audited trait implementations"));
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "cloned")
        );
        std::fs::remove_dir_all(root).expect("remove exact external Self fixture");
    }

    #[test]
    fn an_exact_inherent_method_excludes_a_same_named_external_trait_body() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_inherent() {}
pub fn from_trait() {}
struct Subject;
impl Subject {
    fn clone(&self) -> Self { from_inherent(); Subject }
}
impl std::clone::Clone for Subject {
    fn clone(&self) -> Self { from_trait(); Subject }
}
fn main() { let _ = Subject.clone(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("inherent-collision inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument inherent collision");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read inherent-collision diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("exact inherent method excludes trait resolution");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("from_inherent"));
        assert!(dead.contains("from_trait"));
        std::fs::remove_dir_all(root).expect("remove inherent-collision fixture");
    }

    #[test]
    fn an_exact_free_call_excludes_a_same_named_external_trait_body() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_trait() {}
struct Subject;
impl std::ops::Add for Subject {
    type Output = Self;
    fn add(self, _: Self) -> Self { from_trait(); Subject }
}
fn add(_: Subject) -> Subject { Subject }
fn main() { let _ = add(Subject); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("free-call collision inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument free-call collision");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read free-call collision diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("exact free call excludes trait resolution");
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "from_trait")
        );
        std::fs::remove_dir_all(root).expect("remove free-call collision fixture");
    }

    #[test]
    fn generic_qself_static_dispatch_credits_only_its_exact_trait_declaration() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn direct<T: Factory>() -> T { <T as Factory>::make(Argument) }
fn where_bound<T>() -> T where T: Factory { <T as Factory>::make(Argument) }
fn main() { let _ = direct::<Product>(); let _ = where_bound::<Product>(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("generic-qself inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument generic-qself dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read generic-qself diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("generic qself has an exact declaration-only route");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("Factory::make"));
        assert!(dead.contains("from_argument"));
        assert!(dead.contains("from_product"));
        std::fs::remove_dir_all(root).expect("remove generic-qself fixture");
    }

    #[test]
    fn parameterized_generic_qself_credits_only_its_exact_trait_declaration() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory<A> { fn make(_: A) -> Self; }
pub struct Argument;
impl Factory<Argument> for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory<Argument> for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn direct<T: Factory<Argument>>() -> T {
    <T as Factory<Argument>>::make(Argument)
}
fn where_bound<T>() -> T where T: Factory<Argument> {
    <T as Factory<Argument>>::make(Argument)
}
fn main() { let _ = direct::<Product>(); let _ = where_bound::<Product>(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("parameterized generic inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument parameterized generic dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read parameterized generic diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("parameterized generic qself has a declaration-only route");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("Factory::make"));
        assert!(dead.contains("from_argument"));
        assert!(dead.contains("from_product"));
        std::fs::remove_dir_all(root).expect("remove parameterized generic fixture");
    }

    #[test]
    fn generic_qself_rejects_indirect_and_nonparameter_shapes() {
        let syntax = syn::parse_file(
            r#"
trait Factory<'a> { fn make(); }
trait Bridge<'a>: Factory<'a> {}
trait Holder { type Item; }
struct Wrapper<T>(T);
struct Concrete;
type Alias = Concrete;
fn indirect<'a, T: Bridge<'a>>() { <T as Factory<'a>>::make(); }
fn higher_ranked<T: for<'a> Factory<'a>>() { <T as Factory<'static>>::make(); }
fn alias() { <Alias as Factory<'static>>::make(); }
fn associated<T: Holder>() { <T::Item as Factory<'static>>::make(); }
fn generic_self<T>() { <Wrapper<T> as Factory<'static>>::make(); }
"#,
        )
        .expect("parse rejected generic-qself shapes");
        let mut sites = DispatchSites::default();
        sites.visit_file(&syntax);
        assert_eq!(sites.sites.len(), 5);
        assert!(
            sites
                .sites
                .iter()
                .all(|site| site.generic_qself_trait.is_none())
        );
    }

    #[test]
    fn unsupported_generic_qself_bounds_fail_closed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub trait Factory<'a> { fn make() -> Self; }
pub trait Bridge: for<'a> Factory<'a> {}
pub struct Product;
impl<'a> Factory<'a> for Product { fn make() -> Self { Product } }
impl Bridge for Product {}
pub trait Holder { type Item: for<'a> Factory<'a>; }
impl Holder for Product { type Item = Product; }
fn indirect<T: Bridge>() -> T { <T as Factory<'static>>::make() }
fn higher_ranked<T: for<'a> Factory<'a>>() -> T {
    <T as Factory<'static>>::make()
}
fn associated<T: Holder>() -> T::Item {
    <T::Item as Factory<'static>>::make()
}
fn main() {
    let _ = indirect::<Product>();
    let _ = higher_ranked::<Product>();
    let _ = associated::<Product>();
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("unsupported generic inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument unsupported generic dispatches");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read unsupported generic diagnostics");
        }
        let failures = uses.dispatch_failures(&inventory);
        assert_eq!(failures.len(), 3);
        assert!(failures.iter().all(|failure| failure.implementations == 0));
        std::fs::remove_dir_all(root).expect("remove unsupported generic fixture");
    }

    #[test]
    fn an_exact_local_default_is_not_confused_with_a_sibling_override() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_default() {}
pub fn from_override() {}
trait Action { fn act(&self) { from_default(); } }
struct Defaulted;
impl Action for Defaulted {}
struct Overridden;
impl Action for Overridden { fn act(&self) { from_override(); } }
fn main() { <Defaulted as Action>::act(&Defaulted); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("local-default inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument local-default dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read local-default diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("exact local default dispatch");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("from_default"));
        assert!(dead.contains("from_override"));
        std::fs::remove_dir_all(root).expect("remove local-default fixture");
    }

    #[test]
    fn generic_qself_with_concrete_self_evidence_is_not_declaration_only() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
pub struct Product;
fn generic<T: Factory>() -> T { <T as Factory>::make(Argument) }
fn main() {}
"#,
        );
        let inventory = Inventory::discover(&root).expect("generic evidence inventory");
        let (file, site) = inventory
            .files
            .iter()
            .enumerate()
            .find_map(|(file, source)| {
                source
                    .dispatch_sites
                    .iter()
                    .position(|site| site.generic_qself_trait.as_deref() == Some("Factory"))
                    .map(|site| (file, site))
            })
            .expect("generic qself dispatch site");
        let declaration = inventory
            .nodes
            .iter()
            .find_map(|node| match &node.trait_route {
                Some(TraitRoute::Declaration(key))
                    if key.member == "make" && last_path_segment(&key.trait_path) == "Factory" =>
                {
                    Some(node.id)
                }
                _ => None,
            })
            .expect("Factory declaration marker");
        let concrete = inventory
            .nodes
            .iter()
            .find(|node| node.type_declaration && node.symbol == "Argument")
            .map(|node| node.id)
            .expect("concrete type marker");
        let evidence = DispatchEvidence {
            caller: 0,
            nodes: [declaration, concrete].into_iter().collect(),
            callee_nodes: [declaration, concrete].into_iter().collect(),
            self_nodes: [concrete].into_iter().collect(),
        };

        assert!(!Uses::default().declaration_only_generic_dispatch(
            &inventory,
            &inventory.files[file].dispatch_sites[site],
            &evidence,
        ));
        std::fs::remove_dir_all(root).expect("remove generic evidence fixture");
    }

    #[test]
    fn concrete_qself_static_dispatch_uses_self_not_argument_zero() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = <Product as Factory>::make(Argument); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("concrete-qself inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument concrete-qself dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read concrete-qself diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("concrete qself resolves by Self");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("from_product"));
        assert!(dead.contains("from_argument"));
        std::fs::remove_dir_all(root).expect("remove concrete-qself fixture");
    }

    #[test]
    fn type_qualified_static_dispatch_uses_callee_not_argument_zero() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub trait Factory { fn make(_: Argument) -> Self; }
pub struct Argument;
impl Factory for Argument {
    fn make(_: Argument) -> Self { from_argument(); Argument }
}
pub struct Product;
impl Factory for Product {
    fn make(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = Product::make(Argument); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("type-qualified inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument type-qualified dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read type-qualified diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("type-qualified dispatch resolves by its callee");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("from_product"));
        assert!(dead.contains("from_argument"));
        std::fs::remove_dir_all(root).expect("remove type-qualified fixture");
    }

    #[test]
    fn a_type_qualified_external_trait_call_credits_its_exact_callee_type() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub struct Origin;
pub struct Argument;
impl From<Origin> for Argument {
    fn from(_: Origin) -> Self { from_argument(); Argument }
}
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_product(); Product }
}
fn main() { let _ = Product::from(Argument); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("type-qualified external inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument type-qualified external dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read type-qualified external diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("a type-qualified external call resolves by its callee type");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("from_product"));
        assert!(dead.contains("from_argument"));
        std::fs::remove_dir_all(root).expect("remove type-qualified external fixture");
    }

    #[test]
    fn a_type_qualified_external_trait_call_with_two_implementations_fails_closed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_origin() {}
pub struct Origin;
pub struct Argument;
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_argument(); Product }
}
impl From<Origin> for Product {
    fn from(_: Origin) -> Self { from_origin(); Product }
}
fn main() { let _ = Product::from(Argument); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("two-implementation inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument two-implementation dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read two-implementation diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("two implementations of one external trait must be refused")
            .to_string();
        assert!(reason.contains("resolves to 2 audited trait implementations"));
        std::fs::remove_dir_all(root).expect("remove two-implementation fixture");
    }

    #[test]
    fn an_aliased_callee_does_not_borrow_argument_zero_for_an_external_trait() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_argument() {}
pub fn from_product() {}
pub struct Origin;
pub struct Argument;
impl From<Origin> for Argument {
    fn from(_: Origin) -> Self { from_argument(); Argument }
}
pub struct Product;
impl From<Argument> for Product {
    fn from(_: Argument) -> Self { from_product(); Product }
}
pub type Alias = Product;
fn main() { let _ = Alias::from(Argument); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("aliased-callee inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument aliased-callee dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read aliased-callee diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("an aliased callee must not resolve through its argument")
            .to_string();
        assert!(reason.contains("resolves to 0 audited trait implementations"));
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "from_argument"),
            "argument zero supplied the Self of an aliased callee"
        );
        std::fs::remove_dir_all(root).expect("remove aliased-callee fixture");
    }

    #[test]
    fn a_generic_variable_receiver_dispatch_fails_closed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn reached() {}
trait Action { fn act(&self); }
struct Wrapper<T>(T);
impl<T> Action for Wrapper<T> { fn act(&self) { reached(); } }
fn main() { let value = Wrapper(0_u8); value.act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("generic-variable inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument generic variable dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read generic-variable diagnostics");
        }
        let reason = uses
            .validate_dispatches(&inventory)
            .expect_err("erased generic dispatch must be refused")
            .to_string();
        assert!(reason.contains("resolves to 0 audited trait implementations"));
        std::fs::remove_dir_all(root).expect("remove generic-variable fixture");
    }

    #[test]
    fn erased_generic_self_types_cannot_activate_the_wrong_trait_specialization() {
        let root = fixture(
            r#"#![warn(deprecated)]
use std::marker::PhantomData;
pub fn dormant_specialization() {}
struct Selected;
struct Other;
struct Wrapper<T> { marker: PhantomData<T> }
trait Action { fn act(&self); }
impl Action for Wrapper<Other> {
    fn act(&self) { dormant_specialization(); }
}
impl Wrapper<Selected> { fn act(&self) {} }
fn main() { Wrapper::<Selected> { marker: PhantomData }.act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("generic-dispatch inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument generic dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read generic-dispatch diagnostics");
        }
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "dormant_specialization"),
            "an erased generic type activated the wrong trait specialization"
        );
        std::fs::remove_dir_all(root).expect("remove generic-dispatch fixture");
    }

    #[test]
    fn unique_external_method_dispatch_activates_only_its_exact_trait() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn cloned() {}
pub fn other_cloned() {}
pub fn dormant_clone() {}
struct Subject;
impl std::clone::Clone for Subject {
    fn clone(&self) -> Self { cloned(); Subject }
}
trait Other { fn clone(&self) -> Self; }
trait Another { fn clone(&self) -> Self; }
struct Both;
impl Other for Both {
    fn clone(&self) -> Self { other_cloned(); Both }
}
impl Another for Both {
    fn clone(&self) -> Self { dormant_clone(); Both }
}
fn main() {
    let _ = <Subject as std::clone::Clone>::clone(&Subject);
    let _ = Other::clone(&Both);
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("external-dispatch inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument external dispatch");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read external-dispatch diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("explicit external dispatches");
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("cloned"));
        assert!(!dead.contains("other_cloned"));
        assert!(dead.contains("dormant_clone"));
        std::fs::remove_dir_all(root).expect("remove external-dispatch fixture");
    }

    #[test]
    fn independent_type_and_trait_uses_do_not_activate_an_implementation() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn orphan() {}
trait Action { fn act(&self); }
struct Active;
impl Action for Active { fn act(&self) {} }
struct Dormant;
impl Action for Dormant { fn act(&self) { orphan(); } }
fn main() { Active.act(); let _ = Dormant; }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "orphan")
        );
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn a_reachable_type_does_not_activate_an_uncalled_trait_member() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_dormant_trait() {}
trait Action { fn act(&self); }
struct Installed;
impl Action for Installed { fn act(&self) { from_dormant_trait(); } }
fn main() { let _installed = Installed; }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(dead.iter().any(|node| node.symbol == "from_dormant_trait"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn constructing_a_type_does_not_activate_its_external_trait_body() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn orphan() {}
struct Dormant;
impl std::fmt::Display for Dormant {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        orphan();
        output.write_str("dormant")
    }
}
fn main() { let _ = Dormant; }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "orphan")
        );
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn reviewed_nominal_drop_types_activate_only_from_exact_constructor_spans() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"#![warn(deprecated)]
pub mod mm;
pub mod sync;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { sync::construct(); mm::mention(); }
"#,
        )
        .expect("drop fixture crate root");
        std::fs::write(
            root.join("wost/src/sync.rs"),
            r#"pub fn unit_dropped() {}
pub fn tuple_dropped() {}
pub fn record_dropped() {}
pub struct SpinLockGuard;
impl std::ops::Drop for SpinLockGuard { fn drop(&mut self) { unit_dropped(); } }
pub struct ForgottenSpinLockGuard(pub u8);
impl std::ops::Drop for ForgottenSpinLockGuard { fn drop(&mut self) { tuple_dropped(); } }
pub struct ThreadLockGuard { pub value: u8 }
impl std::ops::Drop for ThreadLockGuard { fn drop(&mut self) { record_dropped(); } }
use self::{
    ForgottenSpinLockGuard as TupleAlias,
    SpinLockGuard as UnitAlias,
    ThreadLockGuard as RecordAlias,
};
pub fn construct() {
    let _ = UnitAlias;
    let _ = TupleAlias(1);
    let _ = RecordAlias { value: 2 };
}
"#,
        )
        .expect("drop fixture constructors");
        std::fs::create_dir_all(root.join("wost/src/mm")).expect("drop fixture module");
        std::fs::write(
            root.join("wost/src/mm/mod.rs"),
            r#"pub fn dormant_drop() {}
pub fn type_only_drop() {}
pub fn static_drop() {}
pub fn custom_drop() {}
pub struct Frame;
impl std::ops::Drop for Frame { fn drop(&mut self) { dormant_drop(); } }
impl Frame { pub fn from_raw() -> Self { Self } }
pub fn construct_dormant() { let _ = Frame; }
pub struct Segment;
impl std::ops::Drop for Segment { fn drop(&mut self) { type_only_drop(); } }
impl Segment {
    pub fn from_raw() -> Self { Self }
    pub const SIZE: usize = 1;
    #[allow(non_upper_case_globals)]
    pub const Segment: usize = 2;
    pub fn size() -> usize { Self::SIZE }
}
pub struct USegment;
impl std::ops::Drop for USegment { fn drop(&mut self) { static_drop(); } }
impl From<()> for USegment { fn from(_: ()) -> Self { Self } }
pub static STATIC_VALUE: USegment = USegment;
mod hostile {
    pub trait Drop { fn drop(&mut self); }
    pub struct Live;
    impl Drop for Live { fn drop(&mut self) { super::custom_drop(); } }
    pub fn construct() { let _ = Live; }
}
pub fn mention() {
    let _ = std::mem::size_of::<Segment>();
    let _ = Segment::SIZE;
    let _ = Segment::Segment;
    let _ = Segment::size();
    let _ = &STATIC_VALUE;
    hostile::construct();
}
"#,
        )
        .expect("drop fixture negative cases");
        let inventory = Inventory::discover(&root).expect("drop fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument drop fixture");
            let output = compile_wost_fixture(&root);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read drop fixture diagnostics");
        }
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("sync::unit_dropped"));
        assert!(!dead.contains("sync::tuple_dropped"));
        assert!(!dead.contains("sync::record_dropped"));
        assert!(dead.contains("mm::dormant_drop"));
        assert!(dead.contains("mm::type_only_drop"));
        assert!(dead.contains("mm::static_drop"));
        assert!(dead.contains("mm::custom_drop"));
        std::fs::remove_dir_all(root).expect("remove drop fixture");
    }

    #[test]
    fn a_reviewed_owned_self_producer_activates_drop_without_type_use_shortcuts() {
        for (body, drop_live) in [
            ("let _window = Window::open();", true),
            (
                "let _ = core::mem::size_of::<Window>(); let _ = Window::status();",
                false,
            ),
        ] {
            let root = fixture("pub fn foundation() {}\nfn main() {}\n");
            std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
                .expect("Drop producer fixture module directory");
            std::fs::write(
                root.join("wost/src/lib.rs"),
                r#"#![warn(deprecated)]
pub mod arch { pub mod x86_64 { pub mod usercopy; } }
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { arch::x86_64::usercopy::exercise(); }
"#,
            )
            .expect("Drop producer fixture crate root");
            std::fs::write(
                root.join("wost/src/arch/x86_64/usercopy.rs"),
                format!(
                    r#"pub fn drop_body_api() {{}}
pub struct Window;
impl Window {{
    pub fn open() -> Self {{ Self }}
    pub fn status() -> usize {{ 1 }}
}}
impl core::ops::Drop for Window {{
    fn drop(&mut self) {{ drop_body_api(); }}
}}
pub fn exercise() {{ {body} }}
"#
                ),
            )
            .expect("Drop producer fixture source");
            let inventory = Inventory::discover(&root).expect("Drop producer inventory");
            let mut uses = Uses::default();
            for round in 0..inventory.rounds {
                let instrumentation = inventory
                    .instrument(&root, round)
                    .expect("instrument Drop producer fixture");
                let output = compile_wost_fixture(&root);
                uses.observe(
                    RootKind::Production,
                    &inventory,
                    &root,
                    &output,
                    &instrumentation,
                )
                .expect("read Drop producer diagnostics");
            }
            assert_eq!(
                !uses
                    .dead(&inventory)
                    .iter()
                    .any(|node| node.symbol == "arch::x86_64::usercopy::drop_body_api"),
                drop_live,
                "unexpected Drop verdict for `{body}`"
            );
            std::fs::remove_dir_all(root).expect("remove Drop producer fixture");
        }
    }

    #[test]
    fn plain_type_from_dispatch_reaches_the_reviewed_usegment_drop_producer() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/mm"))
            .expect("USegment producer fixture module directory");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"#![warn(deprecated)]
pub mod mm;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { mm::construct(); }
"#,
        )
        .expect("USegment producer fixture root");
        std::fs::write(
            root.join("wost/src/mm/mod.rs"),
            r#"pub fn drop_body_api() {}
pub struct Segment;
pub struct USegment;
impl From<Segment> for USegment { fn from(_: Segment) -> Self { Self } }
impl core::ops::Drop for USegment { fn drop(&mut self) { drop_body_api(); } }
pub fn construct() { let _ = <USegment as From<Segment>>::from(Segment); }
"#,
        )
        .expect("USegment producer fixture source");
        let inventory = Inventory::discover(&root).expect("USegment producer inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument USegment producer fixture");
            let output = compile_wost_fixture(&root);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read USegment producer diagnostics");
        }
        uses.validate_dispatches(&inventory)
            .expect("explicit USegment From dispatch resolves exactly");
        assert!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "mm::drop_body_api")
        );
        std::fs::remove_dir_all(root).expect("remove USegment producer fixture");
    }

    #[test]
    fn a_new_owned_self_drop_producer_is_refused_until_reviewed() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
            .expect("unreviewed producer fixture module directory");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            "pub mod arch { pub mod x86_64 { pub mod usercopy; } }\n",
        )
        .expect("unreviewed producer fixture root");
        std::fs::write(
            root.join("wost/src/arch/x86_64/usercopy.rs"),
            r#"pub struct Window;
impl Window {
    pub fn open() -> Self { Self }
    pub fn alternate() -> Self { Self }
}
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
        )
        .expect("unreviewed producer fixture source");
        let error = Inventory::discover(&root)
            .expect_err("a new owned-Self producer must require review")
            .to_string();
        assert!(
            error.contains("unreviewed implicit Drop producer"),
            "{error}"
        );
        std::fs::remove_dir_all(root).expect("remove unreviewed producer fixture");
    }

    #[test]
    fn a_cross_file_owned_self_drop_producer_is_refused_until_reviewed() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
            .expect("cross-file producer fixture module directory");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"pub mod arch { pub mod x86_64 { pub mod usercopy; } }
mod sibling;
"#,
        )
        .expect("cross-file producer fixture root");
        std::fs::write(
            root.join("wost/src/arch/x86_64/usercopy.rs"),
            r#"pub struct Window;
impl Window { pub fn open() -> Self { Self } }
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
        )
        .expect("cross-file producer target source");
        std::fs::write(
            root.join("wost/src/sibling.rs"),
            r#"impl crate::arch::x86_64::usercopy::Window {
    pub fn alternate() -> Self { Self }
}
"#,
        )
        .expect("cross-file producer source");
        compile_wost_fixture(&root);
        let error = Inventory::discover(&root)
            .expect_err("a cross-file owned-Self producer must require review")
            .to_string();
        assert!(
            error.contains("unreviewed implicit Drop producer"),
            "{error}"
        );
        std::fs::remove_dir_all(root).expect("remove cross-file producer fixture");
    }

    #[test]
    fn every_reviewed_drop_producer_resolves_in_the_current_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask has a repository parent");
        let mut targets = fixture_targets(root).expect("current source fixture targets");
        for target in &mut targets {
            if target.relative == Path::new("kernel/src/main.rs") {
                target.package = crate::paths::KERNEL_PACKAGE.to_string();
                target.package_id = crate::paths::KERNEL_PACKAGE.to_string();
            }
        }
        let inventory =
            Inventory::discover_with_targets(root, &targets).expect("current source API inventory");
        for (index, producer) in REVIEWED_IMPLICIT_DROP_PRODUCERS.iter().enumerate() {
            let matches = inventory
                .nodes
                .iter()
                .filter(|node| node.reviewed_drop_producer == Some(index))
                .collect::<Vec<_>>();
            assert_eq!(
                matches.len(),
                1,
                "reviewed producer {}::{} did not resolve exactly once",
                producer.owner,
                producer.member
            );
            assert!(matches[0].direct_self_construction);
        }
    }

    #[test]
    fn a_reviewed_drop_producer_must_belong_to_its_exact_result_type() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/arch/x86_64"))
            .expect("producer-owner fixture module directory");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            "pub mod arch { pub mod x86_64 { pub mod usercopy; } }\n",
        )
        .expect("producer-owner fixture root");
        std::fs::write(
            root.join("wost/src/arch/x86_64/usercopy.rs"),
            r#"pub struct Other;
pub struct Window;
impl Window { pub fn open() -> Self { Self } }
impl core::ops::Drop for Window { fn drop(&mut self) {} }
"#,
        )
        .expect("producer-owner fixture source");
        let inventory = Inventory::discover(&root).expect("producer-owner inventory");
        let mut nodes = inventory.nodes.clone();
        let producer = nodes
            .iter_mut()
            .find(|node| {
                node.reviewed_drop_producer
                    == REVIEWED_IMPLICIT_DROP_PRODUCERS
                        .iter()
                        .position(|producer| producer.produced == "arch::x86_64::usercopy::Window")
            })
            .expect("reviewed Window producer");
        producer.implementation_owner = Some("arch::x86_64::usercopy::Other".to_string());
        let error = trait_activations(&nodes, &inventory.files)
            .expect_err("a producer owned by another type must be refused")
            .to_string();
        assert!(error.contains("does not belong to its exact result type"));
        std::fs::remove_dir_all(root).expect("remove producer-owner fixture");
    }

    #[test]
    fn a_new_implicit_drop_contract_is_refused_until_reviewed() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn drop_body_api() {}
struct Unreviewed;
impl std::ops::Drop for Unreviewed { fn drop(&mut self) { drop_body_api(); } }
fn main() { let _ = Unreviewed; }
"#,
        );
        compile_fixture(&root, &[]);
        let error = Inventory::discover(&root)
            .expect_err("an unreviewed implicit Drop implementation must be refused");
        assert!(
            format!("{error:#}").contains("reviewed implicit-drop boundary"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove unreviewed Drop fixture");

        let root = fixture(
            r#"pub mod sync {
    pub fn drop_body_api() {}
    pub struct SpinLockGuard;
    impl std::ops::Drop for SpinLockGuard {
        fn drop(&mut self) { drop_body_api(); }
    }
}
fn main() { let _ = sync::SpinLockGuard; }
"#,
        );
        compile_fixture(&root, &[]);
        let targets = [CargoTarget {
            package: "impostor".to_string(),
            package_id: "impostor-id".to_string(),
            name: "wost".to_string(),
            relative: PathBuf::from("libs/fixture/src/main.rs"),
            kind: CargoTargetKind::Bin,
            test: true,
            executable: true,
        }];
        let error = Inventory::discover_with_targets(&root, &targets)
            .expect_err("a colliding Cargo target name must not inherit a Drop contract");
        assert!(
            format!("{error:#}").contains("reviewed implicit-drop boundary"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove colliding Drop fixture");
    }

    #[test]
    fn reviewed_drop_activation_is_an_explicit_construction_boundary_for_leaks() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"#![warn(deprecated)]
pub mod sync;
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { sync::leak(); }
"#,
        )
        .expect("Drop leak fixture crate root");
        std::fs::write(
            root.join("wost/src/sync.rs"),
            r#"pub fn drop_body_api() {}
pub struct SpinLockGuard;
impl std::ops::Drop for SpinLockGuard { fn drop(&mut self) { drop_body_api(); } }
pub fn leak() {
    std::mem::forget(SpinLockGuard);
    let bound = SpinLockGuard;
    std::mem::forget(bound);
    let _manual = std::mem::ManuallyDrop::new(SpinLockGuard);
}
"#,
        )
        .expect("Drop leak fixture source");
        let inventory = Inventory::discover(&root).expect("Drop leak fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument Drop leak fixture");
            let output = compile_wost_fixture(&root);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read Drop leak fixture diagnostics");
        }
        assert!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "sync::drop_body_api"),
            "the reviewed construction boundary stopped activating its Drop contract"
        );
        std::fs::remove_dir_all(root).expect("remove Drop leak fixture");
    }

    #[test]
    fn a_global_allocator_activates_only_its_exact_global_alloc_implementation() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn allocated() {}
pub fn dormant_allocator() {}
pub fn custom_allocator_trait() {}
struct Selected;
unsafe impl std::alloc::GlobalAlloc for Selected {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 {
        allocated();
        std::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) { allocated(); }
}
trait GlobalAlloc { fn allocate(&self); }
impl GlobalAlloc for Selected { fn allocate(&self) { custom_allocator_trait(); } }
struct Dormant;
unsafe impl std::alloc::GlobalAlloc for Dormant {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 {
        dormant_allocator();
        std::ptr::null_mut()
    }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
#[global_allocator]
static ALLOCATOR: Selected = Selected;
fn main() { let _ = Box::new(1_u8); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("allocator fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument allocator fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read allocator fixture diagnostics");
        }
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(!dead.contains("allocated"));
        assert!(dead.contains("dormant_allocator"));
        assert!(dead.contains("custom_allocator_trait"));
        std::fs::remove_dir_all(root).expect("remove allocator fixture");
    }

    #[test]
    fn a_global_allocator_type_alias_is_refused_without_exact_impl_identity() {
        let root = fixture(
            r#"struct Selected;
unsafe impl std::alloc::GlobalAlloc for Selected {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
type Alias = Selected;
#[global_allocator]
static ALLOCATOR: Alias = Selected;
pub fn api() {}
fn main() { let _ = Box::new(1_u8); }
"#,
        );
        compile_fixture(&root, &[]);
        let error = Inventory::discover(&root)
            .expect_err("an unresolved allocator type alias must be refused");
        assert!(format!("{error:#}").contains("unsupported type alias"));
        std::fs::remove_dir_all(root).expect("remove allocator-alias fixture");
    }

    #[test]
    fn generic_global_allocator_identities_are_refused_before_type_arguments_can_collapse() {
        let root = fixture(
            r#"use std::marker::PhantomData;
struct Selected;
struct Other;
struct Wrapper<T>(PhantomData<T>);
unsafe impl std::alloc::GlobalAlloc for Wrapper<Selected> {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
unsafe impl std::alloc::GlobalAlloc for Wrapper<Other> {
    unsafe fn alloc(&self, _: std::alloc::Layout) -> *mut u8 { dormant(); std::ptr::null_mut() }
    unsafe fn dealloc(&self, _: *mut u8, _: std::alloc::Layout) {}
}
#[global_allocator]
static ALLOCATOR: Wrapper<Selected> = Wrapper(PhantomData);
pub fn dormant() {}
fn main() { let _ = Box::new(1_u8); }
"#,
        );
        compile_fixture(&root, &[]);
        let error = Inventory::discover(&root)
            .expect_err("a generic global allocator identity must be refused");
        assert!(
            format!("{error:#}").contains("no exact nominal type"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove generic allocator fixture");
    }

    #[test]
    fn opaque_formatting_macros_do_not_activate_trait_bodies() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_display() {}
pub fn from_debug() {}
struct Both;
impl std::fmt::Display for Both {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        from_display();
        output.write_str("display")
    }
}
impl std::fmt::Debug for Both {
    fn fmt(&self, output: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        from_debug();
        output.write_str("debug")
    }
}
fn main() { let _ = format!("{}", Both); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(dead.iter().any(|node| node.symbol == "from_display"));
        assert!(dead.iter().any(|node| node.symbol == "from_debug"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn dependency_trait_imports_do_not_authorize_local_aliases() {
        let syntax = syn::parse_file(
            "use dependency::Observer;\nuse crate::local::Observer as LocalObserver;\n",
        )
        .expect("parse imports");
        let imports = TraitImports::collect(&syntax.items);

        assert!(imports.external_trait(&[], "Observer"));
        assert!(!imports.external_trait(&[], "LocalObserver"));
    }

    #[test]
    fn standard_trait_imports_have_external_authority() {
        let syntax = syn::parse_file("use core::fmt::Display;\n").expect("parse standard import");
        let imports = TraitImports::collect(&syntax.items);
        assert!(imports.external_trait(&[], "Display"));
    }

    #[test]
    fn dependency_roots_cannot_forge_implicit_standard_traits() {
        for (source, drop_path, allocator_path) in [
            (
                "extern crate evil as std;\n",
                Some("std::ops::Drop"),
                "std::alloc::GlobalAlloc",
            ),
            (
                "extern crate evil as core;\n",
                Some("core::ops::Drop"),
                "core::alloc::GlobalAlloc",
            ),
            (
                "extern crate evil as alloc;\n",
                None,
                "alloc::alloc::GlobalAlloc",
            ),
        ] {
            let syntax = syn::parse_file(source).expect("parse dependency root alias");
            let imports = TraitImports::collect(&syntax.items);
            if let Some(drop_path) = drop_path {
                assert_eq!(imports.implicit_standard_trait(&[], drop_path), None);
            }
            assert_eq!(imports.implicit_standard_trait(&[], allocator_path), None);
        }

        let syntax = syn::parse_file("extern crate core;\n").expect("parse canonical root");
        let imports = TraitImports::collect(&syntax.items);
        assert_eq!(
            imports.implicit_standard_trait(&[], "core::ops::Drop"),
            Some(ImplicitTrait::Drop)
        );
        assert_eq!(
            imports.implicit_standard_trait(&[], "core::alloc::GlobalAlloc"),
            Some(ImplicitTrait::GlobalAlloc)
        );
    }

    #[test]
    fn same_named_types_in_other_crates_do_not_ambiguate_trait_dispatch() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn target() {}
trait Observe { fn observe(&self); }
struct Subject;
impl Observe for Subject { fn observe(&self) { target(); } }
fn main() { Observe::observe(&Subject); }
"#,
        );
        std::fs::create_dir_all(root.join("libs/other/src")).expect("other source directory");
        std::fs::write(root.join("libs/other/src/lib.rs"), "pub struct Subject;\n")
            .expect("other same-name type");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("resolve same-name fixture diagnostics");
        }
        assert!(
            !uses
                .dead(&inventory)
                .iter()
                .any(|node| node.symbol == "target")
        );
        std::fs::remove_dir_all(root).expect("remove same-name fixture");
    }

    #[test]
    fn crate_modules_and_alias_chains_are_not_external_traits() {
        let syntax = syn::parse_file(
            r#"mod local { pub trait Observer {} }
use local as route;
use route::Observer as LocalObserver;
"#,
        )
        .expect("parse local imports");
        let imports = TraitImports::collect(&syntax.items);

        assert!(!imports.external_trait(&[], "LocalObserver"));
    }

    #[test]
    fn local_declarations_shadow_prelude_trait_names() {
        let syntax = syn::parse_file("trait Drop {}\n").expect("parse local trait");
        let imports = TraitImports::collect(&syntax.items);

        assert!(!imports.external_trait(&[], "Drop"));
    }

    #[test]
    fn every_edition_2024_prelude_trait_is_external() {
        let imports = TraitImports::default();

        for name in [
            "AsMut",
            "AsRef",
            "Clone",
            "Copy",
            "Default",
            "DoubleEndedIterator",
            "Drop",
            "Eq",
            "ExactSizeIterator",
            "Extend",
            "Fn",
            "FnMut",
            "FnOnce",
            "From",
            "FromIterator",
            "Future",
            "Hash",
            "Into",
            "IntoFuture",
            "IntoIterator",
            "Iterator",
            "Ord",
            "PartialEq",
            "PartialOrd",
            "Send",
            "Sized",
            "Sync",
            "ToOwned",
            "TryFrom",
            "TryInto",
            "Unpin",
        ] {
            assert!(imports.external_trait(&[], name), "missed {name}");
        }
    }

    #[test]
    fn prelude_traits_remain_resolved_beneath_local_globs() {
        let syntax = syn::parse_file("mod tests { use super::*; }\n").expect("parse glob import");
        let imports = TraitImports::collect(&syntax.items);

        assert!(imports.external_trait(&["tests".to_string()], "Drop"));
    }

    #[test]
    fn local_globs_can_shadow_prelude_trait_names() {
        let syntax = syn::parse_file("trait Drop {}\nmod tests { use super::*; }\n")
            .expect("parse shadowing glob import");
        let imports = TraitImports::collect(&syntax.items);

        assert!(!imports.external_trait(&["tests".to_string()], "Drop"));
    }

    #[test]
    fn workspace_qualified_trait_implementations_bind_to_the_declaring_crate() {
        let root = fixture(
            r#"#![warn(deprecated)]
extern crate self as fixture;
pub fn from_workspace_impl() {}
pub struct Installed;
pub mod route { pub use crate::Installed; }
pub trait Observer { fn observe(&self); }
#[path = "../../../xtask/src/observer.rs"]
mod observer;
fn main() { observer::run(); }
"#,
        );
        std::fs::create_dir_all(root.join("xtask/src")).expect("external caller directory");
        std::fs::write(
            root.join("xtask/src/observer.rs"),
            r#"use fixture::route::Installed;
impl fixture::Observer for Installed {
    fn observe(&self) { fixture::from_workspace_impl(); }
}
pub fn run() { fixture::Observer::observe(&Installed); }
"#,
        )
        .expect("external trait implementation");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("resolve workspace-qualified trait implementation");
        }
        let dead = uses.dead(&inventory);
        assert!(!dead.iter().any(|node| node.symbol == "from_workspace_impl"));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn an_unresolved_trait_implementation_refuses_a_clean_verdict() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_compound_self() {}
struct Part;
trait Action { fn act(&self); }
impl Action for (Part, Part) { fn act(&self) { from_compound_self(); } }
fn main() { (Part, Part).act(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut refusal = None;
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            if let Err(error) = Uses::default().observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            ) {
                refusal = Some(format!("{error:#}"));
                break;
            }
        }
        assert!(
            refusal
                .as_deref()
                .is_some_and(|error| error.contains("unresolved trait implementation"))
        );
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn a_nested_runtime_root_does_not_activate_its_dormant_lexical_parent() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_nested_runtime() {}
pub fn from_dormant_parent() {}
fn dormant_parent() {
    #[unsafe(no_mangle)]
    extern "C" fn nested_runtime() { from_nested_runtime(); }
    from_dormant_parent();
}
fn main() {}
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses.dead(&inventory);
        assert!(
            !dead
                .iter()
                .any(|node| node.label.contains("fn from_nested_runtime"))
        );
        assert!(
            dead.iter()
                .any(|node| node.label.contains("fn from_dormant_parent"))
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn verification_graphs_include_runtime_roots_from_both_cfg_surfaces() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn from_shared_main() {}
pub fn from_verification_runtime() {}
#[cfg(feature = "selftest")]
#[unsafe(no_mangle)]
extern "C" fn verification_runtime() { from_verification_runtime(); }
fn main() {
    #[cfg(feature = "selftest")]
    from_shared_main();
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &["--cfg", "feature=\"selftest\""]);
            uses.observe(
                RootKind::Verification,
                &inventory,
                &root,
                &output,
                &instrumentation,
            )
            .expect("read verification diagnostics");
        }
        let dead = uses.dead(&inventory);
        for live in ["fn from_shared_main", "fn from_verification_runtime"] {
            assert!(
                !dead.iter().any(|node| node.label.contains(live)),
                "verification runtime did not reach {live}"
            );
        }

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn mutually_exclusive_compiler_graphs_cannot_complete_each_others_call_chain() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn target() {}
fn bridge() {
    #[cfg(audit_edge)]
    target();
}
#[cfg(not(audit_edge))]
fn main() { bridge(); }
#[cfg(audit_edge)]
fn main() {}
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let target = inventory
            .nodes
            .iter()
            .find(|node| node.label.contains("fn target"))
            .expect("target item")
            .id;
        let mut merged = Uses::default();
        let mut graphs = GraphUses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            for (index, extra) in [&["--cfg", "audit_edge"][..], &[][..]]
                .into_iter()
                .enumerate()
            {
                let output = compile_fixture(&root, extra);
                graphs
                    .observe(
                        index,
                        RootKind::Production,
                        &inventory,
                        &root,
                        &output,
                        &insertions,
                    )
                    .expect("read graph diagnostics");
                merged
                    .observe(
                        RootKind::Production,
                        &inventory,
                        &root,
                        &output,
                        &insertions,
                    )
                    .expect("read merged diagnostics");
            }
        }

        assert!(
            !merged.dead(&inventory).iter().any(|node| node.id == target),
            "roots and edges from different compiler graphs formed a synthetic caller"
        );
        assert!(
            graphs.dead(&inventory).iter().any(|node| node.id == target),
            "the target was not dead in every individual compiler graph"
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn a_later_round_preserves_higher_graph_evidence() {
        let root = fixture("#![warn(deprecated)]\npub fn target() {}\nfn main() { target(); }\n");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let target = inventory
            .nodes
            .iter()
            .find(|node| node.label.contains("fn target"))
            .expect("target item")
            .id;
        let mut live = None;
        let mut quiet = None;
        for round in 0..inventory.antichain_rounds {
            let instrumentation = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            if output.contains(&format!("{MARKER}{target}")) {
                live = Some((instrumentation, output));
            } else if !instrumentation.ids.contains(&target) {
                quiet = Some((instrumentation, output));
            }
        }
        let (live_instrumentation, live_output) = live.expect("a target-use round");
        let (quiet_instrumentation, quiet_output) = quiet.expect("a non-target round");
        let mut graphs = GraphUses::default();

        graphs
            .observe(
                1,
                RootKind::Production,
                &inventory,
                &root,
                &live_output,
                &live_instrumentation,
            )
            .expect("observe graph one's resolved target use");
        assert!(!graphs.dead(&inventory).iter().any(|node| node.id == target));

        for graph in [0, 1] {
            graphs
                .observe(
                    graph,
                    RootKind::Production,
                    &inventory,
                    &root,
                    &quiet_output,
                    &quiet_instrumentation,
                )
                .expect("observe the next quiet compiler round");
        }

        assert!(
            graphs.graphs.get(1).is_some_and(|graph| {
                !graph.roots.is_empty() && graph.edges.values().any(|used| used.contains(&target))
            }),
            "observing graph zero erased prior graph-one evidence"
        );
        assert!(!graphs.dead(&inventory).iter().any(|node| node.id == target));
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn an_api_verdict_cannot_outlive_its_source_snapshot() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-snapshot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for directory in COPY_DIRECTORIES {
            std::fs::create_dir_all(root.join(directory)).expect("snapshot directory");
        }
        std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
        for file in COPY_FILES {
            std::fs::write(root.join(file), file).expect("snapshot input");
        }
        let source_relative = Path::new("libs/fixture/src/lib.rs");
        let source = root.join(source_relative);
        std::fs::create_dir_all(source.parent().unwrap()).expect("fixture source directory");
        std::fs::write(&source, "pub fn original() {}\n").expect("fixture source");
        let lock = File::open(std::env::current_exe().expect("test executable"))
            .expect("fixture lock handle");
        let workspace = AuditWorkspace {
            _lock: lock,
            source_root: root.clone(),
            root: root.clone(),
            build: root.join("build"),
            cargo_home: root.clone(),
            registry_source: root.clone(),
            cargo_inputs: BTreeMap::new(),
            execution_root: root.clone(),
            source: source_snapshot(&root).expect("initial source snapshot"),
        };
        workspace
            .require_source_unchanged(&root)
            .expect("unchanged source");

        std::fs::write(&source, "pub fn replacement() {}\n").expect("change source");
        let error = workspace.require_source_unchanged(&root).unwrap_err();
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains("libs/fixture/src/lib.rs")
        );
        std::fs::write(&source, "pub fn original() {}\n").expect("restore source");

        let added_relative = Path::new("wost/src/added.rs");
        let added = root.join(added_relative);
        std::fs::create_dir_all(added.parent().unwrap()).expect("added source directory");
        std::fs::write(&added, "pub fn added() {}\n").expect("add source");
        let error = workspace.require_source_unchanged(&root).unwrap_err();
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains("wost/src/added.rs")
        );
        std::fs::remove_file(added).expect("remove added source");

        std::fs::remove_file(root.join("Cargo.lock")).expect("remove snapshot input");
        let error = workspace.require_source_unchanged(&root).unwrap_err();
        assert!(format!("{error:#}").contains("Cargo.lock"));
    }

    #[test]
    fn compiler_graphs_cannot_change_the_instrumented_copy() {
        let root = std::env::temp_dir().join(format!(
            "wizard-api-copy-snapshot-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        for directory in COPY_DIRECTORIES {
            std::fs::create_dir_all(root.join(directory)).expect("snapshot directory");
        }
        std::fs::create_dir_all(root.join("wost/src")).expect("empty wost source directory");
        for file in COPY_FILES {
            std::fs::write(root.join(file), file).expect("snapshot input");
        }
        let source = root.join("libs/fixture/src/lib.rs");
        let manifest = root.join("libs/fixture/Cargo.toml");
        let config = root.join(".cargo/config.toml");
        std::fs::create_dir_all(source.parent().unwrap()).expect("fixture source directory");
        std::fs::write(&source, "pub fn original() {}\n").expect("fixture source");
        std::fs::write(&manifest, "[package]\nname = \"fixture\"\n").expect("fixture manifest");
        std::fs::write(&config, "[net]\noffline = true\n").expect("fixture cargo config");
        let lock = File::open(std::env::current_exe().expect("test executable"))
            .expect("fixture lock handle");
        let workspace = AuditWorkspace {
            _lock: lock,
            source_root: root.clone(),
            root: root.clone(),
            build: root.join("build"),
            cargo_home: root.clone(),
            registry_source: root.clone(),
            cargo_inputs: BTreeMap::new(),
            execution_root: root.clone(),
            source: source_snapshot(&root).expect("initial source snapshot"),
        };
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        inventory.instrument(&root, 0).expect("instrument fixture");
        let expected = source_snapshot(&root).expect("instrumented source snapshot");
        workspace
            .require_copy_unchanged(&expected)
            .expect("intentional instrumentation is authoritative");

        for path in [root.join("Cargo.toml"), config, manifest] {
            let original = std::fs::read(&path).expect("copied input");
            std::fs::write(&path, b"replacement").expect("replace copied input");
            let error = workspace.require_copy_unchanged(&expected).unwrap_err();
            let relative = path.strip_prefix(&root).unwrap().to_string_lossy();
            assert!(
                format!("{error:#}")
                    .replace('\\', "/")
                    .contains(&relative.replace('\\', "/")),
                "{error:#}"
            );
            std::fs::write(path, original).expect("restore copied input");
        }

        let added = root.join("libs/fixture/build-script-write");
        std::fs::write(&added, b"outside OUT_DIR").expect("simulated build-script write");
        let error = workspace.require_copy_unchanged(&expected).unwrap_err();
        assert!(
            format!("{error:#}")
                .replace('\\', "/")
                .contains("libs/fixture/build-script-write")
        );
        std::fs::remove_dir_all(root).expect("remove copied-input fixture");
    }

    #[test]
    fn copied_source_is_bound_after_instrumentation_and_after_each_graph() {
        let source = include_str!("api.rs");
        let pristine_compile = source
            .find("let _pristine = compile_graphs(&workspace, 0, &pristine_source, &plan, None)?;")
            .expect("pristine compilation step");
        let pristine_authority = source
            .find("let pristine_build_authority = (0..plan.compilations.len())")
            .expect("pristine build-script authority snapshot");
        let instrumentation = source
            .find("let insertions = inventory.instrument(&workspace.root, round)?;")
            .expect("instrumentation step");
        let snapshot = source
            .find("let instrumented_source = source_snapshot(&workspace.root)?;")
            .expect("instrumented snapshot");
        let compilation = source
            .find("Some(&pristine_build_authority)")
            .expect("compiler graph loop");
        assert!(
            pristine_compile < pristine_authority
                && pristine_authority < instrumentation
                && instrumentation < snapshot
                && snapshot < compilation
        );

        let compile_graphs = source
            .split_once("fn compile_graphs(")
            .and_then(|(_, source)| source.split_once("fn compile_graph("))
            .map(|(source, _)| source)
            .expect("compile-graphs body");
        let compiler = compile_graphs
            .find("let compiled = compile_graph(")
            .expect("compiler graph execution");
        let authority = compile_graphs
            .find("require_build_script_authority_unchanged(")
            .expect("build-script authority guard");
        let accepted = compile_graphs
            .find("compilations.push(compiled);")
            .expect("accepted compilation");
        assert!(compiler < authority && authority < accepted);

        let compile_graph = source
            .split_once("fn compile_graph(")
            .and_then(|(_, source)| source.split_once("fn cargo_arguments"))
            .map(|(source, _)| source)
            .expect("compile_graph body");
        let output = compile_graph
            .find(".output()")
            .expect("compiler completion");
        let copy_guard = compile_graph
            .find("workspace.require_copy_unchanged(expected)?;")
            .expect("copied-source guard");
        let verdict = compile_graph
            .find("if !result.status.success()")
            .expect("compiler verdict");
        assert!(output < copy_guard && copy_guard < verdict);
    }

    #[test]
    fn source_freshness_is_checked_after_compilation_and_before_the_verdict() {
        let source = include_str!("api.rs");
        let discovery = source
            .split_once("impl AuditPlan {")
            .and_then(|(_, source)| source.split_once("fn from_catalog("))
            .map(|(source, _)| source)
            .expect("API-audit plan discovery body");
        let metadata = discovery
            .find("let current = CargoCatalog::discover(workspace, expected, pass, &compiler)?;")
            .expect("Cargo metadata discovery");
        let metadata_complete = discovery
            .find(
                "let catalog = catalog.context(\"the API audit has no registered Cargo graph\")?;",
            )
            .expect("completed Cargo metadata catalog");
        let metadata_authority = discovery
            .find("compiler.require_input_authority_unchanged()?;")
            .expect("post-metadata compiler-authority guard");
        assert_eq!(
            discovery
                .matches("compiler.require_input_authority_unchanged()?;")
                .count(),
            1
        );
        assert!(metadata < metadata_complete && metadata_complete < metadata_authority);

        let check = source
            .split_once("pub fn check(paths: &Paths) -> Result<()> {")
            .and_then(|(_, source)| source.split_once("pub(crate) fn report_names_exact_dead_item"))
            .map(|(source, _)| source)
            .expect("API-audit check body");
        let pristine = check
            .find("let _pristine = compile_graphs(&workspace, 0, &pristine_source, &plan, None)?;")
            .expect("pristine compiler pass");
        let pristine_baseline = check
            .find("let pristine_build_authority = (0..plan.compilations.len())")
            .expect("pristine build-output baseline");
        let compile = check
            .find("for round in 0..inventory.rounds {")
            .expect("compiler loop");
        let observation = check
            .find("uses.observe_cargo(")
            .expect("compiler diagnostic observation");
        let round_authority = check
            .find("plan.compiler.require_target_libraries_unchanged()?;")
            .expect("per-round target-library guard");
        let registry_proofs = check
            .match_indices("workspace.require_cargo_inputs_unchanged()?;")
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        let [pristine_registry, round_registry] = registry_proofs.as_slice() else {
            panic!(
                "API check must rebind the registry after the pristine round and once per round"
            );
        };
        assert!(*pristine_registry < pristine_baseline);
        assert!(compile < *round_registry && *round_registry < round_authority);
        let freshness = check
            .find("workspace.require_source_unchanged(paths.root())?;")
            .expect("source freshness guard");
        let full_authority = check
            .match_indices("plan.compiler.require_input_authority_unchanged()?;")
            .map(|(position, _)| position)
            .collect::<Vec<_>>();
        let [post_pristine, final_authority] = full_authority.as_slice() else {
            panic!("API check must contain exactly two full compiler-authority guards");
        };
        let verdict = check
            .find("let dead = uses.dead(&inventory);")
            .expect("API verdict");
        assert_eq!(
            check
                .matches("plan.compiler.require_target_libraries_unchanged()?;")
                .count(),
            1
        );
        assert!(
            pristine < *post_pristine
                && *post_pristine < pristine_baseline
                && pristine_baseline < compile
                && compile < observation
                && observation < round_authority
                && round_authority < freshness
                && freshness < *final_authority
                && *final_authority < verdict
        );
    }

    #[test]
    fn opaque_macro_expansions_cannot_manufacture_a_call_chain() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub mod api {
    pub fn expanded() {}
    pub fn dormant() {}
}
#[macro_export]
macro_rules! invoke_expanded { () => { $crate::api::expanded() } }
macro_rules! never_expanded { () => { $crate::api::dormant() } }
fn main() { invoke_expanded!(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &insertions,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert!(dead.iter().any(|label| label.contains("fn api::expanded")));
        assert!(
            !dead
                .iter()
                .any(|label| label.contains("macro invoke_expanded"))
        );
        assert!(dead.iter().any(|label| label.contains("fn api::dormant")));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn macro_arguments_cannot_hide_dormant_lexical_callers() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn forwarded() {}
pub fn asserted() {}
pub fn evaluated() -> bool { true }
macro_rules! hide { ($path:path) => {{ fn dormant() { $path(); } }} }
fn main() {
    let _ = hide!(forwarded);
    assert!({ fn dormant() { asserted(); } true });
    assert!(evaluated());
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &insertions,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.symbol.as_str())
            .collect::<BTreeSet<_>>();
        assert!(dead.contains("forwarded"));
        assert!(dead.contains("asserted"));
        assert!(!dead.contains("evaluated"));
        std::fs::remove_dir_all(root).expect("remove macro-argument fixture");
    }

    #[test]
    fn locally_shadowed_standard_macros_are_not_transparent_callers() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn hidden() -> bool { true }
macro_rules! assert { ($condition:expr) => { fn dormant() { let _ = $condition; } } }
fn main() { assert!(hidden()); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("shadowed-macro inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument shadowed-macro fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &insertions,
            )
            .expect("read shadowed-macro diagnostics");
        }
        assert!(
            uses.dead(&inventory)
                .iter()
                .any(|node| node.symbol == "hidden"),
            "a locally shadowed standard macro manufactured a caller"
        );
        std::fs::remove_dir_all(root).expect("remove shadowed-standard-macro fixture");
    }

    #[test]
    fn noncanonical_language_root_bindings_disable_macro_transparency() {
        let context = ModuleContext {
            scope: Vec::new(),
            availability: Availability::Production,
            package: "fixture".to_string(),
            crate_name: "fixture".to_string(),
            target_root: PathBuf::from("libs/fixture/src/main.rs"),
            target_kind: CargoTargetKind::Bin,
            language_main: true,
        };
        for source in [
            "extern crate evil as std;\nuse std::assert;\n",
            "extern crate self as core;\nuse core::assert;\n",
            "use evil as alloc;\nuse alloc::assert;\n",
            "use evil::std;\nuse std::assert;\n",
            "mod core {}\nuse core::assert;\n",
        ] {
            let syntax = syn::parse_file(source).expect("parse hostile root binding");
            let mut policy = MacroShadowPolicy {
                shadows_standard: false,
                reserved_root_rebinding: None,
                context: &context,
                relative: &context.target_root,
            };
            policy.visit_file(&syntax);
            assert!(policy.shadows_standard, "trusted hostile binding: {source}");
        }

        let syntax = syn::parse_file("extern crate core;\nuse core::assert;\n")
            .expect("parse canonical standard binding");
        let mut policy = MacroShadowPolicy {
            shadows_standard: false,
            reserved_root_rebinding: None,
            context: &context,
            relative: &context.target_root,
        };
        policy.visit_file(&syntax);
        assert!(!policy.shadows_standard);
    }

    #[test]
    fn a_rebound_core_root_cannot_forge_the_approved_global_asm_macro() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::write(
            root.join("wost/src/lib.rs"),
            r#"#![no_std]
extern crate self as core;
pub fn hidden() {}
pub mod arch {
    macro_rules! global_asm {
        ($($tokens:tt)*) => { pub fn manufactured() { $crate::hidden(); } };
    }
    pub(crate) use global_asm;
}
::core::arch::global_asm!("ignored");
"#,
        )
        .expect("fake global-asm fixture");
        compile_wost_fixture(&root);
        let error = Inventory::discover(&root)
            .expect_err("a rebound core root must not authorize global_asm");
        assert!(format!("{error:#}").contains("rebinds reserved language root `core`"));
        std::fs::remove_dir_all(root).expect("remove fake-global-asm fixture");
    }

    #[test]
    fn an_item_macro_disables_unqualified_transparency_across_its_caller_target() {
        let root = fixture("pub fn foundation() {}\nfn main() {}\n");
        std::fs::create_dir_all(root.join("kernel/src")).expect("caller fixture directory");
        std::fs::write(
            root.join("kernel/src/main.rs"),
            r#"macro_rules! manufacture_shadow {
    () => { macro_rules! assert { ($value:expr) => { fn dormant() { let _ = $value; } } } }
}
manufacture_shadow!();
mod caller;
fn main() { caller::run(); }
"#,
        )
        .expect("caller target root");
        std::fs::write(
            root.join("kernel/src/caller.rs"),
            r#"pub fn hidden() -> bool { true }
pub fn run() {
    assert!(hidden());
    ::std::assert!(hidden());
}
"#,
        )
        .expect("caller target module");
        let targets = vec![
            CargoTarget {
                package: "fixture".to_string(),
                package_id: "fixture".to_string(),
                name: "fixture".to_string(),
                relative: PathBuf::from("libs/fixture/src/main.rs"),
                kind: CargoTargetKind::Bin,
                test: true,
                executable: true,
            },
            CargoTarget {
                package: "caller".to_string(),
                package_id: "caller".to_string(),
                name: "caller".to_string(),
                relative: PathBuf::from("kernel/src/main.rs"),
                kind: CargoTargetKind::Bin,
                test: true,
                executable: true,
            },
        ];
        let inventory = Inventory::discover_with_targets(&root, &targets)
            .expect("caller macro fixture inventory");
        let file = inventory
            .files
            .iter()
            .position(|file| file.relative == Path::new("kernel/src/caller.rs"))
            .expect("classified caller module");
        assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 3, column: 4 }));
        assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 4, column: 4 }));
        std::fs::remove_dir_all(root).expect("remove caller macro fixture");
    }

    #[test]
    fn a_statement_macro_can_manufacture_a_later_block_scoped_shadow() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub fn hidden() -> bool { true }
pub fn absolute() -> bool { true }
macro_rules! manufacture_shadow {
    () => { macro_rules! assert { ($value:expr) => { fn dormant() { let _ = $value; } } } }
}
fn main() {
    manufacture_shadow!();
    assert!(hidden());
    ::std::assert!(absolute());
}
"#,
        );
        let inventory = Inventory::discover(&root).expect("statement macro fixture inventory");
        let file = inventory
            .files
            .iter()
            .position(|file| file.relative == Path::new("libs/fixture/src/main.rs"))
            .expect("classified statement macro source");
        assert!(inventory.is_opaque_macro_use(file, LineColumn { line: 9, column: 4 }));
        assert!(inventory.is_opaque_macro_use(
            file,
            LineColumn {
                line: 10,
                column: 4
            }
        ));
        std::fs::remove_dir_all(root).expect("remove statement macro fixture");
    }

    #[test]
    fn absolute_standard_prefixes_require_canonical_reserved_roots() {
        for alias in ["std", "core", "alloc"] {
            let source = r#"#![no_std]
#![warn(deprecated)]
extern crate self as ALIAS;
#[macro_export]
macro_rules! assert { ($value:expr) => {{ fn dormant() { let _ = $value; } }} }
pub fn hidden() -> bool { true }
#[unsafe(no_mangle)]
pub extern "C" fn runtime() { ::ALIAS::assert!(hidden()); }
"#
            .replace("ALIAS", alias);
            let root = fixture(&source);
            compile_fixture(&root, &["--crate-type", "lib"]);
            let error = Inventory::discover(&root)
                .expect_err("a reserved language-root rebinding must be refused")
                .to_string();
            assert!(
                error.contains("rebinds reserved language root"),
                "unexpected `::{alias}` rebinding verdict: {error}"
            );
            std::fs::remove_dir_all(root).expect("remove absolute alias fixture");
        }
    }

    #[test]
    fn item_macros_cannot_hide_generated_public_surface() {
        let root = fixture(
            r#"macro_rules! generate_api { () => { pub fn hidden() {} } }
generate_api!();
fn main() {}
"#,
        );
        let error = Inventory::discover(&root).unwrap_err();
        let report = format!("{error:#}");
        assert!(report.contains("public expansion the API audit cannot inventory"));
        assert!(report.contains("generate_api at line 2"));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn shadowed_runtime_macro_names_cannot_hide_public_surface() {
        let root = fixture(
            "macro_rules! global_asm { () => { pub fn hidden_api() {} } }\nglobal_asm!();\nfn main() {}\n",
        );
        let error = Inventory::discover(&root).expect_err("shadowed runtime macro must be refused");
        let report = format!("{error:#}");
        assert!(report.contains("public expansion the API audit cannot inventory"));
        assert!(report.contains("global_asm at line 2"));
        std::fs::remove_dir_all(root).expect("remove shadowed macro fixture");
    }

    #[test]
    fn wost_entry_contract_binds_the_handler_to_the_linker_entry() {
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repository root");
        validate_wost_entry_contract(repository).expect("current entry contract");

        let root = fixture("fn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/boot")).expect("entry fixture directory");
        let source = include_str!("../../wost/src/boot/mod.rs");
        let changed = source.replacen(
            "#[unsafe(no_mangle)]",
            "#[unsafe(export_name = \"different_entry\")]",
            1,
        );
        assert_ne!(source, changed, "entry contract mutation did not apply");
        std::fs::write(root.join("wost/src/boot/mod.rs"), changed).expect("mutated entry fixture");
        let error = validate_wost_entry_contract(&root)
            .expect_err("a renamed linker entry must be refused");
        assert!(format!("{error:#}").contains("linker-visible"), "{error:#}");

        let expanded = source.replacen(
            "        extern \"C\" fn kmain() -> ! {",
            "        pub fn hidden_api() {}\n\n        extern \"C\" fn kmain() -> ! {",
            1,
        );
        assert_ne!(source, expanded, "entry expansion mutation did not apply");
        std::fs::write(root.join("wost/src/boot/mod.rs"), expanded)
            .expect("expanded entry fixture");
        let error = validate_wost_entry_contract(&root)
            .expect_err("an extra generated item must be refused");
        assert!(format!("{error:#}").contains("unapproved"), "{error:#}");

        let changed = source.replacen(
            "$crate::boot::__macro::base_revision();",
            "$crate::boot::__macro::BaseRevision::new();",
            1,
        );
        assert_ne!(source, changed, "base-revision mutation did not apply");
        std::fs::write(root.join("wost/src/boot/mod.rs"), changed)
            .expect("mutated base-revision fixture");
        let error = validate_wost_entry_contract(&root)
            .expect_err("an indirect base-revision constructor must be refused");
        assert!(format!("{error:#}").contains("base revision"), "{error:#}");

        let changed = source.replacen(
            "framebuffer: &__WOST_FRAMEBUFFER,",
            "framebuffer: &__WOST_MEMORY_MAP,",
            1,
        );
        assert_ne!(source, changed, "request-field mutation did not apply");
        std::fs::write(root.join("wost/src/boot/mod.rs"), changed)
            .expect("mutated request-block fixture");
        let error = validate_wost_entry_contract(&root)
            .expect_err("a miswired request field must be refused");
        assert!(
            format!("{error:#}").contains("exact request block"),
            "{error:#}"
        );

        let changed = source.replacen(
            "        extern \"C\" fn kmain() -> ! {\n            $crate::boot::enter(",
            "        extern \"C\" fn kmain() -> ! {\n            let _unapproved_runtime_work = 1;\n            $crate::boot::enter(",
            1,
        );
        assert_ne!(source, changed, "entry-body mutation did not apply");
        std::fs::write(root.join("wost/src/boot/mod.rs"), changed)
            .expect("mutated entry-body fixture");
        let error = validate_wost_entry_contract(&root)
            .expect_err("additional opaque runtime work must be refused");
        assert!(
            format!("{error:#}").contains("outside its exact audited contract"),
            "{error:#}"
        );
        std::fs::remove_dir_all(root).expect("remove entry-contract fixture");
    }

    #[test]
    fn exact_kernel_entry_reaches_its_handler_without_trusting_other_macro_tokens() {
        let root = fixture("fn main() {}\n");
        std::fs::create_dir_all(root.join("wost/src/boot")).expect("boot fixture directory");
        std::fs::create_dir_all(root.join("kernel/src")).expect("kernel fixture directory");
        std::fs::write(root.join("wost/src/lib.rs"), "pub mod boot;\n").expect("wost fixture root");
        std::fs::write(
            root.join("wost/src/boot/mod.rs"),
            format!(
                "{}\npub fn foundation() {{}}\n",
                validated_entry_fixture_source()
            ),
        )
        .expect("wost boot fixture");
        let kernel = r#"::wost::entry!(kernel_main);
fn kernel_main(_: wost::boot::BootInfo) -> ! {
    wost::boot::foundation();
    loop {}
}
"#;
        std::fs::write(root.join("kernel/src/main.rs"), kernel).expect("kernel entry fixture");
        let target = |package: &str, name: &str, relative: &str, kind, executable| CargoTarget {
            package: package.to_string(),
            package_id: format!("{package}-id"),
            name: name.to_string(),
            relative: PathBuf::from(relative),
            kind,
            test: true,
            executable,
        };
        let targets = vec![
            target(
                "fixture",
                "fixture",
                "libs/fixture/src/main.rs",
                CargoTargetKind::Bin,
                true,
            ),
            target(
                "wost",
                "wost",
                "wost/src/lib.rs",
                CargoTargetKind::Library,
                false,
            ),
            target(
                crate::paths::KERNEL_PACKAGE,
                crate::paths::KERNEL_PACKAGE,
                "kernel/src/main.rs",
                CargoTargetKind::Bin,
                true,
            ),
        ];
        let mut packages = BTreeMap::<String, Vec<CargoTarget>>::new();
        for target in &targets {
            packages
                .entry(target.package.clone())
                .or_default()
                .push(target.clone());
        }
        let catalog = CargoCatalog {
            packages,
            resolved: BTreeMap::from([(
                format!("{}-id", crate::paths::KERNEL_PACKAGE),
                vec![ResolvedDependency {
                    name: "wost".to_string(),
                    package_id: "wost-id".to_string(),
                    normal: true,
                    build: false,
                    dev: false,
                }],
            )]),
        };
        let raw = Inventory::discover_with_targets(&root, &targets)
            .expect("entry inventory without catalog authority");
        assert!(raw.entry.is_none());
        let inventory =
            Inventory::discover_from_catalog(&root, &catalog).expect("entry-linked inventory");
        let entry = inventory.entry.expect("trusted entry link");
        let foundation = inventory
            .nodes
            .iter()
            .find(|node| node.symbol == "boot::foundation")
            .expect("foundation API");
        let mut ids = BTreeSet::new();
        ids.extend([entry.function_node, entry.macro_declaration, foundation.id]);
        let diagnostic = |at: LineColumn, id| {
            format!(
                "kernel/src/main.rs:{}:{}: warning: resolved use {MARKER}{id}\n",
                at.line,
                at.column + 1
            )
        };
        let mut output = diagnostic(entry.argument.start, entry.function_node);
        output.push_str(&diagnostic(entry.macro_path.start, entry.macro_declaration));
        output.push_str(&diagnostic(
            LineColumn { line: 3, column: 4 },
            foundation.id,
        ));
        let mut uses = Uses::default();
        uses.observe(
            RootKind::Production,
            &inventory,
            &root,
            &output,
            &Instrumentation {
                insertions: BTreeMap::new(),
                ids,
            },
        )
        .expect("trusted entry diagnostics");
        let live = uses.live(&inventory);
        assert!(live.contains(&entry.invocation_node));
        assert!(live.contains(&entry.function_node));
        assert!(live.contains(&entry.macro_declaration));
        assert!(live.contains(&foundation.id));

        std::fs::write(
            root.join("kernel/src/main.rs"),
            kernel.replace("entry!(kernel_main)", "entry!(other)"),
        )
        .expect("malformed kernel entry fixture");
        let error = Inventory::discover_from_catalog(&root, &catalog)
            .expect_err("a different macro argument must not become an entry root");
        assert!(
            format!("{error:#}").contains("public expansion")
                || format!("{error:#}").contains("trusted"),
            "{error:#}"
        );

        let local_entry = r#"#![no_std]
pub mod boot { pub struct BootInfo; }
#[macro_export]
macro_rules! entry {
    ($path:path) => { const _: fn($crate::boot::BootInfo) -> ! = $path; };
}
extern crate self as wost;
::wost::entry!(kernel_main);
fn kernel_main(_: boot::BootInfo) -> ! { loop {} }
"#;
        let generated_binding = r#"#![no_std]
pub mod boot { pub struct BootInfo; }
#[macro_export]
macro_rules! entry {
    ($path:path) => { const _: fn($crate::boot::BootInfo) -> ! = $path; };
}
macro_rules! bind_wost { () => { extern crate self as wost; } }
bind_wost!();
::wost::entry!(kernel_main);
fn kernel_main(_: boot::BootInfo) -> ! { loop {} }
"#;
        for (label, source, refused) in [
            (
                "local wost extern-root binding",
                local_entry,
                "rebinds the kernel entry dependency root",
            ),
            (
                "opaque root item generator",
                generated_binding,
                "opaque root item macro",
            ),
        ] {
            std::fs::write(root.join("kernel/src/main.rs"), source)
                .expect("hostile kernel entry fixture");
            let result = Command::new("rustc")
                .current_dir(&root)
                .args([
                    "--edition=2024",
                    "--crate-name=kernel_entry_fixture",
                    "--crate-type=lib",
                    "kernel/src/main.rs",
                    "--emit=metadata",
                    "--error-format=short",
                    "-o",
                    "kernel-entry-fixture.rmeta",
                ])
                .output()
                .expect("compile hostile kernel entry fixture");
            assert!(
                result.status.success(),
                "{label} did not compile:\n{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let error = Inventory::discover_from_catalog(&root, &catalog).expect_err(label);
            assert!(format!("{error:#}").contains(refused), "{label}: {error:#}");
        }
        std::fs::remove_dir_all(root).expect("remove entry-link fixture");
    }

    #[test]
    fn decoded_literals_cannot_forge_a_compiler_diagnostic_marker() {
        for (label, source) in [
            (
                "direct escaped note",
                r#"#![warn(deprecated)]
#[deprecated(note = "wizard-api-audit:\x31")]
fn forged() {}
pub fn foundation() {}
fn main() { forged(); }
"#,
            ),
            (
                "escaped note in a macro body",
                r#"#![warn(deprecated)]
macro_rules! install_forge {
    () => {
        #[deprecated(note = "wizard-api-audit:\x31")]
        fn forged() {}
        fn call() { forged(); }
    }
}
install_forge!();
pub fn foundation() {}
fn main() { call(); }
"#,
            ),
        ] {
            let root = fixture(source);
            let output = compile_fixture(&root, &[]);
            let hostile_marker = format!("{MARKER}{}", 1);
            assert!(
                output.contains(&hostile_marker),
                "{label} did not produce the hostile compiler diagnostic: {output}"
            );
            let error = Inventory::discover(&root).expect_err(label);
            assert!(
                format!("{error:#}").contains("reserved diagnostic marker"),
                "{label}: {error:#}"
            );
            std::fs::remove_dir_all(root).expect("remove escaped-marker fixture");
        }
    }

    #[test]
    fn copied_source_cannot_forge_a_compiler_diagnostic_marker() {
        for external in [false, true] {
            let source = if external {
                "pub fn target() {}\nfn main() {}\n".to_string()
            } else {
                format!(
                    "#[deprecated(note = \"{MARKER}0\")]\npub fn forged() {{}}\nfn main() {{ forged(); }}\n"
                )
            };
            let root = fixture(&source);
            if external {
                std::fs::create_dir_all(root.join("kernel/src"))
                    .expect("external caller directory");
                std::fs::write(
                    root.join("kernel/src/caller.rs"),
                    format!(
                        "#[deprecated(note = \"{MARKER}0\")]\nfn forged() {{}}\nfn call() {{ forged(); }}\n"
                    ),
                )
                .expect("external caller source");
            }
            let error = Inventory::discover(&root).unwrap_err();
            assert!(
                format!("{error:#}").contains("reserved diagnostic marker"),
                "{error:#}"
            );

            std::fs::remove_dir_all(&root).expect("remove fixture");
        }
    }

    #[test]
    fn the_audit_implementation_cannot_author_a_diagnostic_id() {
        let root = fixture("pub fn target() {}\nfn main() {}\n");
        let implementation = root.join("xtask/src/api.rs");
        std::fs::create_dir_all(implementation.parent().unwrap())
            .expect("audit implementation directory");
        std::fs::write(
            &implementation,
            format!(
                "#[deprecated(note = \"{MARKER}0\")]\nfn forged() {{}}\nfn call() {{ forged(); }}\n"
            ),
        )
        .expect("authored diagnostic fixture");

        let error = Inventory::discover(&root).unwrap_err();
        assert!(
            format!("{error:#}").contains("reserved diagnostic marker"),
            "{error:#}"
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn a_diagnostic_id_must_be_instrumented_in_the_current_round() {
        let root = fixture("pub fn target() {}\nfn main() {}\n");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let target = inventory
            .nodes
            .iter()
            .find(|node| node.label.contains("fn target"))
            .expect("target item");
        let round = inventory.antichain_rounds;
        let instrumentation = inventory.instrument(&root, round).expect("isolation round");
        assert!(!instrumentation.ids.contains(&target.id));
        let forged = format!(
            "outside.rs:1:1: warning: forged deprecation diagnostic {MARKER}{}\n",
            target.id
        );

        let error = Uses::default()
            .observe(
                RootKind::Production,
                &inventory,
                &root,
                &forged,
                &instrumentation,
            )
            .unwrap_err();
        assert!(
            format!("{error:#}").contains("not instrumented in this compiler round"),
            "{error:#}"
        );

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn an_unclassified_diagnostic_location_cannot_default_to_a_root() {
        let root = fixture("pub fn target() {}\n\nfn main() {}\n");
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let target = inventory
            .nodes
            .iter()
            .find(|node| node.report && node.label.contains("fn target"))
            .expect("target item");
        let (instrumentation, _) = (0..inventory.antichain_rounds)
            .find_map(|round| {
                let instrumentation = inventory.instrument(&root, round).ok()?;
                instrumentation
                    .ids
                    .contains(&target.id)
                    .then_some((instrumentation, round))
            })
            .expect("target instrumentation round");
        let forged = format!(
            "libs/fixture/src/main.rs:2:1: warning: unclassified use {MARKER}{}\n",
            target.id
        );

        let error = Uses::default()
            .observe(
                RootKind::Production,
                &inventory,
                &root,
                &forged,
                &instrumentation,
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("has no classified caller"));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn an_uncalled_trait_implementation_is_not_a_production_root() {
        let root = fixture(
            r#"#![warn(deprecated)]
pub trait Observer { fn observe(&self); }
pub struct Installed;
impl Observer for Installed { fn observe(&self) {} }
pub trait Called { fn call(&self); }
pub struct Active;
impl Called for Active { fn call(&self) {} }
pub trait GenericBound {}
pub struct Generic<T>(T);
impl<T> Generic<T> where T: GenericBound { fn unused(&self) {} }
fn main() { Active.call(); }
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let mut uses = Uses::default();
        for round in 0..inventory.rounds {
            let insertions = inventory
                .instrument(&root, round)
                .expect("instrument fixture");
            let output = compile_fixture(&root, &[]);
            uses.observe(
                RootKind::Production,
                &inventory,
                &root,
                &output,
                &insertions,
            )
            .expect("read fixture diagnostics");
        }
        let dead = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        assert!(dead.iter().any(|label| label.contains("trait Observer")));
        assert!(dead.iter().any(|label| label.contains("struct Installed")));
        assert!(
            dead.iter()
                .any(|label| label.contains("trait method Observer::observe"))
        );
        assert!(!dead.iter().any(|label| label.contains("trait Called")));
        assert!(
            !dead
                .iter()
                .any(|label| label.contains("trait method Called::call"))
        );
        assert!(
            dead.iter()
                .any(|label| label.contains("trait GenericBound"))
        );
        assert!(dead.iter().any(|label| label.contains("struct Generic")));

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn nested_test_and_selftest_cfgs_stay_out_of_the_production_inventory() {
        let root = fixture(
            r#"#[cfg(any(test, feature = "selftest"))]
pub fn verification_any() {}
#[cfg(all(test, unix))]
pub fn verification_all() {}
#[cfg_attr(not(test), cfg(any()))]
pub fn verification_cfg_attr() {}
#[cfg_attr(not(feature = "selftest"), cfg(any()))]
pub fn verification_feature_attr() {}
#[cfg(any(test, unix))]
pub fn maybe_production() {}
#[cfg_attr(test, cfg(any()))]
pub fn production_when_not_testing() {}
"#,
        );
        let inventory = Inventory::discover(&root).expect("fixture inventory");
        let labels = inventory
            .nodes
            .iter()
            .filter(|node| node.report)
            .map(|node| node.label.as_str())
            .collect::<Vec<_>>();
        for excluded in [
            "verification_any",
            "verification_all",
            "verification_cfg_attr",
            "verification_feature_attr",
        ] {
            assert!(!labels.iter().any(|label| label.contains(excluded)));
        }
        for included in ["maybe_production", "production_when_not_testing"] {
            assert!(labels.iter().any(|label| label.contains(included)));
        }

        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn deprecation_suppression_cannot_hide_an_audit_marker() {
        for attribute in [
            "#![allow(deprecated)]",
            "#![expect(deprecated)]",
            "#![cfg_attr(test, allow(warnings))]",
            "macro_rules! hidden { () => { #[allow(deprecated)] fn masked() {} } }",
            "macro_rules! hidden { ($lint:ident) => { #[allow($lint)] fn masked() {} } }\nhidden!(deprecated);",
            "macro_rules! hidden { ($attribute:ident, $lint:ident) => { #[$attribute($lint)] fn masked() {} } }\nhidden!(allow, deprecated);",
        ] {
            let root = fixture(&format!("{attribute}\npub fn api() {{}}\n"));
            let error = reject_deprecation_suppression(&root).unwrap_err();
            assert!(format!("{error:#}").contains("suppresses deprecation diagnostics"));
            std::fs::remove_dir_all(&root).expect("remove fixture");
        }

        let root = fixture(
            "fn allow(_: ()) {}\nfn deprecated() {}\nmacro_rules! harmless { () => { allow(deprecated()) } }\npub fn api() {}\n",
        );
        reject_deprecation_suppression(&root).expect("ordinary calls are not lint attributes");
        std::fs::remove_dir_all(&root).expect("remove fixture");
    }

    #[test]
    fn antichain_codes_separate_every_ordered_pair() {
        let codes = antichain_codes(1_200);
        assert_eq!(codes.len(), 1_200);
        for (left_index, left) in codes.iter().enumerate() {
            for (right_index, right) in codes.iter().enumerate() {
                if left_index != right_index {
                    assert_ne!(right & !left, 0);
                }
            }
        }
    }
}
