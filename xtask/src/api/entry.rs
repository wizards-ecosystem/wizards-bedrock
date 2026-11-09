use syn::visit::{self, Visit};

use super::*;

#[derive(Debug, Clone)]
pub(super) struct EntrySpec {
    pub(super) file: usize,
    pub(super) invocation: SourceRange,
    pub(super) macro_path: SourceRange,
    pub(super) argument: SourceRange,
    pub(super) handler: String,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EntryContractUse {
    symbol: &'static str,
    primary: SourceRange,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EntryContractAuthority {
    pub(super) uses: [EntryContractUse; 4],
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EntryContractNode {
    pub(super) id: usize,
    pub(super) primary: SourceRange,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct EntryLink {
    pub(super) file: usize,
    pub(super) invocation: SourceRange,
    pub(super) macro_path: SourceRange,
    pub(super) argument: SourceRange,
    pub(super) invocation_node: usize,
    pub(super) function_node: usize,
    pub(super) macro_declaration: usize,
    pub(super) contract_nodes: [EntryContractNode; 4],
}

pub(super) fn virtual_source_relative(reported: &str) -> Option<PathBuf> {
    let portable = reported.replace('\\', "/");
    let prefix = format!("{}/", crate::cargo::VIRTUAL_SOURCE_ROOT);
    let relative = PathBuf::from(portable.strip_prefix(&prefix)?);
    let mut components = relative.components();
    let first = components.next()?;
    (matches!(first, std::path::Component::Normal(_))
        && components.all(|component| matches!(component, std::path::Component::Normal(_))))
    .then_some(relative)
}

pub(super) fn resolve_entry_link(
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
                    && node.symbol == spec.handler
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

pub(super) fn validate_wost_entry_contract(root: &Path) -> Result<[EntryContractUse; 4]> {
    let relative = Path::new("wost/src/boot/mod.rs");
    let source =
        read_bounded_authority_text(&root.join(relative), "API audit wost entry-contract source")?;
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
    let mut signature = None;
    let mut base_revision = None;
    let mut entry_point = None;
    for item in &generated.items {
        match item {
            Item::Const(item)
                if item.ident == "__WOST_ENTRY_SIGNATURE"
                    && matches!(item.vis, Visibility::Inherited) =>
            {
                signature = Some(item);
            }
            Item::Static(item) if matches!(item.vis, Visibility::Inherited) => {
                if item.ident == "__WOST_BASE_REVISION" {
                    base_revision = Some(item);
                }
                actual_statics.insert(item.ident.to_string());
            }
            Item::Fn(item)
                if item.sig.ident == "kmain" && matches!(item.vis, Visibility::Inherited) =>
            {
                entry_point = Some(item);
            }
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
    let Some(signature) = signature else {
        bail!("the `wost::entry` expansion must bind one entry signature");
    };
    ensure!(
        signature.attrs.is_empty()
            && matches!(&*signature.expr, Expr::Path(path) if path.path.is_ident("kernel_main")),
        "the `wost::entry` signature must bind the supplied handler directly"
    );
    let Some(base_revision) = base_revision else {
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
    let Some(entry_point) = entry_point else {
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

pub(super) fn validate_audited_attributes(relative: &Path, syntax: &syn::File) -> Result<()> {
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
