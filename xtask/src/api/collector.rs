use syn::visit::{self, Visit};

use super::*;

#[derive(Default)]
pub(super) struct NonCallUses {
    pub(super) ranges: Vec<SourceRange>,
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

pub(super) struct Collector<'source> {
    pub(super) audit: bool,
    pub(super) file: usize,
    pub(super) relative: &'source Path,
    pub(super) nodes: &'source mut Vec<Node>,
    pub(super) trait_imports: &'source TraitImports,
    pub(super) unsupported_macros: Vec<String>,
    pub(super) entry_specs: &'source mut Vec<EntrySpec>,
    pub(super) scope: Vec<String>,
    pub(super) package: &'source str,
    pub(super) target_root: &'source Path,
    pub(super) target_kind: CargoTargetKind,
    pub(super) language_main: bool,
    pub(super) lexical_item_depth: usize,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Availability {
    Production,
    Verification,
}

impl Availability {
    pub(super) fn for_item(self, attributes: &[Attribute]) -> Self {
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
    pub(super) fn item(&mut self, item: &Item, parent: Availability) {
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
        {
            return false;
        }
        let Some(handler) = exact_kernel_entry_handler(item) else {
            return false;
        };
        self.entry_specs.push(EntrySpec {
            file: self.file,
            invocation: range(item.span()),
            macro_path: range(item.mac.path.span()),
            argument: range(handler.span()),
            handler: handler.segments[0].ident.to_string(),
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

pub(super) fn trait_path(path: &syn::Path) -> String {
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

pub(super) fn exact_nominal_type_path(ty: &syn::Type) -> Option<String> {
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

pub(super) fn has_semantic_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident(name)
            || attribute.path().is_ident("unsafe")
                && attribute
                    .parse_args::<Meta>()
                    .is_ok_and(|meta| meta.path().is_ident(name))
    })
}

pub(super) fn exact_path(path: &syn::Path, expected: &[&str]) -> bool {
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
