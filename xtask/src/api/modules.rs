use super::*;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct ModuleContext {
    pub(super) scope: Vec<String>,
    pub(super) availability: Availability,
    pub(super) package: String,
    pub(super) crate_name: String,
    pub(super) target_root: PathBuf,
    pub(super) target_kind: CargoTargetKind,
    pub(super) language_main: bool,
}

#[derive(Debug)]
struct ModuleRoute {
    relative: PathBuf,
    module_directory: PathBuf,
    context: ModuleContext,
}

pub(super) fn discover_module_contexts(
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
        let source = read_bounded_authority_text(
            &root.join(&route.relative),
            "API audit routed module source",
        )?;
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

pub(super) fn source_item_path(file: &SourceFile, symbol: &str) -> String {
    format!("{}::{symbol}", file.crate_name)
}

pub(super) fn workspace_type_matches(file: &SourceFile, symbol: &str, path: &str) -> bool {
    let Some((crate_name, _)) = path.split_once("::") else {
        return false;
    };
    file.crate_name == crate_name && last_path_segment(symbol) == last_path_segment(path)
}

pub(super) fn last_path_segment(path: &str) -> &str {
    path.rsplit_once("::").map_or(path, |(_, name)| name)
}

pub(super) fn validate_kernel_entry_source(
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

pub(super) fn validate_harness_target_source(
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
    exact_kernel_entry_handler(item).is_some()
}

pub(super) fn exact_kernel_entry_handler(item: &syn::ItemMacro) -> Option<syn::Path> {
    if !item.attrs.is_empty() || !exact_path(&item.mac.path, &["wost", "entry"]) {
        return None;
    }
    item.mac.parse_body::<syn::Path>().ok().filter(|handler| {
        handler.leading_colon.is_none()
            && handler.segments.len() == 1
            && handler.segments[0].arguments.is_empty()
            && handler.segments[0].ident == "kernel_main"
    })
}

pub(super) fn audited_source(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.starts_with(Path::new("wost/src")) {
        return true;
    }
    let mut components = relative.components();
    components
        .next()
        .is_some_and(|part| part.as_os_str() == "libs")
        && components.next().is_some()
        && components
            .next()
            .is_some_and(|part| part.as_os_str() == "src")
}
