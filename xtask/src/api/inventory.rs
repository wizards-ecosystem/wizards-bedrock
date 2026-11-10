use super::*;

#[derive(Clone, Debug)]
pub(super) struct Node {
    pub(super) id: usize,
    pub(super) file: usize,
    pub(super) label: String,
    pub(super) symbol: String,
    pub(super) type_declaration: bool,
    pub(super) type_alias: bool,
    pub(super) callable: bool,
    pub(super) trait_default: bool,
    pub(super) macro_declaration: bool,
    pub(super) range: SourceRange,
    pub(super) insertion: LineColumn,
    pub(super) code: u64,
    pub(super) isolation: Option<Isolation>,
    pub(super) report: bool,
    pub(super) instrument: bool,
    pub(super) roots: RootSet,
    pub(super) trait_route: Option<TraitRoute>,
    pub(super) implementation_owner: Option<String>,
    pub(super) direct_self_construction: bool,
    pub(super) reviewed_drop_producer: Option<usize>,
    pub(super) global_allocator: bool,
    pub(super) global_allocator_type: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(super) struct RootSet {
    pub(super) production: bool,
    pub(super) verification: bool,
}

impl RootSet {
    pub(super) fn contains(self, kind: RootKind) -> bool {
        match kind {
            RootKind::Production => self.production,
            RootKind::Verification => self.production || self.verification,
            RootKind::TestHarness => self.verification,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Isolation {
    Variant,
    TraitMember,
}

const ISOLATIONS: [Isolation; 2] = [Isolation::Variant, Isolation::TraitMember];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) struct SourceRange {
    pub(super) start: LineColumn,
    pub(super) end: LineColumn,
}

impl SourceRange {
    pub(super) fn contains(self, at: LineColumn) -> bool {
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

#[derive(Clone, Debug)]
pub(super) struct SourceFile {
    pub(super) relative: PathBuf,
    pub(super) crate_name: String,
    pub(super) origin: CargoTarget,
    text: String,
    non_call_uses: Vec<SourceRange>,
    macro_syntax: Vec<SourceRange>,
    transparent_macros: Vec<SourceRange>,
    macro_paths: Vec<SourceRange>,
    pub(super) dispatch_sites: Vec<DispatchSite>,
    value_constructions: Vec<ValueConstruction>,
}

#[derive(Clone, Debug)]
pub(super) struct Inventory {
    pub(super) files: Vec<SourceFile>,
    pub(super) nodes: Vec<Node>,
    pub(super) antichain_rounds: usize,
    pub(super) rounds: usize,
    pub(super) trait_activations: Vec<TraitActivation>,
    pub(super) implicit_trait_activations: Vec<ImplicitTraitActivation>,
    pub(super) unresolved_trait_implementations: BTreeSet<usize>,
    pub(super) entry: Option<EntryLink>,
}

impl Inventory {
    pub(super) fn discover_from_catalog(root: &Path, catalog: &CargoCatalog) -> Result<Self> {
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

    pub(super) fn discover_with_targets_and_entry_authority(
        root: &Path,
        targets: &[CargoTarget],
        authority: Option<&EntryContractAuthority>,
    ) -> Result<Self> {
        let (audited, paths) = discover_sources(root)?;
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
            let text = read_bounded_authority_text(&path, "API audit workspace source")?;
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
            let origin = source_origin(&relative, context, targets)?;
            let syntax =
                syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
            validate_kernel_entry_source(&relative, context, &syntax)?;
            validate_harness_target_source(&relative, context, &syntax)?;
            if audited.contains(&path) {
                validate_audited_attributes(&relative, &syntax)?;
            }
            let source =
                SourceFile::scan(relative, context, origin, text, &syntax, &macro_policies)?;
            let trait_imports = TraitImports::collect_in_scope(&syntax.items, &context.scope);
            collect_source_nodes(
                Collector {
                    audit: audited.contains(&path),
                    file: files.len(),
                    relative: &source.relative,
                    nodes: &mut nodes,
                    trait_imports: &trait_imports,
                    unsupported_macros: Vec::new(),
                    entry_specs: &mut entry_specs,
                    scope: context.scope.clone(),
                    package: &context.package,
                    target_root: &context.target_root,
                    target_kind: context.target_kind,
                    language_main: context.language_main,
                    lexical_item_depth: 0,
                },
                &syntax.items,
                context.availability,
            )?;
            files.push(source);
        }

        ensure!(
            nodes.iter().any(|node| node.report),
            "the public API inventory is empty"
        );
        let antichain_rounds = assign_ids_and_codes(&mut nodes, &files)?;
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

    pub(super) fn instrument(&self, root: &Path, round: usize) -> Result<Instrumentation> {
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

    pub(super) fn caller(&self, file: usize, at: LineColumn) -> Option<usize> {
        self.nodes
            .iter()
            .filter(|node| node.file == file && node.range.contains(at))
            .min_by_key(|node| node.range.extent())
            .map(|node| node.id)
    }

    pub(super) fn is_non_call_use(&self, file: usize, at: LineColumn) -> bool {
        self.files[file]
            .non_call_uses
            .iter()
            .any(|range| range.contains(at))
    }

    pub(super) fn exact_entry_invocation_caller(
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

    pub(super) fn exact_entry_contract_expansion_caller(
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

    pub(super) fn is_opaque_macro_use(&self, file: usize, at: LineColumn) -> bool {
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

    pub(super) fn is_direct_macro_call(&self, file: usize, at: LineColumn, used: usize) -> bool {
        self.nodes[used].macro_declaration
            && self.files[file]
                .macro_paths
                .iter()
                .any(|range| range.contains(at))
    }

    pub(super) fn dispatch_site(&self, file: usize, at: LineColumn) -> Option<usize> {
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

    pub(super) fn is_value_construction(&self, file: usize, at: LineColumn, used: usize) -> bool {
        self.nodes[used].type_declaration
            && self.files[file]
                .value_constructions
                .iter()
                .any(|site| site.range.contains(at))
    }

    pub(super) fn file_index(&self, root: &Path, reported: &str) -> Option<usize> {
        if let Some(relative) = virtual_source_relative(reported) {
            return self.files.iter().position(|file| file.relative == relative);
        }
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

    pub(super) fn report_count(&self) -> usize {
        self.nodes.iter().filter(|node| node.report).count()
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Insertion {
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) bytes: usize,
}

#[derive(Debug)]
pub(super) struct Instrumentation {
    pub(super) insertions: BTreeMap<usize, Vec<Insertion>>,
    pub(super) ids: BTreeSet<usize>,
}

pub(super) fn range(span: Span) -> SourceRange {
    SourceRange {
        start: span.start(),
        end: span.end(),
    }
}

impl SourceFile {
    fn scan(
        relative: PathBuf,
        context: &ModuleContext,
        origin: &CargoTarget,
        text: String,
        syntax: &syn::File,
        macro_policies: &BTreeMap<PathBuf, bool>,
    ) -> Result<Self> {
        let mut non_call_uses = NonCallUses::default();
        non_call_uses.visit_file(syntax);
        let mut macro_syntax = MacroSyntax {
            unqualified_standard: *macro_policies
                .get(&context.target_root)
                .context("a classified source has no macro-resolution policy")?,
            ..MacroSyntax::default()
        };
        macro_syntax.visit_file(syntax);
        let mut dispatch_sites = DispatchSites::default();
        dispatch_sites.visit_file(syntax);
        let mut value_constructions = ValueConstructions::default();
        value_constructions.visit_file(syntax);
        Ok(Self {
            relative,
            crate_name: context.crate_name.clone(),
            origin: origin.clone(),
            text,
            non_call_uses: non_call_uses.ranges,
            macro_syntax: macro_syntax.ranges,
            transparent_macros: macro_syntax.transparent,
            macro_paths: macro_syntax.paths,
            dispatch_sites: dispatch_sites.sites,
            value_constructions: value_constructions.sites,
        })
    }
}

fn discover_sources(root: &Path) -> Result<(BTreeSet<PathBuf>, Vec<PathBuf>)> {
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
    Ok((audited, paths))
}

fn source_origin<'targets>(
    relative: &Path,
    context: &ModuleContext,
    targets: &'targets [CargoTarget],
) -> Result<&'targets CargoTarget> {
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
    Ok(*origin)
}

fn collect_source_nodes(
    mut collector: Collector<'_>,
    items: &[Item],
    availability: Availability,
) -> Result<()> {
    for item in items {
        collector.item(item, availability);
    }
    ensure!(
        collector.unsupported_macros.is_empty(),
        "{} contains item macro invocation(s) whose public expansion the API audit cannot inventory: {}",
        collector.relative.display(),
        collector.unsupported_macros.join(", ")
    );
    Ok(())
}

fn assign_ids_and_codes(nodes: &mut [Node], files: &[SourceFile]) -> Result<usize> {
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
            node.code = codes
                .next()
                .with_context(|| format!("assigning an antichain code to {}", node.label))?;
        }
    }
    Ok(antichain_rounds)
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

pub(super) fn optional_path_is_present(path: &Path) -> Result<bool> {
    path.try_exists()
        .with_context(|| format!("checking whether {} exists", path.display()))
}

pub(super) fn collect_optional_rust_files(
    directory: &Path,
    paths: &mut Vec<PathBuf>,
) -> Result<()> {
    if optional_path_is_present(directory)? {
        collect_rust_files(directory, paths)?;
    }
    Ok(())
}

pub(super) fn antichain_codes(count: usize) -> Vec<u64> {
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

pub(super) fn byte_offset(text: &str, at: LineColumn) -> Result<usize> {
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RootKind {
    Production,
    Verification,
    TestHarness,
}
