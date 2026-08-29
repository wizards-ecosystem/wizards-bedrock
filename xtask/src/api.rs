//! Compiler-resolved evidence that every public foundation API has a caller.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use proc_macro2::{LineColumn, Span};
use syn::spanned::Spanned;
use syn::{Attribute, ForeignItem, ImplItem, Item, TraitItem, Visibility};

use crate::gate;
use crate::paths::Paths;

const MARKER: &str = "wizard-api-audit:";
const SOURCE_ROOTS: [&str; 2] = ["libs", "wost/src"];
const COPY_DIRECTORIES: [&str; 6] = [".cargo", "kernel", "libs", "tools", "wost", "xtask"];
const COPY_FILES: [&str; 3] = ["Cargo.lock", "Cargo.toml", "rust-toolchain.toml"];

#[derive(Debug, Clone)]
struct Node {
    id: usize,
    file: usize,
    label: String,
    range: SourceRange,
    insertion: LineColumn,
    code: u64,
}

#[derive(Debug, Clone, Copy)]
struct SourceRange {
    start: LineColumn,
    end: LineColumn,
}

impl SourceRange {
    fn contains(self, at: LineColumn) -> bool {
        (at.line > self.start.line || at.line == self.start.line && at.column >= self.start.column)
            && (at.line < self.end.line || at.line == self.end.line && at.column <= self.end.column)
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
    text: String,
    reexports: Vec<SourceRange>,
}

#[derive(Debug, Clone)]
struct Inventory {
    files: Vec<SourceFile>,
    nodes: Vec<Node>,
    rounds: usize,
}

impl Inventory {
    fn discover(root: &Path) -> Result<Self> {
        let mut paths = Vec::new();
        for relative in SOURCE_ROOTS {
            collect_rust_files(&root.join(relative), &mut paths)?;
        }
        paths.sort();

        let mut files = Vec::new();
        let mut nodes = Vec::new();
        for path in paths {
            let relative = path
                .strip_prefix(root)
                .with_context(|| format!("{} is outside the audit root", path.display()))?
                .to_path_buf();
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let syntax =
                syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
            let file = files.len();
            let reexports = {
                let mut collector = Collector {
                    file,
                    relative: &relative,
                    nodes: &mut nodes,
                    reexports: Vec::new(),
                    scope: Vec::new(),
                };
                for item in &syntax.items {
                    collector.item(item, false);
                }
                collector.reexports
            };
            files.push(SourceFile {
                relative,
                text,
                reexports,
            });
        }

        ensure!(!nodes.is_empty(), "the public API inventory is empty");
        nodes.sort_by_key(|node| {
            (
                files[node.file].relative.clone(),
                node.insertion.line,
                node.insertion.column,
                node.label.clone(),
            )
        });
        let codes = antichain_codes(nodes.len());
        let rounds = codes
            .iter()
            .map(|code| 64 - code.leading_zeros() as usize)
            .max()
            .unwrap_or(0);
        for (id, (node, code)) in nodes.iter_mut().zip(codes).enumerate() {
            node.id = id;
            node.code = code;
        }
        ensure!(rounds > 0, "the API audit produced no compiler rounds");

        Ok(Self {
            files,
            nodes,
            rounds,
        })
    }

    fn instrument(&self, root: &Path, round: usize) -> Result<BTreeMap<usize, Vec<Insertion>>> {
        let mut by_file: BTreeMap<usize, Vec<&Node>> = BTreeMap::new();
        for node in self
            .nodes
            .iter()
            .filter(|node| node.code & (1 << round) != 0)
        {
            by_file.entry(node.file).or_default().push(node);
        }

        let mut inserted = BTreeMap::new();
        for (index, file) in self.files.iter().enumerate() {
            let mut text = file.text.clone();
            let mut additions = Vec::new();
            let mut selected = by_file.get(&index).cloned().unwrap_or_default();
            selected.sort_by_key(|node| {
                std::cmp::Reverse((node.insertion.line, node.insertion.column))
            });
            for node in selected {
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
        Ok(inserted)
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

    fn file_index(&self, root: &Path, reported: &str) -> Option<usize> {
        let reported = normalise(reported);
        let root = normalise(&root.to_string_lossy());
        let relative = reported
            .strip_prefix(&format!("{root}/"))
            .unwrap_or(&reported);
        self.files
            .iter()
            .position(|file| normalise(&file.relative.to_string_lossy()) == relative)
    }
}

#[derive(Debug, Clone, Copy)]
struct Insertion {
    line: usize,
    column: usize,
    bytes: usize,
}

struct Collector<'a> {
    file: usize,
    relative: &'a Path,
    nodes: &'a mut Vec<Node>,
    reexports: Vec<SourceRange>,
    scope: Vec<String>,
}

impl Collector<'_> {
    fn item(&mut self, item: &Item, skipped: bool) {
        let skipped = skipped || cfg_test(item_attrs(item));
        if skipped {
            return;
        }

        match item {
            Item::Const(item) if public(&item.vis) => {
                self.record("const", &item.ident.to_string(), item.span())
            }
            Item::Enum(item) if public(&item.vis) => {
                self.record("enum", &item.ident.to_string(), item.span());
                self.scope.push(item.ident.to_string());
                for variant in &item.variants {
                    self.record("variant", &variant.ident.to_string(), variant.span());
                }
                self.scope.pop();
            }
            Item::Fn(item) if public(&item.vis) => {
                self.record("fn", &item.sig.ident.to_string(), item.span())
            }
            Item::ForeignMod(item) => {
                for foreign in &item.items {
                    match foreign {
                        ForeignItem::Fn(item) if public(&item.vis) => {
                            self.record("foreign fn", &item.sig.ident.to_string(), item.span())
                        }
                        ForeignItem::Static(item) if public(&item.vis) => {
                            self.record("foreign static", &item.ident.to_string(), item.span())
                        }
                        ForeignItem::Type(item) if public(&item.vis) => {
                            self.record("foreign type", &item.ident.to_string(), item.span())
                        }
                        _ => {}
                    }
                }
            }
            Item::Impl(item) => {
                self.scope.push("impl".to_string());
                for member in &item.items {
                    match member {
                        ImplItem::Const(member) if public(&member.vis) => self.record(
                            "associated const",
                            &member.ident.to_string(),
                            member.span(),
                        ),
                        ImplItem::Fn(member) if public(&member.vis) => {
                            self.record("method", &member.sig.ident.to_string(), member.span())
                        }
                        ImplItem::Type(member) if public(&member.vis) => {
                            self.record("associated type", &member.ident.to_string(), member.span())
                        }
                        _ => {}
                    }
                }
                self.scope.pop();
            }
            Item::Macro(item) if has_attribute(&item.attrs, "macro_export") => {
                if let Some(ident) = &item.ident {
                    self.record("macro", &ident.to_string(), item.span());
                }
            }
            Item::Mod(item) => {
                if let Some((_, items)) = &item.content {
                    self.scope.push(item.ident.to_string());
                    for item in items {
                        self.item(item, skipped);
                    }
                    self.scope.pop();
                }
            }
            Item::Static(item) if public(&item.vis) => {
                self.record("static", &item.ident.to_string(), item.span())
            }
            Item::Struct(item) if public(&item.vis) => {
                self.record("struct", &item.ident.to_string(), item.span());
            }
            Item::Trait(item) if public(&item.vis) => {
                self.record("trait", &item.ident.to_string(), item.span());
                self.scope.push(item.ident.to_string());
                for member in &item.items {
                    match member {
                        TraitItem::Const(member) => {
                            self.record("trait const", &member.ident.to_string(), member.span())
                        }
                        TraitItem::Fn(member) => self.record(
                            "trait method",
                            &member.sig.ident.to_string(),
                            member.span(),
                        ),
                        TraitItem::Type(member) => {
                            self.record("trait type", &member.ident.to_string(), member.span())
                        }
                        _ => {}
                    }
                }
                self.scope.pop();
            }
            Item::Type(item) if public(&item.vis) => {
                self.record("type", &item.ident.to_string(), item.span())
            }
            Item::Union(item) if public(&item.vis) => {
                self.record("union", &item.ident.to_string(), item.span());
            }
            Item::Use(item) if public(&item.vis) => {
                self.reexports.push(range(item.span()));
            }
            _ => {}
        }
    }

    fn record(&mut self, kind: &str, name: &str, span: Span) {
        let mut qualified = self.scope.clone();
        qualified.push(name.to_string());
        let qualified = qualified.join("::");
        self.nodes.push(Node {
            id: 0,
            file: self.file,
            label: format!(
                "{kind} {qualified} at {}:{}",
                self.relative.display(),
                span.start().line
            ),
            range: range(span),
            insertion: span.start(),
            code: 0,
        });
    }
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

fn cfg_test(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Ident>()
                .is_ok_and(|ident| ident == "test")
    })
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
    for entry in
        std::fs::read_dir(directory).with_context(|| format!("reading {}", directory.display()))?
    {
        let entry =
            entry.with_context(|| format!("reading an entry in {}", directory.display()))?;
        let kind = entry
            .file_type()
            .with_context(|| format!("reading the type of {}", entry.path().display()))?;
        if kind.is_dir() {
            collect_rust_files(&entry.path(), paths)?;
        } else if kind.is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        {
            paths.push(entry.path());
        }
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
    let line_start = text
        .split_inclusive('\n')
        .take(at.line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    let offset = line_start.saturating_add(at.column);
    ensure!(
        text.is_char_boundary(offset),
        "compiler span at {}:{} is not a UTF-8 boundary",
        at.line,
        at.column
    );
    Ok(offset)
}

fn normalise(path: &str) -> String {
    path.replace('\\', "/")
}

#[derive(Debug, Default)]
struct Uses {
    roots: BTreeSet<usize>,
    edges: BTreeMap<usize, BTreeSet<usize>>,
}

impl Uses {
    fn observe(
        &mut self,
        inventory: &Inventory,
        root: &Path,
        output: &str,
        insertions: &BTreeMap<usize, Vec<Insertion>>,
    ) -> Result<()> {
        for line in output.lines().filter(|line| line.contains(MARKER)) {
            let marker = line
                .find(MARKER)
                .context("an API diagnostic lost its marker")?;
            let id = line[marker + MARKER.len()..]
                .chars()
                .take_while(|character| character.is_ascii_digit())
                .collect::<String>()
                .parse::<usize>()
                .context("an API diagnostic has a malformed marker")?;
            ensure!(
                id < inventory.nodes.len(),
                "an API diagnostic names unknown item {id}"
            );

            let location = &line[..marker];
            let warning = location
                .find(": warning:")
                .context("an API diagnostic is not in short compiler format")?;
            let mut parts = location[..warning].rsplitn(3, ':');
            let column = parts
                .next()
                .context("an API diagnostic has no column")?
                .parse::<usize>()
                .context("an API diagnostic column is not a number")?;
            let source_line = parts
                .next()
                .context("an API diagnostic has no line")?
                .parse::<usize>()
                .context("an API diagnostic line is not a number")?;
            let reported = parts.next().context("an API diagnostic has no path")?;
            let Some(file) = inventory.file_index(root, reported) else {
                self.roots.insert(id);
                continue;
            };
            let mut column = column.saturating_sub(1);
            for insertion in insertions
                .get(&file)
                .into_iter()
                .flatten()
                .filter(|insertion| insertion.line == source_line)
            {
                if column >= insertion.column + insertion.bytes {
                    column -= insertion.bytes;
                }
            }
            let at = LineColumn {
                line: source_line,
                column,
            };
            if inventory.is_reexport(file, at) {
                continue;
            }
            match inventory.caller(file, at) {
                Some(caller) if caller != id => {
                    self.edges.entry(caller).or_default().insert(id);
                }
                Some(_) => {}
                None => {
                    self.roots.insert(id);
                }
            }
        }
        Ok(())
    }

    fn dead<'a>(&self, inventory: &'a Inventory) -> Vec<&'a Node> {
        let mut live = self.roots.clone();
        let mut pending: VecDeque<usize> = live.iter().copied().collect();
        while let Some(caller) = pending.pop_front() {
            for &used in self.edges.get(&caller).into_iter().flatten() {
                if live.insert(used) {
                    pending.push_back(used);
                }
            }
        }
        inventory
            .nodes
            .iter()
            .filter(|node| !live.contains(&node.id))
            .collect()
    }
}

struct AuditWorkspace {
    _lock: File,
    root: PathBuf,
    build: PathBuf,
}

impl AuditWorkspace {
    fn create(paths: &Paths) -> Result<Self> {
        let state = paths.target_dir().join("api-audit");
        std::fs::create_dir_all(&state).with_context(|| format!("creating {}", state.display()))?;
        let lock_path = state.join("LOCK");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        lock.try_lock()
            .context("another public API audit is already running")?;

        let root = state.join("workspace");
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .with_context(|| format!("removing stale {}", root.display()))?;
        }
        std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
        for file in COPY_FILES {
            copy_file(&paths.root().join(file), &root.join(file))?;
        }
        for directory in COPY_DIRECTORIES {
            copy_tree(&paths.root().join(directory), &root.join(directory))?;
        }

        Ok(Self {
            _lock: lock,
            root,
            build: state.join("build"),
        })
    }
}

impl Drop for AuditWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("creating {}", destination.display()))?;
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
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::copy(source, destination)
        .with_context(|| format!("copying {} to {}", source.display(), destination.display()))?;
    Ok(())
}

/// Compile every public `libs/` and `wost` item against all of its real callers.
pub fn check(paths: &Paths) -> Result<()> {
    let workspace = AuditWorkspace::create(paths)?;
    let inventory = Inventory::discover(&workspace.root)?;
    let mut uses = Uses::default();
    println!(
        "[xtask] auditing {} public API items in {} compiler rounds",
        inventory.nodes.len(),
        inventory.rounds
    );
    for round in 0..inventory.rounds {
        let insertions = inventory.instrument(&workspace.root, round)?;
        let output = compile_graphs(&workspace, round)?;
        uses.observe(&inventory, &workspace.root, &output, &insertions)?;
    }

    let dead = uses.dead(&inventory);
    if dead.is_empty() {
        println!(
            "[xtask] all {} public API items have compiler-resolved callers",
            inventory.nodes.len()
        );
        return Ok(());
    }
    let report = dead
        .iter()
        .map(|node| format!("\n  {}", node.label))
        .collect::<String>();
    bail!("{} public API item(s) have no caller:{report}", dead.len());
}

fn compile_graphs(workspace: &AuditWorkspace, round: usize) -> Result<String> {
    let mut output = String::new();
    for pass in gate::REGISTRY.cargo_passes() {
        let mut args = vec![
            "check".to_string(),
            "--offline".to_string(),
            "--message-format=short".to_string(),
        ];
        if pass.target.is_none() {
            args.push("--all-targets".to_string());
        }
        args.extend(crate::cargo::package_args(pass.packages));
        if let Some(target) = pass.target {
            args.extend(["--target".to_string(), target.to_string()]);
        }
        if let Some(features) = pass.features {
            args.extend(["--features".to_string(), features.to_string()]);
        }
        let result = Command::new(env!("CARGO"))
            .current_dir(&workspace.root)
            .env("CARGO_TARGET_DIR", &workspace.build)
            .args(&args)
            .output()
            .with_context(|| format!("spawning API audit round {round} for {}", pass.label()))?;
        output.push_str(&String::from_utf8_lossy(&result.stdout));
        output.push_str(&String::from_utf8_lossy(&result.stderr));
        if !result.status.success() {
            bail!(
                "API audit round {round} failed to compile {} ({}):\n{}",
                pass.label(),
                result.status,
                output
            );
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    static NEXT: AtomicUsize = AtomicUsize::new(0);

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
    pub fn documented() {}
    pub fn same_name() {}
    pub mod nested { pub fn same_name() {} }
}
pub use api::{reexport_only, reexported as public_name};
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
            uses.observe(&inventory, &root, &output, &insertions)
                .expect("read fixture diagnostics");
        }
        let dead: Vec<&str> = uses
            .dead(&inventory)
            .iter()
            .map(|node| node.label.as_str())
            .collect();
        assert!(dead.iter().any(|label| label.contains("reexport_only")));
        assert!(dead.iter().any(|label| label.contains("documented")));
        assert!(dead.iter().any(|label| label.contains("fn api::same_name")));
        assert!(!dead.iter().any(|label| label.contains("reexported")));
        assert!(!dead.iter().any(|label| label.contains("nested::same_name")));

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
