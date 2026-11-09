use std::collections::BTreeMap;

use syn::visit::{self, Visit};
use syn::{
    Expr, ExprCall, GenericArgument, ImplItem, ImplItemFn, Item, ItemEnum, ItemFn, ItemStruct,
    Macro, PathArguments, Type,
};

use crate::paths::Paths;

use super::sources::production_sources_below;
use super::{
    cfg_test, observer_impl_item_attributes, observer_item_attributes, require_error,
    require_success,
};

#[test]
fn pci_configuration_writes_require_function_authority() {
    let paths = Paths::discover().expect("workspace paths");
    for (label, authority) in [
        ("pure", "pci_config::FunctionAuthority"),
        ("hardware", "wost::arch::current::pci::FunctionClaim"),
    ] {
        for bound in ["Copy", "Clone"] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<{authority}>(); }}\n"
            );
            require_error(
                &paths,
                &format!("pci-function-{label}-linear"),
                &source,
                "E0277",
            )
            .unwrap_or_else(|diagnostics| {
                panic!("PCI {label} function authority satisfied {bound}:\n{diagnostics}")
            });
        }
    }

    let pure_forgery = "#![no_std]\n\
        fn probe(authority: pci_config::FunctionAuthority) {\n\
            let pci_config::FunctionAuthority { lease: _, .. } = authority;\n\
        }\n";
    require_error(&paths, "pci-pure-function-forgery", pure_forgery, "E0451").unwrap_or_else(
        |diagnostics| {
            panic!("pure PCI function authority exposed its private lease:\n{diagnostics}")
        },
    );

    let hardware_forgery = "#![no_std]\n\
        fn probe(authority: wost::arch::current::pci::FunctionClaim) {\n\
            let wost::arch::current::pci::FunctionClaim { identity: _, .. } = authority;\n\
        }\n";
    require_error(
        &paths,
        "pci-hardware-function-forgery",
        hardware_forgery,
        "E0451",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("hardware PCI function authority exposed its private identity:\n{diagnostics}")
    });

    for (bound, label) in [("Copy", "copy"), ("Clone", "clone")] {
        let source = format!(
            "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
             fn probe() {{ requires::<wost::arch::current::pci::NvmeBusMasterAuthority<'static>>(); }}\n"
        );
        require_error(
            &paths,
            &format!("nvme-bus-master-authority-{label}"),
            &source,
            "E0277",
        )
        .unwrap_or_else(|diagnostics| {
            panic!("NVMe bus-master authority satisfied {bound}:\n{diagnostics}")
        });
    }

    let forged = "#![no_std]\n\
        use wost::arch::current::pci::{MappedBar, NvmeBusMasterAuthority};\n\
        fn forge<'a>(bar: &'a MappedBar, observed: nvme::DisabledRegisters) -> NvmeBusMasterAuthority<'a> {\n\
            NvmeBusMasterAuthority { bar, _observed: observed }\n\
        }\n";
    require_error(&paths, "nvme-bus-master-forgery", forged, "E0451").unwrap_or_else(
        |diagnostics| panic!("NVMe bus-master authority became forgeable:\n{diagnostics}"),
    );

    // The domain proof is minted only by a programmed unit (D-357).
    let forged_domain = "#![no_std]\n\
        fn forge(function: pci_config::Address) -> wost::arch::current::vtd::DmaDomain<'static> {\n\
            wost::arch::current::vtd::DmaDomain { function, _unit: core::marker::PhantomData }\n\
        }\n";
    require_error(&paths, "dma-domain-forgery", forged_domain, "E0451").unwrap_or_else(
        |diagnostics| panic!("the DMA domain proof became forgeable:\n{diagnostics}"),
    );
    let reused_domain = "#![no_std]\n\
        fn spend(_: wost::arch::current::vtd::DmaDomain) {}\n\
        fn probe(domain: wost::arch::current::vtd::DmaDomain) {\n\
            spend(domain);\n\
            spend(domain);\n\
        }\n";
    require_error(&paths, "dma-domain-reuse", reused_domain, "E0382").unwrap_or_else(
        |diagnostics| panic!("one DMA domain proof could be spent twice:\n{diagnostics}"),
    );
    let cloned_domain = "#![no_std]\nfn requires<T: Clone>() {}\n\
        fn probe() { requires::<wost::arch::current::vtd::DmaDomain>(); }\n";
    require_error(&paths, "dma-domain-clone", cloned_domain, "E0277")
        .unwrap_or_else(|diagnostics| panic!("a DMA domain proof could be cloned:\n{diagnostics}"));

    let parallel_registry = "#![no_std]\nfn probe(_: pci_config::CommandLeases<1>) {}\n";
    require_error(
        &paths,
        "pci-parallel-command-registry",
        parallel_registry,
        "E0603",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("a caller could substitute a parallel command registry:\n{diagnostics}")
    });
    let unchecked_release = "#![no_std]\n\
        fn probe(registry: &mut pci_config::FunctionClaims<1>, authority: pci_config::FunctionAuthority) {\n\
            let _ = registry.release(authority);\n\
        }\n";
    require_error(
        &paths,
        "pci-unchecked-function-release",
        unchecked_release,
        "E0624",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("function authority bypassed its live sublease check:\n{diagnostics}")
    });

    for (label, body, code) in [
        ("memory", "let _ = pci::enable_memory_space(at);", "E0425"),
        ("bus-master", "let _ = pci::enable_bus_master(at);", "E0425"),
        (
            "bus-master-upgrade",
            "let _ = pci::admit_bus_master(at, domain, interrupts, state);",
            "E0308",
        ),
        (
            "restore",
            "let _ = pci::restore_command(at, state);",
            "E0308",
        ),
        ("commit", "let _ = pci::commit_command(at, state);", "E0308"),
        ("probe", "let _ = pci::probe_bars(at);", "E0308"),
        (
            "probe-retry",
            "let _ = pci::retry_probe(at, retry);",
            "E0308",
        ),
        ("release", "let _ = pci::release_function(at);", "E0308"),
    ] {
        let arguments = match label {
            "bus-master-upgrade" => {
                ", domain: wost::arch::current::vtd::DmaDomain, interrupts: wost::arch::current::vtd::InterruptIsolation, state: &mut pci_config::CommandState"
            }
            "restore" | "commit" => ", state: pci_config::CommandState",
            "probe-retry" => ", retry: pci_config::ProbeRetry",
            _ => "",
        };
        let source = format!(
            "#![no_std]\nuse wost::arch::current::pci;\n\
             fn probe(at: pci_config::Address{arguments}) {{ {body} }}\n"
        );
        require_error(&paths, &format!("pci-raw-{label}"), &source, code).unwrap_or_else(
            |diagnostics| panic!("raw PCI address reached {label} mutation:\n{diagnostics}"),
        );
    }

    for (label, input) in [
        ("function", "&pci::FunctionClaim"),
        ("mapped-bar", "&pci::MappedBar"),
    ] {
        let source = format!(
            "#![no_std]\nuse wost::arch::current::pci;\n\
             fn probe(input: {input}, domain: wost::arch::current::vtd::DmaDomain, interrupts: wost::arch::current::vtd::InterruptIsolation, state: &mut pci_config::CommandState) {{\n\
                 let _ = pci::admit_bus_master(input, domain, interrupts, state);\n\
             }}\n"
        );
        require_error(
            &paths,
            &format!("pci-unqualified-bus-master-{label}"),
            &source,
            "E0308",
        )
        .unwrap_or_else(|diagnostics| {
            panic!("{label} authority bypassed fresh NVMe observation:\n{diagnostics}")
        });
    }

    let read_only = "#![no_std]\nuse wost::arch::current::pci;\n\
        fn probe(at: pci_config::Address) { let _ = pci::read(at); let _ = pci::claim_function(at); }\n";
    require_success(&paths, "pci-read-and-claim", read_only).unwrap_or_else(|diagnostics| {
        panic!("read-only enumeration or the sole claim door was lost:\n{diagnostics}")
    });
}

/// Types whose values are PCI function authority (D-335, D-342).
const PCI_AUTHORITY_ROOTS: [&str; 2] = ["FunctionAuthority", "FunctionClaim"];

/// The only doors that may produce authority without consuming any.
const PCI_AUTHORITY_MINTS: [(&str, &str); 2] = [
    ("libs/pci-config/src/claims.rs", "FunctionClaims::claim"),
    ("wost/src/arch/x86_64/pci.rs", "claim_function"),
];

/// Traits none of whose members can manufacture the implementing type.
const PCI_AUTHORITY_TRAITS: [&str; 5] = ["Debug", "Display", "Error", "PartialEq", "Eq"];

const PCI_AUTHORITY_DERIVES: [&str; 3] = ["Debug", "PartialEq", "Eq"];

const UNBOUNDED: u32 = u32::MAX;

/// How a type holds PCI authority: owned copies, a shared view, or a channel
/// the inventory cannot count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Holding {
    owned: u32,
    shared: bool,
    escape: bool,
}

impl Holding {
    const ESCAPE: Self = Self {
        owned: 0,
        shared: false,
        escape: true,
    };

    const fn any(self) -> bool {
        self.owned > 0 || self.shared || self.escape
    }

    const fn both(self, other: Self) -> Self {
        Self {
            owned: self.owned.saturating_add(other.owned),
            shared: self.shared || other.shared,
            escape: self.escape || other.escape,
        }
    }

    fn either(self, other: Self) -> Self {
        Self {
            owned: self.owned.max(other.owned),
            shared: self.shared || other.shared,
            escape: self.escape || other.escape,
        }
    }

    const fn times(self, count: u32) -> Self {
        Self {
            owned: self.owned.saturating_mul(count),
            ..self
        }
    }
}

#[derive(Clone, Copy)]
struct AuthorityTypes<'a> {
    carriers: &'a BTreeMap<String, Holding>,
    self_type: Option<Holding>,
}

impl AuthorityTypes<'_> {
    fn names_ident(self, ident: &proc_macro2::Ident) -> bool {
        self.carriers.contains_key(&ident.to_string())
            || (ident == "Self" && self.self_type.is_some_and(Holding::any))
    }

    fn names_tokens(self, tokens: proc_macro2::TokenStream) -> bool {
        tokens.into_iter().any(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => self.names_ident(&ident),
            proc_macro2::TokenTree::Group(group) => self.names_tokens(group.stream()),
            _ => false,
        })
    }

    fn uncounted(self, visit: impl FnOnce(&mut AuthorityNames<'_>)) -> Holding {
        let mut names = AuthorityNames {
            types: self,
            found: false,
        };
        visit(&mut names);
        if names.found {
            Holding::ESCAPE
        } else {
            Holding::default()
        }
    }

    fn holding(self, ty: &Type) -> Holding {
        match ty {
            Type::Path(path) if path.qself.is_none() => self.path_holding(&path.path),
            Type::Reference(reference) => {
                let inner = self.holding(&reference.elem);
                if reference.mutability.is_some() && (inner.owned > 0 || inner.escape) {
                    Holding::ESCAPE
                } else {
                    Holding {
                        owned: 0,
                        shared: inner.any(),
                        escape: inner.escape,
                    }
                }
            }
            Type::Tuple(tuple) => tuple
                .elems
                .iter()
                .fold(Holding::default(), |held, element| {
                    held.both(self.holding(element))
                }),
            Type::Array(array) => {
                let length = match &array.len {
                    Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(length),
                        ..
                    }) => length.base10_parse::<u32>().unwrap_or(UNBOUNDED),
                    _ => UNBOUNDED,
                };
                self.holding(&array.elem).times(length)
            }
            Type::Slice(slice) => self.holding(&slice.elem).times(UNBOUNDED),
            Type::Paren(inner) => self.holding(&inner.elem),
            Type::Group(inner) => self.holding(&inner.elem),
            other => self.uncounted(|names| names.visit_type(other)),
        }
    }

    fn path_holding(self, path: &syn::Path) -> Holding {
        let Some(last) = path.segments.last() else {
            return Holding::default();
        };
        let mut held = if path.segments.len() == 1 && last.ident == "Self" {
            self.self_type.unwrap_or_default()
        } else {
            self.carriers
                .get(&last.ident.to_string())
                .copied()
                .unwrap_or_default()
        };
        for earlier in path.segments.iter().take(path.segments.len() - 1) {
            held =
                held.both(self.uncounted(|names| names.visit_path_arguments(&earlier.arguments)));
        }
        match &last.arguments {
            PathArguments::None => held,
            PathArguments::AngleBracketed(arguments) => {
                let alternatives = last.ident == "Option" || last.ident == "Result";
                let mut inner = Holding::default();
                for argument in &arguments.args {
                    let argument = match argument {
                        GenericArgument::Type(ty) => self.holding(ty),
                        other => self.uncounted(|names| names.visit_generic_argument(other)),
                    };
                    inner = if alternatives {
                        inner.either(argument)
                    } else {
                        inner.both(argument)
                    };
                }
                held.both(if alternatives {
                    inner
                } else {
                    inner.times(UNBOUNDED)
                })
            }
            PathArguments::Parenthesized(_) => {
                held.both(self.uncounted(|names| names.visit_path_arguments(&last.arguments)))
            }
        }
    }
}

struct AuthorityNames<'a> {
    types: AuthorityTypes<'a>,
    found: bool,
}

impl<'ast> Visit<'ast> for AuthorityNames<'_> {
    fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
        self.found |= self.types.names_ident(ident);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.found |= self.types.names_tokens(mac.tokens.clone());
        visit::visit_macro(self, mac);
    }
}

fn authority_fields<'f>(
    types: AuthorityTypes<'_>,
    fields: impl IntoIterator<Item = &'f syn::Field>,
) -> Holding {
    fields.into_iter().fold(Holding::default(), |held, field| {
        held.both(types.holding(&field.ty))
    })
}

fn grow_pci_carriers(
    items: &[Item],
    carriers: &BTreeMap<String, Holding>,
    grown: &mut BTreeMap<String, Holding>,
) {
    let types = AuthorityTypes {
        carriers,
        self_type: None,
    };
    for item in items {
        if cfg_test(observer_item_attributes(item)) {
            continue;
        }
        let (name, held) = match item {
            Item::Struct(item) => (&item.ident, authority_fields(types, &item.fields)),
            Item::Union(item) => (&item.ident, authority_fields(types, &item.fields.named)),
            Item::Enum(item) => (
                &item.ident,
                item.variants
                    .iter()
                    .fold(Holding::default(), |held, variant| {
                        held.either(authority_fields(types, &variant.fields))
                    }),
            ),
            Item::Type(item) => (&item.ident, types.holding(&item.ty)),
            Item::Use(item) => {
                grow_pci_renames(&item.tree, carriers, grown);
                continue;
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    grow_pci_carriers(items, carriers, grown);
                }
                continue;
            }
            _ => continue,
        };
        if held.any() {
            let entry = grown.entry(name.to_string()).or_default();
            *entry = entry.either(held);
        }
    }
}

fn grow_pci_renames(
    tree: &syn::UseTree,
    carriers: &BTreeMap<String, Holding>,
    grown: &mut BTreeMap<String, Holding>,
) {
    match tree {
        syn::UseTree::Path(path) => grow_pci_renames(&path.tree, carriers, grown),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                grow_pci_renames(tree, carriers, grown);
            }
        }
        syn::UseTree::Rename(rename) => {
            if let Some(held) = carriers.get(&rename.ident.to_string()) {
                let entry = grown.entry(rename.rename.to_string()).or_default();
                *entry = entry.either(*held);
            }
        }
        syn::UseTree::Name(_) | syn::UseTree::Glob(_) => {}
    }
}

fn copies(count: u32) -> String {
    if count == UNBOUNDED {
        "unboundedly many".to_string()
    } else {
        count.to_string()
    }
}

struct AuthorityInventory<'a> {
    file: &'a str,
    carriers: &'a BTreeMap<String, Holding>,
    names_authority: bool,
    self_type: Option<Holding>,
    owner: Option<String>,
    violations: Vec<String>,
    mints: BTreeMap<String, usize>,
}

impl<'a> AuthorityInventory<'a> {
    const fn types(&self) -> AuthorityTypes<'a> {
        AuthorityTypes {
            carriers: self.carriers,
            self_type: self.self_type,
        }
    }

    fn refuse(&mut self, reason: &str) {
        self.violations.push(format!("{}: {reason}", self.file));
    }

    fn require_fields<'f>(
        &mut self,
        owner: &syn::Ident,
        fields: impl IntoIterator<Item = &'f syn::Field>,
    ) {
        let types = AuthorityTypes {
            carriers: self.carriers,
            self_type: None,
        };
        if fields
            .into_iter()
            .any(|field| types.holding(&field.ty).escape)
        {
            self.refuse(&format!(
                "a field of `{owner}` reaches PCI authority through a type the inventory cannot count"
            ));
        }
    }

    fn require_derives(&mut self, owner: &syn::Ident, attributes: &[syn::Attribute]) {
        if !self
            .carriers
            .get(&owner.to_string())
            .is_some_and(|held| held.owned > 0)
        {
            return;
        }
        for attribute in attributes
            .iter()
            .filter(|attribute| attribute.path().is_ident("derive"))
        {
            let derived = attribute
                .parse_args_with(
                    syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
                )
                .unwrap_or_else(|error| panic!("parsing the derives of `{owner}`: {error}"));
            for path in derived {
                let name = path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                    .unwrap_or_default();
                if !PCI_AUTHORITY_DERIVES.contains(&name.as_str()) {
                    self.refuse(&format!("PCI authority `{owner}` derives `{name}`"));
                }
            }
        }
    }

    fn require_signature(&mut self, signature: &syn::Signature) {
        let name = match &self.owner {
            Some(owner) => format!("{owner}::{}", signature.ident),
            None => signature.ident.to_string(),
        };
        let types = self.types();
        let mut consumed = 0_u32;
        let mut uncounted = false;
        for input in &signature.inputs {
            let held = match input {
                syn::FnArg::Receiver(receiver) => types.holding(&receiver.ty),
                syn::FnArg::Typed(argument) => types.holding(&argument.ty),
            };
            uncounted |= held.escape;
            consumed = consumed.saturating_add(held.owned);
        }
        let produced = match &signature.output {
            syn::ReturnType::Default => Holding::default(),
            syn::ReturnType::Type(_, output) => types.holding(output),
        };
        if uncounted {
            self.refuse(&format!(
                "`{name}` takes PCI authority through a parameter the inventory cannot count"
            ));
        }
        if produced.escape {
            self.refuse(&format!(
                "`{name}` returns PCI authority through a type the inventory cannot count"
            ));
        }
        if produced.owned > consumed {
            let audited = PCI_AUTHORITY_MINTS
                .iter()
                .any(|(file, mint)| *file == self.file && *mint == name);
            if audited && consumed == 0 && produced.owned == 1 {
                *self.mints.entry(name).or_default() += 1;
            } else {
                self.refuse(&format!(
                    "`{name}` returns {} PCI authority copies while consuming {}",
                    copies(produced.owned),
                    copies(consumed)
                ));
            }
        }
    }

    fn require_opaque_macro(&mut self) {
        if self.names_authority {
            self.refuse("an item macro can expand to a door the inventory cannot see");
        }
    }
}

impl<'ast> Visit<'ast> for AuthorityInventory<'_> {
    fn visit_item(&mut self, item: &'ast Item) {
        if !cfg_test(observer_item_attributes(item)) {
            visit::visit_item(self, item);
        }
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        if !cfg_test(observer_impl_item_attributes(item)) {
            visit::visit_impl_item(self, item);
        }
    }

    fn visit_item_struct(&mut self, item: &'ast ItemStruct) {
        self.require_derives(&item.ident, &item.attrs);
        self.require_fields(&item.ident, &item.fields);
        visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast ItemEnum) {
        self.require_derives(&item.ident, &item.attrs);
        for variant in &item.variants {
            self.require_fields(&item.ident, &variant.fields);
        }
        visit::visit_item_enum(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.require_derives(&item.ident, &item.attrs);
        self.require_fields(&item.ident, &item.fields.named);
        visit::visit_item_union(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("static `{}` stores PCI authority", item.ident));
        }
        visit::visit_item_static(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("constant `{}` stores PCI authority", item.ident));
        }
        visit::visit_item_const(self, item);
    }

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!("constant `{}` stores PCI authority", item.ident));
        }
        visit::visit_impl_item_const(self, item);
    }

    fn visit_impl_item_type(&mut self, item: &'ast syn::ImplItemType) {
        if self.types().holding(&item.ty).any() {
            self.refuse(&format!(
                "associated type `{}` names PCI authority",
                item.ident
            ));
        }
        visit::visit_impl_item_type(self, item);
    }

    fn visit_generics(&mut self, generics: &'ast syn::Generics) {
        if self
            .types()
            .uncounted(|names| names.visit_generics(generics))
            .escape
        {
            self.refuse("a generic parameter or bound names PCI authority");
        }
        visit::visit_generics(self, generics);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if self.types().names_tokens(mac.tokens.clone()) {
            self.refuse("a macro receives PCI authority names the inventory cannot follow");
        }
        visit::visit_macro(self, mac);
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        self.require_opaque_macro();
        visit::visit_item_macro(self, item);
    }

    fn visit_impl_item_macro(&mut self, item: &'ast syn::ImplItemMacro) {
        self.require_opaque_macro();
        visit::visit_impl_item_macro(self, item);
    }

    fn visit_trait_item_macro(&mut self, item: &'ast syn::TraitItemMacro) {
        self.require_opaque_macro();
        visit::visit_trait_item_macro(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let held = AuthorityTypes {
            carriers: self.carriers,
            self_type: None,
        }
        .holding(&item.self_ty);
        let owner = match item.self_ty.as_ref() {
            Type::Path(path) => path
                .path
                .segments
                .last()
                .map(|segment| segment.ident.to_string()),
            _ => None,
        };
        if let Some((_, implemented, _)) = &item.trait_
            && held.owned > 0
        {
            let implemented = implemented
                .segments
                .last()
                .map(|segment| segment.ident.to_string())
                .unwrap_or_default();
            if !PCI_AUTHORITY_TRAITS.contains(&implemented.as_str()) {
                self.refuse(&format!(
                    "PCI authority `{}` implements `{implemented}`",
                    owner.as_deref().unwrap_or("?")
                ));
            }
        }
        let self_type = self.self_type.replace(held);
        let previous = core::mem::replace(&mut self.owner, owner);
        visit::visit_item_impl(self, item);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        let self_type = self.self_type.take();
        let previous = self.owner.replace(item.ident.to_string());
        visit::visit_item_trait(self, item);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        let self_type = self.self_type.take();
        let previous = self.owner.take();
        self.require_signature(&function.sig);
        visit::visit_item_fn(self, function);
        self.self_type = self_type;
        self.owner = previous;
    }

    fn visit_impl_item_fn(&mut self, method: &'ast ImplItemFn) {
        self.require_signature(&method.sig);
        visit::visit_impl_item_fn(self, method);
    }

    fn visit_trait_item_fn(&mut self, method: &'ast syn::TraitItemFn) {
        self.require_signature(&method.sig);
        visit::visit_trait_item_fn(self, method);
    }
}

fn pci_authority_violations(sources: &[(&str, &str)]) -> Vec<String> {
    let files = sources
        .iter()
        .map(|(relative, text)| {
            syn::parse_file(text).unwrap_or_else(|error| panic!("parsing {relative}: {error}"))
        })
        .collect::<Vec<_>>();
    let mut carriers = PCI_AUTHORITY_ROOTS
        .iter()
        .map(|root| {
            (
                (*root).to_string(),
                Holding {
                    owned: 1,
                    ..Holding::default()
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    loop {
        let mut grown = carriers.clone();
        for syntax in &files {
            grow_pci_carriers(&syntax.items, &carriers, &mut grown);
        }
        if grown == carriers {
            break;
        }
        carriers = grown;
    }

    let mut violations = Vec::new();
    let mut mints = BTreeMap::<(String, String), usize>::new();
    for (&(relative, text), syntax) in sources.iter().zip(&files) {
        let tokens = text
            .parse::<proc_macro2::TokenStream>()
            .unwrap_or_else(|error| panic!("tokenizing {relative}: {error}"));
        let mut inventory = AuthorityInventory {
            file: relative,
            carriers: &carriers,
            names_authority: AuthorityTypes {
                carriers: &carriers,
                self_type: None,
            }
            .names_tokens(tokens),
            self_type: None,
            owner: None,
            violations: Vec::new(),
            mints: BTreeMap::new(),
        };
        inventory.visit_file(syntax);
        violations.append(&mut inventory.violations);
        for (mint, seen) in inventory.mints {
            *mints.entry((relative.to_string(), mint)).or_default() += seen;
        }
    }
    for (file, mint) in PCI_AUTHORITY_MINTS {
        let seen = mints
            .get(&(file.to_string(), mint.to_string()))
            .copied()
            .unwrap_or(0);
        if seen != 1 {
            violations.push(format!(
                "{file}: audited mint `{mint}` was seen {seen} times"
            ));
        }
    }
    violations
}

#[test]
fn pci_function_authority_has_audited_mints_and_no_duplication_door() {
    let paths = Paths::discover().expect("workspace paths");
    let mut sources = Vec::new();
    for directory in ["libs/pci-config/src", "wost/src"] {
        for path in production_sources_below(&paths.root().join(directory)) {
            let relative = path
                .strip_prefix(paths.root())
                .expect("source below the workspace")
                .to_string_lossy()
                .replace('\\', "/");
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("reading {relative}: {error}"));
            sources.push((relative, text));
        }
    }
    let sources = sources
        .iter()
        .map(|(relative, text)| (relative.as_str(), text.as_str()))
        .collect::<Vec<_>>();
    let violations = pci_authority_violations(&sources);
    assert!(
        violations.is_empty(),
        "PCI function authority has an uncounted door:\n{}",
        violations.join("\n")
    );
}

#[test]
fn the_pci_authority_inventory_counts_every_door_and_channel() {
    const PURE: &str = "libs/pci-config/src/claims.rs";
    const HARDWARE: &str = "wost/src/arch/x86_64/pci.rs";
    let pure = "#[derive(Debug, PartialEq, Eq)]\n\
        pub struct FunctionAuthority { lease: u64 }\n\
        pub struct FunctionClaims;\n\
        impl FunctionClaims {\n\
            pub fn claim(&mut self) -> Result<FunctionAuthority, ()> { Err(()) }\n\
        }\n\
        pub struct FunctionReleaseFailure { authority: FunctionAuthority }\n\
        impl FunctionReleaseFailure {\n\
            pub fn into_authority(self) -> FunctionAuthority { self.authority }\n\
        }\n\
        impl core::fmt::Display for FunctionReleaseFailure {\n\
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result { Ok(()) }\n\
        }\n\
        #[cfg(test)]\n\
        mod tests {\n\
            fn authority() -> super::FunctionAuthority { loop {} }\n\
        }\n";
    let hardware = "pub struct FunctionClaim { authority: FunctionAuthority }\n\
        impl FunctionClaim { pub const fn address(&self) -> u32 { 0 } }\n\
        pub struct ReleaseFailure { function: FunctionClaim }\n\
        pub fn claim_function() -> Result<FunctionClaim, ()> { Err(()) }\n\
        pub fn release_function(function: FunctionClaim) -> Result<(), ReleaseFailure> { Ok(()) }\n\
        pub fn probe_bars(function: &FunctionClaim) -> Option<&FunctionClaim> { Some(function) }\n";
    let check = |pure: &str, hardware: &str| {
        pci_authority_violations(&[(PURE, pure), (HARDWARE, hardware)])
    };
    assert_eq!(check(pure, hardware), Vec::<String>::new());

    for (label, pure_extra, hardware_extra, expected) in [
        (
            "renamed",
            "",
            "impl FunctionClaim { pub fn fork(&self) -> Self { loop {} } }\n",
            "`FunctionClaim::fork` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "split",
            "impl FunctionAuthority { pub fn split(self) -> (Self, Self) { loop {} } }\n",
            "",
            "`FunctionAuthority::split` returns 2 PCI authority copies while consuming 1",
        ),
        (
            "array",
            "",
            "pub fn pair(a: FunctionClaim) -> [FunctionClaim; 2] { loop {} }\n",
            "`pair` returns 2 PCI authority copies while consuming 1",
        ),
        (
            "container",
            "",
            "pub fn many(a: FunctionClaim) -> Vec<FunctionClaim> { loop {} }\n",
            "`many` returns unboundedly many PCI authority copies while consuming 1",
        ),
        (
            "alias",
            "pub type Token = FunctionAuthority;\npub fn token() -> Token { loop {} }\n",
            "",
            "`token` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "rename",
            "",
            "use crate::FunctionClaim as Lease;\npub fn lease() -> Lease { loop {} }\n",
            "`lease` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "wrapper",
            "pub struct Spare(FunctionReleaseFailure);\npub fn spare() -> Option<Spare> { None }\n",
            "",
            "`spare` returns 1 PCI authority copies while consuming 0",
        ),
        (
            "mutable parameter",
            "",
            "pub fn overwrite(slot: &mut Option<FunctionClaim>) {}\n",
            "`overwrite` takes PCI authority through a parameter the inventory cannot count",
        ),
        (
            "mutable receiver",
            "impl FunctionAuthority { pub fn retarget(&mut self) {} }\n",
            "",
            "`FunctionAuthority::retarget` takes PCI authority through a parameter",
        ),
        (
            "callback",
            "",
            "pub fn lend(sink: impl FnOnce(FunctionClaim)) {}\n",
            "`lend` takes PCI authority through a parameter the inventory cannot count",
        ),
        (
            "pointer",
            "",
            "pub fn raw() -> *const FunctionClaim { loop {} }\n",
            "`raw` returns PCI authority through a type the inventory cannot count",
        ),
        (
            "static",
            "",
            "static STASH: Option<FunctionClaim> = None;\n",
            "static `STASH` stores PCI authority",
        ),
        (
            "field",
            "",
            "pub struct Handle<'a> { claim: &'a mut FunctionClaim }\n",
            "a field of `Handle` reaches PCI authority through a type the inventory cannot count",
        ),
        (
            "trait default",
            "",
            "pub trait Twin: Sized { fn twin(&self) -> Self { loop {} } }\nimpl Twin for FunctionClaim {}\n",
            "PCI authority `FunctionClaim` implements `Twin`",
        ),
        (
            "derive",
            "#[derive(Debug, Clone)]\npub struct Spare { authority: FunctionAuthority }\n",
            "",
            "PCI authority `Spare` derives `Clone`",
        ),
        (
            "associated type",
            "",
            "pub struct Minter;\nimpl Iterator for Minter {\n    type Item = FunctionClaim;\n    fn next(&mut self) -> Option<Self::Item> { None }\n}\n",
            "associated type `Item` names PCI authority",
        ),
        (
            "bound",
            "",
            "pub fn with<F: FnOnce(FunctionClaim)>(sink: F) {}\n",
            "a generic parameter or bound names PCI authority",
        ),
        (
            "macro argument",
            "",
            "pub fn named() { let _ = core::mem::size_of::<u8>(); door!(FunctionClaim); }\n",
            "a macro receives PCI authority names the inventory cannot follow",
        ),
        (
            "item macro",
            "",
            "macro_rules! door { () => {}; }\n",
            "an item macro can expand to a door the inventory cannot see",
        ),
        (
            "configured",
            "",
            "#[cfg(feature = \"selftest\")]\npub fn probe_claim() -> FunctionClaim { loop {} }\n",
            "`probe_claim` returns 1 PCI authority copies while consuming 0",
        ),
    ] {
        let violations = check(
            &format!("{pure}{pure_extra}"),
            &format!("{hardware}{hardware_extra}"),
        );
        assert!(
            violations
                .iter()
                .any(|violation| violation.contains(expected)),
            "the {label} door was not refused with `{expected}`: {violations:?}"
        );
    }

    let unaudited = hardware.replace("claim_function()", "claim_unaudited()");
    let violations = check(pure, &unaudited);
    assert!(
        violations.contains(&format!(
            "{HARDWARE}: audited mint `claim_function` was seen 0 times"
        )),
        "a missing audited mint was not reported: {violations:?}"
    );
    assert!(
        violations.iter().any(|violation| violation
            .contains("`claim_unaudited` returns 1 PCI authority copies while consuming 0")),
        "an unaudited mint was not refused: {violations:?}"
    );
}

#[derive(Debug, PartialEq, Eq)]
enum PciBarAdmissionEvent {
    Admit,
    Map,
    ClaimDisabled,
    EnableDecode,
}

#[derive(Default)]
struct PciBarAdmissionShape {
    events: Vec<PciBarAdmissionEvent>,
    raw_device_maps: usize,
}

impl<'ast> Visit<'ast> for PciBarAdmissionShape {
    fn visit_expr_call(&mut self, expression: &'ast ExprCall) {
        if let Expr::Path(function) = expression.func.as_ref()
            && let Some(name) = function.path.segments.last().map(|segment| &segment.ident)
        {
            if name == "map_pci_bar" {
                self.events.push(PciBarAdmissionEvent::Map);
            } else if name == "map_device" {
                self.raw_device_maps += 1;
            }
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let event = match expression.method.to_string().as_str() {
            "admit_memory_bar" => Some(PciBarAdmissionEvent::Admit),
            "map_pci_bar" => Some(PciBarAdmissionEvent::Map),
            "claim_disabled" => Some(PciBarAdmissionEvent::ClaimDisabled),
            "enable_memory_bar" => Some(PciBarAdmissionEvent::EnableDecode),
            "map_device" => {
                self.raw_device_maps += 1;
                None
            }
            _ => None,
        };
        if let Some(event) = event {
            self.events.push(event);
        }
        visit::visit_expr_method_call(self, expression);
    }
}

#[test]
fn pci_bar_mapping_requires_exact_linear_authority() {
    let paths = Paths::discover().expect("workspace paths");
    for authority in [
        "pci_config::BarAuthority",
        "wost::arch::current::pci::AdmittedBar",
        "wost::arch::current::pci::BarMapping",
        "wost::arch::current::pci::DisabledBar",
        "wost::arch::current::pci::MappedBar",
        "wost::arch::current::pci::ReleasableBar",
    ] {
        for bound in ["Copy", "Clone"] {
            let source = format!(
                "#![no_std]\nfn requires<T: {bound}>() {{}}\n\
                 fn probe() {{ requires::<{authority}>(); }}\n"
            );
            require_error(&paths, "pci-bar-linear", &source, "E0277").unwrap_or_else(
                |diagnostics| {
                    panic!("PCI BAR authority {authority} satisfied {bound}:\n{diagnostics}")
                },
            );
        }
    }

    let exact_mapping = "#![no_std]\n\
        use wost::arch::current::{paging, pci};\n\
        fn probe(space: &mut paging::AddressSpace, bar: pci::AdmittedBar)\n\
            -> Result<pci::BarMapping, paging::BarMapFailure>\n\
        { space.map_pci_bar(bar) }\n";
    require_success(&paths, "pci-bar-exact-mapping", exact_mapping).unwrap_or_else(|diagnostics| {
        panic!("the exact PCI BAR mapping door was lost:\n{diagnostics}")
    });

    let raw_mapping = "#![no_std]\n\
        use wost::arch::current::paging::{AddressSpace, AdmittedDeviceRange};\n\
        fn probe(space: &mut AddressSpace, range: AdmittedDeviceRange) {\n\
            let _ = space.map_pci_bar(range);\n\
        }\n";
    require_error(&paths, "pci-bar-raw-mapping", raw_mapping, "E0308").unwrap_or_else(
        |diagnostics| {
            panic!("a function-free physical range reached PCI BAR mapping:\n{diagnostics}")
        },
    );

    let replayed_mapping = "#![no_std]\n\
        use wost::arch::current::{paging::AddressSpace, pci::AdmittedBar};\n\
        fn probe(space: &mut AddressSpace, bar: AdmittedBar) {\n\
            let _ = space.map_pci_bar(bar);\n\
            let _ = space.map_pci_bar(bar);\n\
        }\n";
    require_error(
        &paths,
        "pci-bar-replayed-mapping",
        replayed_mapping,
        "E0382",
    )
    .unwrap_or_else(|diagnostics| {
        panic!("one PCI BAR admission mapped more than once:\n{diagnostics}")
    });

    let main_source = std::fs::read_to_string(paths.root().join("kernel/src/main.rs"))
        .expect("read kernel main source");
    let main_syntax = syn::parse_file(&main_source).expect("parse kernel main source");
    let admission = main_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "admit_controller_window" => Some(function),
            _ => None,
        })
        .expect("kernel main has no PCI BAR admission path");
    let mut shape = PciBarAdmissionShape::default();
    Visit::visit_item_fn(&mut shape, admission);
    assert_eq!(
        shape.events,
        [
            PciBarAdmissionEvent::Admit,
            PciBarAdmissionEvent::Map,
            PciBarAdmissionEvent::ClaimDisabled,
            PciBarAdmissionEvent::EnableDecode,
        ],
        "PCI memory decode no longer follows exact range admission and mapping"
    );
    assert_eq!(
        shape.raw_device_maps, 0,
        "PCI BAR admission used a raw mapping door"
    );

    let nvme_syntax = super::module_syntax(&paths.root().join("kernel/src/nvme.rs"));
    let prepare = nvme_syntax
        .items
        .iter()
        .find_map(|item| match item {
            Item::Fn(function) if function.sig.ident == "prepare" => Some(function),
            _ => None,
        })
        .expect("NVMe source has no prepare function");
    let inputs = &prepare.sig.inputs;
    assert!(!inputs.is_empty(), "NVMe prepare has no context");
    assert!(
        inputs.len() >= 2,
        "NVMe prepare has no mapped register authority"
    );
    let syn::FnArg::Typed(registers) = &inputs[1] else {
        panic!("NVMe prepare has no mapped register authority");
    };
    assert!(
        matches!(registers.ty.as_ref(), Type::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == "MappedBar")),
        "NVMe prepare accepts registers without exact mapped BAR authority"
    );
    assert!(
        inputs.len() == 2,
        "NVMe prepare regained a raw BAR argument"
    );

    let mut nvme_shape = PciBarAdmissionShape::default();
    Visit::visit_file(&mut nvme_shape, &nvme_syntax);
    assert_eq!(
        nvme_shape.raw_device_maps, 0,
        "NVMe regained the generic device mapping door"
    );
}

#[test]
fn pci_command_transactions_hold_the_port_pair_once() {
    let paths = Paths::discover().expect("workspace paths");
    let source = std::fs::read_to_string(paths.root().join("wost/src/arch/x86_64/pci.rs"))
        .expect("PCI source");
    for (start, algorithm) in [
        (
            "pub fn admit_bus_master",
            "pci_config::admit_mapped_bus_master",
        ),
        ("pub fn restore_command", "pci_config::restore_command"),
        ("pub fn commit_command", "pci_config::commit_command"),
        ("pub fn probe_bars", "pci_config::probe_bars"),
        ("pub fn retry_probe", "pci_config::retry_probe"),
    ] {
        let transaction = source
            .split_once(start)
            .unwrap_or_else(|| panic!("missing {start}"))
            .1
            .split_once("\n}\n")
            .expect("complete PCI transaction")
            .0;
        assert_eq!(
            transaction.matches("CONFIGURATION.lock_leaf()").count(),
            1,
            "{start} does not own one continuous port interval"
        );
        assert!(
            transaction.contains(algorithm),
            "{start} bypasses the verified pure transaction"
        );
    }
}
