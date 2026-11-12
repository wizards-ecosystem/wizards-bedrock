use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};
use syn::{Expr, ExprCall, ExprMethodCall, ImplItem, Item, Stmt};

use crate::paths::Paths;

use super::{cfg_test, observer_item_attributes, rust_sources_below};

/// Checks that refuse a link or non-file at a path.
const PATH_CHECKS: [&str; 7] = [
    "canonical_existing_real_directory",
    "require_confined_file",
    "require_confined_absolute_file",
    "require_contained_regular_file",
    "require_direct_file",
    "require_direct_file_if_present",
    "require_regular_file",
];

#[derive(Default)]
struct CheckedReopens {
    checks: bool,
    opens: usize,
}

impl<'ast> Visit<'ast> for CheckedReopens {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let segments = function
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if segments
                .last()
                .is_some_and(|name| PATH_CHECKS.contains(&name.as_str()))
            {
                self.checks = true;
            }
            if segments.ends_with(&["File".to_string(), "open".to_string()]) {
                self.opens += 1;
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_item(&mut self, _item: &'ast Item) {}
}

/// Every function body outside test code, including functions nested in bodies.
fn production_bodies(syntax: &syn::File) -> Vec<(String, &syn::Block)> {
    let mut found = Vec::new();
    let mut pending = syntax.items.iter().collect::<Vec<_>>();
    while let Some(item) = pending.pop() {
        if cfg_test(observer_item_attributes(item)) {
            continue;
        }
        let mut bodies = Vec::new();
        match item {
            Item::Fn(function) => bodies.push((function.sig.ident.to_string(), &*function.block)),
            Item::Impl(implementation) => {
                for member in &implementation.items {
                    if let ImplItem::Fn(method) = member
                        && !cfg_test(&method.attrs)
                    {
                        bodies.push((method.sig.ident.to_string(), &method.block));
                    }
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    pending.extend(items);
                }
            }
            _ => {}
        }
        for (name, body) in bodies {
            for statement in &body.stmts {
                if let Stmt::Item(nested) = statement {
                    pending.push(nested);
                }
            }
            found.push((name, body));
        }
    }
    found
}

/// Rust sources below `directory` outside out-of-line `#[cfg(test)] mod name;` modules.
pub(super) fn production_sources_below(directory: &Path) -> Vec<PathBuf> {
    let sources = rust_sources_below(directory);
    let mut test_modules = Vec::new();
    for path in &sources {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("reading {}: {error}", path.display()));
        let syntax = syn::parse_file(&source)
            .unwrap_or_else(|error| panic!("parsing {}: {error}", path.display()));
        let children = if path
            .file_name()
            .is_some_and(|name| name == "mod.rs" || name == "lib.rs" || name == "main.rs")
        {
            path.parent().expect("source below its root").to_path_buf()
        } else {
            path.with_extension("")
        };
        for item in &syntax.items {
            if let Item::Mod(module) = item
                && module.content.is_none()
                && cfg_test(&module.attrs)
            {
                test_modules.push(children.join(module.ident.to_string()));
            }
        }
    }
    sources
        .into_iter()
        .filter(|path| {
            !test_modules
                .iter()
                .any(|module| path.starts_with(module) || *path == module.with_extension("rs"))
        })
        .collect()
}

fn checked_reopens(relative: &str, source: &str) -> Vec<String> {
    let syntax =
        syn::parse_file(source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
    let mut violations = Vec::new();
    for (name, body) in production_bodies(&syntax) {
        let mut shape = CheckedReopens::default();
        shape.visit_block(body);
        if shape.checks && shape.opens > 0 {
            violations.push(format!(
                "{relative}: `{name}` checks a path, then follows it"
            ));
        }
    }
    violations
}

/// Record builders whose source is established again when their record is qualified, so the
/// Git that answered is the qualifying run's.
const SOURCE_WITHOUT_GIT: [&str; 1] = ["generation_record_from_digests"];

/// Publishers that must re-measure Git after its last answer.
const GIT_PUBLISHERS: [(&str, &str, &str); 2] = [
    ("xtask/src/evidence.rs", "create", "write_manifest"),
    (
        "xtask/src/qualification/record.rs",
        "commit_record",
        "rename",
    ),
];

#[derive(Default)]
struct GitUse {
    binds_source: bool,
    names_resolved: bool,
    bare_git: bool,
    calls: Vec<String>,
}

impl<'ast> Visit<'ast> for GitUse {
    fn visit_expr_call(&mut self, call: &'ast ExprCall) {
        if let Expr::Path(function) = call.func.as_ref() {
            let segments = function
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect::<Vec<_>>();
            if segments.ends_with(&["git".to_string(), "resolved".to_string()]) {
                self.names_resolved = true;
            }
            if segments.ends_with(&["Command".to_string(), "new".to_string()])
                && call.args.first().is_some_and(|argument| {
                    matches!(argument, Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(program), .. })
                        if program.value() == "git")
                })
            {
                self.bare_git = true;
            }
            if let Some(last) = segments.last() {
                self.calls.push(last.clone());
            }
        }
        visit::visit_expr_call(self, call);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "insert"
            && call.args.first().is_some_and(|argument| {
                matches!(argument, Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(key), .. })
                    if key.value() == "source.commit")
            })
        {
            self.binds_source = true;
        }
        self.calls.push(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }

    fn visit_item(&mut self, _item: &'ast Item) {}
}

fn git_bindings(relative: &str, source: &str) -> Vec<String> {
    let syntax =
        syn::parse_file(source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
    let mut violations = Vec::new();
    for (name, body) in production_bodies(&syntax) {
        let mut shape = GitUse::default();
        shape.visit_block(body);
        if shape.binds_source
            && !shape.names_resolved
            && !SOURCE_WITHOUT_GIT.contains(&name.as_str())
        {
            violations.push(format!(
                "{relative}: `{name}` binds a source without the Git that established it"
            ));
        }
        if shape.bare_git && relative != "xtask/src/git.rs" {
            violations.push(format!("{relative}: `{name}` spawns `git` from PATH"));
        }
        for (file, publisher, publication) in GIT_PUBLISHERS {
            if relative == file && name == publisher {
                let measured = shape
                    .calls
                    .iter()
                    .position(|call| call == "require_unchanged");
                let published = shape.calls.iter().position(|call| call == publication);
                if !matches!((measured, published), (Some(measured), Some(published)) if measured < published)
                {
                    violations.push(format!(
                        "{relative}: `{name}` publishes before re-measuring Git"
                    ));
                }
            }
        }
    }
    violations
}

#[test]
fn every_source_identity_names_the_git_that_established_it() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    let mut publishers = 0;
    for path in production_sources_below(&paths.root().join("xtask/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        let syntax =
            syn::parse_file(&source).unwrap_or_else(|error| panic!("parsing {relative}: {error}"));
        publishers += production_bodies(&syntax)
            .iter()
            .filter(|(name, _)| {
                GIT_PUBLISHERS
                    .iter()
                    .any(|(file, publisher, _)| *file == relative && name == publisher)
            })
            .count();
        violations.extend(git_bindings(&relative, &source));
    }
    assert_eq!(publishers, GIT_PUBLISHERS.len(), "a Git publisher moved");
    assert!(
        violations.is_empty(),
        "source identity escapes its Git binding:\n{}",
        violations.join("\n")
    );

    let fixture = "fn bound(record: &mut Record) {\n\
            record.insert(\"source.commit\", commit);\n\
            for field in crate::git::resolved()?.fields() {}\n\
        }\n\
        fn unbound(record: &mut Record) { record.insert(\"source.commit\", commit); }\n\
        fn bare() { let _ = Command::new(\"git\"); }\n";
    assert_eq!(
        git_bindings("xtask/src/fixture.rs", fixture),
        [
            "xtask/src/fixture.rs: `bare` spawns `git` from PATH",
            "xtask/src/fixture.rs: `unbound` binds a source without the Git that established it",
        ]
    );
}

#[test]
fn a_checked_path_is_opened_only_without_following_links() {
    let paths = Paths::discover().expect("workspace paths");
    let mut violations = Vec::new();
    for path in production_sources_below(&paths.root().join("xtask/src")) {
        let relative = path
            .strip_prefix(paths.root())
            .expect("source below the workspace")
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
        violations.extend(checked_reopens(&relative, &source));
    }
    assert!(
        violations.is_empty(),
        "checked paths are reopened by a following open:\n{}",
        violations.join("\n")
    );

    let fixture = "fn checked(parent: &Path, path: &Path) {\n\
            require_direct_file(parent, path, \"x\").unwrap();\n\
            let _ = std::fs::File::open(path);\n\
        }\n\
        fn unchecked(path: &Path) { let _ = File::open(path); }\n\
        #[cfg(test)]\n\
        mod tests { fn checked(p: &Path) { require_regular_file(p, \"x\"); File::open(p); } }\n";
    assert_eq!(
        checked_reopens("fixture.rs", fixture),
        ["fixture.rs: `checked` checks a path, then follows it"]
    );
    for check in ["require_confined_file", "require_confined_absolute_file"] {
        let fixture = format!(
            "fn checked(root: &Path) {{\n\
                let path = {check}(root, \"input\", \"fixture\").unwrap();\n\
                let _ = File::open(path);\n\
            }}\n"
        );
        assert_eq!(
            checked_reopens("fixture.rs", &fixture),
            ["fixture.rs: `checked` checks a path, then follows it"],
            "the {check} confinement door escaped the reopen check"
        );
        let checked_open = fixture.replace("File::open(path)", "open_real_file(path, \"fixture\")");
        assert!(checked_reopens("fixture.rs", &checked_open).is_empty());
    }
}

#[test]
fn an_out_of_line_test_module_is_read_as_test_code() {
    let root = crate::test_support::reserve_temp_directory("wizard-production-sources");
    std::fs::create_dir_all(root.join("tests")).expect("fixture test module directory");
    std::fs::create_dir_all(root.join("kept")).expect("fixture production module directory");
    for (relative, contents) in [
        ("lib.rs", "mod kept;\n#[cfg(test)]\nmod tests;\n"),
        ("kept.rs", "mod inner;\n#[cfg(test)]\nmod tests;\n"),
        ("kept/inner.rs", "fn production() {}\n"),
        ("kept/tests.rs", "fn fixture() {}\n"),
        ("tests.rs", "mod nested;\n"),
        ("tests/nested.rs", "fn fixture() {}\n"),
    ] {
        std::fs::write(root.join(relative), contents).expect("fixture source");
    }
    let found = production_sources_below(&root)
        .iter()
        .map(|path| {
            path.strip_prefix(&root)
                .expect("source below the fixture")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    std::fs::remove_dir_all(&root).expect("removing production-source fixture");
    assert_eq!(found, ["kept/inner.rs", "kept.rs", "lib.rs"]);
}
