//! Compiler-resolved evidence that every public foundation API has a caller.

mod catalog;
mod collector;
mod compile;
mod compiler;
mod conditions;
mod diagnostics;
mod dispatch;
mod entry;
mod inventory;
mod macros;
mod modules;
mod reviewed;
mod traits;
mod uses;
mod workspace;

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use proc_macro2::{Delimiter, LineColumn, Span, TokenStream, TokenTree};
use syn::parse::{ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::{
    Attribute, Expr, ForeignItem, ImplItem, Item, Lit, Meta, Token, TraitItem, UseTree, Visibility,
};

use crate::gate;
use crate::paths::Paths;

use self::catalog::*;
use self::collector::*;
use self::compile::*;
use self::compiler::*;
use self::conditions::*;
use self::diagnostics::*;
use self::dispatch::*;
use self::entry::*;
use self::inventory::*;
use self::macros::*;
use self::modules::*;
use self::reviewed::*;
use self::traits::*;
use self::uses::*;
use self::workspace::*;

const MARKER: &str = "wizard-api-audit:";
const SOURCE_ROOTS: [&str; 2] = ["libs", "wost/src"];
const COPY_DIRECTORIES: [&str; 6] = [".cargo", "kernel", "libs", "tools", "wost", "xtask"];
const COPY_FILES: [&str; 3] = ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml"];
const RESERVED_LANGUAGE_ROOTS: [&str; 3] = ["std", "core", "alloc"];
/// Metadata is small but may still need to resolve the whole offline graph.
const METADATA_TIMEOUT: Duration = Duration::from_secs(120);
/// One pristine compilation graph may include every workspace package.
const COMPILATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// Captured output, streamed stderr and retained typed compiler evidence each
/// keep a fixed 64 MiB ceiling.
const MAX_AUDIT_OUTPUT_BYTES: usize = 64 * 1024 * 1024;
/// One Cargo JSON record is parsed before its typed evidence is retained.
const MAX_AUDIT_JSON_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Count separately bounds spare vector capacity; retained bytes also charge
/// one complete fixed record before charging every owned field.
const MAX_AUDIT_RETAINED_MARKERS: usize =
    MAX_AUDIT_OUTPUT_BYTES / std::mem::size_of::<MarkerDiagnostic>();
/// Preserve enough compact compiler errors to diagnose a refused graph.
const MAX_AUDIT_FAILURE_BYTES: usize = 1024 * 1024;
const TOOL_IDENTITY_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_TOOL_IDENTITY_BYTES: usize = 64 * 1024;
/// Compiler binaries are streamed; valid text is retained only for the decoded-literal scan in
/// `reject_authority_diagnostic_marker`.
const COMPILER_AUTHORITY_BUFFER_BYTES: usize = 64 * 1024;
const MAX_COMPILER_AUTHORITY_TEXT_BYTES: usize = 8 * 1024 * 1024;
const REGISTRY_MATERIALIZATION_TIMEOUT: Duration = Duration::from_secs(10 * 60);

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
    let Some(header) = lines.next() else {
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
    let Some(row) = lines.next().and_then(|line| line.strip_prefix("  ")) else {
        return false;
    };
    if lines.next().is_some() {
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

#[derive(Clone, Debug)]
enum CompilationMode {
    RegisteredPass,
    Example(CargoTarget),
    TestHarness(CargoTarget),
}

#[derive(Clone, Debug)]
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

#[cfg(test)]
mod tests;
