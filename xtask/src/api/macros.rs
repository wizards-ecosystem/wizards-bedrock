use syn::parse::Parse;
use syn::visit::{self, Visit};

use super::*;

#[derive(Default, Debug)]
pub(super) struct MacroSyntax {
    pub(super) ranges: Vec<SourceRange>,
    pub(super) transparent: Vec<SourceRange>,
    pub(super) paths: Vec<SourceRange>,
    pub(super) unqualified_standard: bool,
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

#[derive(Debug)]
pub(super) struct MacroShadowPolicy<'context> {
    pub(super) shadows_standard: bool,
    pub(super) reserved_root_rebinding: Option<String>,
    pub(super) context: &'context ModuleContext,
    pub(super) relative: &'context Path,
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

pub(super) fn discover_macro_policies(
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
        let source = read_bounded_authority_text(path, "API audit macro-resolution source")?;
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
    {
        return false;
    }
    exact_kernel_entry_handler(item).is_some()
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

#[derive(Debug)]
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
