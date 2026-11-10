use super::*;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum TraitRoute {
    Declaration(TraitMemberKey),
    Implementation {
        key: TraitMemberKey,
        self_type: Option<String>,
        exact_self_type: bool,
        external: bool,
        implicit: Option<ImplicitTrait>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ImplicitTrait {
    Drop,
    GlobalAlloc,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct TraitMemberKey {
    pub(super) trait_path: String,
    pub(super) member: String,
}

#[derive(Clone, Debug)]
pub(super) struct TraitActivation {
    pub(super) self_type: usize,
    pub(super) exact_self_type: bool,
    pub(super) declaration: Option<usize>,
    pub(super) key: TraitMemberKey,
    pub(super) implementation: usize,
}

#[derive(Clone, Debug)]
pub(super) struct ImplicitTraitActivation {
    pub(super) trigger: ImplicitTraitTrigger,
    pub(super) implementation: usize,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum ImplicitTraitTrigger {
    ConstructedType(usize),
    LiveNode(usize),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ImportOrigin {
    Standard,
    Dependency,
    Other,
}

#[derive(Default, Debug)]
pub(super) struct TraitImports {
    paths: BTreeMap<Vec<String>, BTreeMap<String, Option<Vec<String>>>>,
    local_names: BTreeMap<Vec<String>, BTreeSet<String>>,
    external_names: BTreeMap<Vec<String>, BTreeMap<String, ImportOrigin>>,
    globs: BTreeMap<Vec<String>, Vec<Vec<String>>>,
}

pub(super) fn trait_activations(
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
                    || workspace_type_matches(&files[candidate.file], &candidate.symbol, name))
        })
        .map(|candidate| candidate.id)
        .collect()
}

impl TraitActivation {
    pub(super) fn call_compatible(&self, site: &DispatchSite, nodes: &BTreeSet<usize>) -> bool {
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

    pub(super) fn matches(
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

pub(super) fn trait_member_matches(
    declaration: &TraitMemberKey,
    implementation: &TraitMemberKey,
) -> bool {
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

impl TraitImports {
    pub(super) fn collect_in_scope(items: &[Item], scope: &[String]) -> Self {
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

    pub(super) fn resolve_path(&self, scope: &[String], path: &str) -> Option<String> {
        let segments = path.split("::").map(str::to_string).collect::<Vec<_>>();
        self.resolve_segments(scope, &segments, &mut BTreeSet::new())
            .map(|segments| segments.join("::"))
    }

    pub(super) fn external_trait(&self, scope: &[String], path: &str) -> bool {
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

    pub(super) fn implicit_standard_trait(
        &self,
        scope: &[String],
        path: &str,
    ) -> Option<ImplicitTrait> {
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
