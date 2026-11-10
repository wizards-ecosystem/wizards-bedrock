use super::*;

#[derive(Default, Debug)]
pub(super) struct Uses {
    pub(super) roots: BTreeSet<usize>,
    pub(super) edges: BTreeMap<usize, BTreeSet<usize>>,
    dispatches: BTreeMap<(usize, usize), DispatchEvidence>,
    pub(super) constructions: BTreeMap<usize, BTreeSet<usize>>,
}

#[derive(Debug)]
pub(super) struct DispatchEvidence {
    pub(super) caller: usize,
    pub(super) nodes: BTreeSet<usize>,
    pub(super) callee_nodes: BTreeSet<usize>,
    pub(super) self_nodes: BTreeSet<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DispatchResolution {
    NoCredit,
    DeclarationOnly,
    Implementation(usize),
    Failure(usize),
}

impl Uses {
    pub(super) fn seed_dispatches(&mut self, inventory: &Inventory) -> Result<()> {
        for (file, source) in inventory.files.iter().enumerate() {
            for (site, dispatch) in source.dispatch_sites.iter().enumerate() {
                let caller = inventory
                    .caller(file, dispatch.range.start)
                    .with_context(|| {
                        format!(
                            "dispatch at {}:{} has no classified caller",
                            source.relative.display(),
                            dispatch.range.start.line
                        )
                    })?;
                let evidence =
                    self.dispatches
                        .entry((file, site))
                        .or_insert_with(|| DispatchEvidence {
                            caller,
                            nodes: BTreeSet::new(),
                            callee_nodes: BTreeSet::new(),
                            self_nodes: BTreeSet::new(),
                        });
                ensure!(
                    evidence.caller == caller,
                    "one dispatch site was attributed to two lexical callers"
                );
            }
        }
        Ok(())
    }

    pub(super) fn observe_cargo(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        diagnostics: &[MarkerDiagnostic],
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        self.seed_dispatches(inventory)?;
        for diagnostic in diagnostics {
            validate_marker_round(diagnostic.id, inventory, instrumentation)?;
            let Some(file) = inventory.file_index(root, &diagnostic.reported) else {
                let expansion = diagnostic.immediate_expansion.as_ref().with_context(|| {
                    format!(
                        "an API diagnostic for item {} came from unclassified source `{}` without an immediate expansion callsite",
                        diagnostic.id, diagnostic.reported
                    )
                })?;
                ensure!(
                    expansion.terminal,
                    "an API diagnostic for item {} has an external primary span with a nested expansion chain",
                    diagnostic.id
                );
                let expansion_file = inventory
                    .file_index(root, &expansion.location.reported)
                    .with_context(|| {
                        format!(
                            "an API diagnostic for item {} came from unclassified source `{}` and unclassified immediate expansion source `{}`",
                            diagnostic.id, diagnostic.reported, expansion.location.reported
                        )
                    })?;
                validate_cargo_diagnostic_origin(
                    diagnostic,
                    inventory,
                    root,
                    expansion_file,
                    &expansion.location.reported,
                )?;
                let at = corrected_marker_location(
                    expansion_file,
                    expansion.location.line,
                    expansion.location.column,
                    instrumentation,
                )?;
                ensure!(
                    inventory.is_opaque_macro_use(expansion_file, at),
                    "an API diagnostic for item {} has an external primary span whose immediate expansion callsite at {}:{} is not an opaque macro invocation",
                    diagnostic.id,
                    inventory.files[expansion_file].relative.display(),
                    expansion.location.line
                );
                continue;
            };
            let primary_at = corrected_marker_location(
                file,
                diagnostic.line,
                diagnostic.column,
                instrumentation,
            )?;
            let entry_caller =
                if cargo_diagnostic_origin_matches(diagnostic, inventory, root, file)? {
                    None
                } else {
                    let authorized =
                        diagnostic
                            .immediate_expansion
                            .as_ref()
                            .and_then(|expansion| {
                                let expansion_file =
                                    inventory.file_index(root, &expansion.location.reported)?;
                                let expansion_at = corrected_marker_location(
                                    expansion_file,
                                    expansion.location.line,
                                    expansion.location.column,
                                    instrumentation,
                                )
                                .ok()?;
                                inventory
                                    .exact_entry_contract_expansion_caller(
                                        file,
                                        primary_at,
                                        diagnostic.id,
                                        expansion_file,
                                        expansion_at,
                                        expansion,
                                    )
                                    .map(|caller| {
                                        (
                                            caller,
                                            expansion_file,
                                            expansion.location.reported.as_str(),
                                        )
                                    })
                            });
                    let Some((caller, expansion_file, expansion_source)) = authorized else {
                        validate_cargo_diagnostic_origin(
                            diagnostic,
                            inventory,
                            root,
                            file,
                            &diagnostic.reported,
                        )?;
                        bail!("API diagnostic origin changed while its entry route was classified");
                    };
                    validate_cargo_diagnostic_origin(
                        diagnostic,
                        inventory,
                        root,
                        expansion_file,
                        expansion_source,
                    )?;
                    Some(caller)
                };
            self.observe_marker(
                kind,
                inventory,
                MarkerObservation {
                    file,
                    id: diagnostic.id,
                    source_line: diagnostic.line,
                    column: diagnostic.column,
                },
                instrumentation,
                entry_caller,
            )?;
        }
        Ok(())
    }

    pub(super) fn observe_marker(
        &mut self,
        kind: RootKind,
        inventory: &Inventory,
        observation: MarkerObservation,
        instrumentation: &Instrumentation,
        entry_caller: Option<usize>,
    ) -> Result<()> {
        let MarkerObservation {
            file,
            id,
            source_line,
            column,
        } = observation;
        validate_marker_round(id, inventory, instrumentation)?;
        let at = corrected_marker_location(file, source_line, column, instrumentation)?;
        if inventory.is_non_call_use(file, at) {
            return Ok(());
        }
        let entry_caller =
            entry_caller.or_else(|| inventory.exact_entry_invocation_caller(file, at, id));
        let direct_macro_call = inventory.is_direct_macro_call(file, at, id);
        if entry_caller.is_none() && !direct_macro_call && inventory.is_opaque_macro_use(file, at) {
            return Ok(());
        }
        let caller = entry_caller
            .or_else(|| inventory.caller(file, at))
            .with_context(|| {
                format!(
                    "an API diagnostic for {} at {}:{} has no classified caller",
                    inventory.nodes[id].label,
                    inventory.files[file].relative.display(),
                    source_line
                )
            })?;
        if inventory.is_value_construction(file, at, id) {
            self.constructions.entry(caller).or_default().insert(id);
        }
        if let Some(site) = inventory.dispatch_site(file, at) {
            let evidence =
                self.dispatches
                    .entry((file, site))
                    .or_insert_with(|| DispatchEvidence {
                        caller,
                        nodes: BTreeSet::new(),
                        callee_nodes: BTreeSet::new(),
                        self_nodes: BTreeSet::new(),
                    });
            ensure!(
                evidence.caller == caller,
                "one dispatch site was attributed to two lexical callers"
            );
            evidence.nodes.insert(id);
            let site = &inventory.files[file].dispatch_sites[site];
            if site.callee_evidence.iter().any(|range| range.contains(at)) {
                evidence.callee_nodes.insert(id);
            }
            if site.self_evidence.iter().any(|range| range.contains(at)) {
                evidence.self_nodes.insert(id);
            }
        }
        ensure!(
            !inventory.unresolved_trait_implementations.contains(&caller),
            "an API diagnostic for {} originates in unresolved trait implementation {}",
            inventory.nodes[id].label,
            inventory.nodes[caller].label
        );
        if caller != id {
            self.edges.entry(caller).or_default().insert(id);
        }
        if inventory.nodes[caller].roots.contains(kind) {
            self.roots.insert(caller);
        }
        Ok(())
    }

    pub(super) fn declaration_only_generic_dispatch(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let Some(trait_path) = site.generic_qself_trait.as_deref() else {
            return false;
        };
        if self.exact_self_type(inventory, site, evidence).is_some()
            || evidence.callee_nodes.iter().any(|node| {
                matches!(
                    inventory.nodes[*node].trait_route,
                    Some(TraitRoute::Implementation { .. })
                )
            })
        {
            return false;
        }
        let DispatchKind::Call { member, .. } = &site.kind;
        let declarations = evidence.callee_nodes.iter().filter(|node| {
            let Some(TraitRoute::Declaration(key)) = &inventory.nodes[**node].trait_route else {
                return false;
            };
            key.member == *member
                && (key.trait_path == trait_path
                    || !trait_path.contains("::")
                        && last_path_segment(&key.trait_path) == trait_path)
        });
        declarations.take(2).count() == 1
    }

    fn exact_self_type(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> Option<usize> {
        let callee_self = evidence
            .self_nodes
            .intersection(&evidence.callee_nodes)
            .copied()
            .collect::<BTreeSet<_>>();
        let mut preferred = callee_self.iter().filter(|node| {
            inventory.nodes[**node].type_declaration && !inventory.nodes[**node].type_alias
        });
        if let Some(exact) = preferred.next().copied() {
            return (site.exact_callee_self_syntax && preferred.next().is_none()).then_some(exact);
        }
        if !site.exact_self_syntax {
            return None;
        }
        let mut types = evidence.self_nodes.iter().filter(|node| {
            inventory.nodes[**node].type_declaration && !inventory.nodes[**node].type_alias
        });
        let exact = types.next().copied()?;
        types.next().is_none().then_some(exact)
    }

    fn selected_non_trait_callable(
        &self,
        inventory: &Inventory,
        evidence: &DispatchEvidence,
    ) -> bool {
        let selected = evidence.callee_nodes.iter().any(|node| {
            inventory.nodes[*node].callable && inventory.nodes[*node].trait_route.is_none()
        });
        let trait_callable = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Declaration(_) | TraitRoute::Implementation { .. })
            )
        });
        selected && !trait_callable
    }

    fn local_default_dispatch(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let Some(self_type) = self.exact_self_type(inventory, site, evidence) else {
            return false;
        };
        if evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Implementation { .. })
            )
        }) {
            return false;
        }
        let DispatchKind::Call { member, .. } = &site.kind;
        let mut declarations = evidence.callee_nodes.iter().filter(|node| {
            matches!(
                &inventory.nodes[**node].trait_route,
                Some(TraitRoute::Declaration(key)) if key.member == *member
            )
        });
        let Some(declaration) = declarations.next().copied() else {
            return false;
        };
        if declarations.next().is_some() || !inventory.nodes[declaration].trait_default {
            return false;
        }
        let Some(TraitRoute::Declaration(declaration_key)) =
            &inventory.nodes[declaration].trait_route
        else {
            unreachable!("a selected declaration retains its route")
        };
        let applicable = inventory.trait_activations.iter().any(|activation| {
            activation.declaration == Some(declaration) && activation.self_type == self_type
        });
        let unresolved = inventory
            .unresolved_trait_implementations
            .iter()
            .any(|implementation| {
                matches!(
                    &inventory.nodes[*implementation].trait_route,
                    Some(TraitRoute::Implementation { key, external: false, .. })
                        if key == declaration_key || trait_member_matches(declaration_key, key)
                )
            });
        !applicable && !unresolved
    }

    fn dispatch_matches<'inventory>(
        &self,
        inventory: &'inventory Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> Vec<&'inventory TraitActivation> {
        let exact_self_type = self.exact_self_type(inventory, site, evidence);
        let has_trait_declaration_evidence = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Declaration(_))
            )
        });
        inventory
            .trait_activations
            .iter()
            .filter(|activation| {
                activation.matches(
                    site,
                    &evidence.callee_nodes,
                    exact_self_type,
                    has_trait_declaration_evidence,
                )
            })
            .collect()
    }

    fn dispatch_requires_resolution(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> bool {
        let self_type = self.exact_self_type(inventory, site, evidence);
        let declaration_keys = evidence
            .callee_nodes
            .iter()
            .filter_map(|node| match &inventory.nodes[*node].trait_route {
                Some(TraitRoute::Declaration(key)) => Some(key),
                _ => None,
            })
            .collect::<Vec<_>>();
        let local_implementation = !declaration_keys.is_empty()
            && (inventory.trait_activations.iter().any(|activation| {
                activation
                    .declaration
                    .is_some_and(|declaration| evidence.callee_nodes.contains(&declaration))
            }) || inventory
                .unresolved_trait_implementations
                .iter()
                .any(|implementation| {
                    matches!(
                        &inventory.nodes[*implementation].trait_route,
                        Some(TraitRoute::Implementation { key, external: false, .. })
                            if declaration_keys.iter().any(|declaration| {
                                declaration == &key || trait_member_matches(declaration, key)
                            })
                    )
                }));
        let external_implementation = declaration_keys.is_empty()
            && inventory.trait_activations.iter().any(|activation| {
                activation.declaration.is_none()
                    && activation.call_compatible(site, &evidence.callee_nodes)
                    && (evidence.callee_nodes.contains(&activation.implementation)
                        || self_type.is_some_and(|self_type| {
                            self_type == activation.self_type
                                && matches!(
                                    &site.kind,
                                    DispatchKind::Call {
                                        trait_qualifier: Some(_),
                                        ..
                                    }
                                )
                        }))
            });
        let direct_implementation = evidence.callee_nodes.iter().any(|node| {
            matches!(
                inventory.nodes[*node].trait_route,
                Some(TraitRoute::Implementation { .. })
            )
        });
        let unqualified_external_self = self_type.is_some_and(|self_type| {
            matches!(
                &site.kind,
                DispatchKind::Call {
                    trait_qualifier: None,
                    ..
                }
            ) && inventory.trait_activations.iter().any(|activation| {
                activation.declaration.is_none()
                    && activation.self_type == self_type
                    && activation.call_compatible(site, &evidence.callee_nodes)
            })
        });
        let unsupported_explicit_qself = site.explicit_qself
            && site.generic_qself_trait.is_none()
            && self_type.is_none()
            && !declaration_keys.is_empty();
        direct_implementation
            || local_implementation
            || external_implementation
            || unqualified_external_self
            || unsupported_explicit_qself
    }

    fn dispatch_resolution(
        &self,
        inventory: &Inventory,
        site: &DispatchSite,
        evidence: &DispatchEvidence,
    ) -> DispatchResolution {
        if self.declaration_only_generic_dispatch(inventory, site, evidence)
            || self.local_default_dispatch(inventory, site, evidence)
        {
            return DispatchResolution::DeclarationOnly;
        }
        if self.selected_non_trait_callable(inventory, evidence) {
            return DispatchResolution::NoCredit;
        }
        let matching = self.dispatch_matches(inventory, site, evidence);
        if !self.dispatch_requires_resolution(inventory, site, evidence) {
            return DispatchResolution::NoCredit;
        }
        match matching.as_slice() {
            [activation] => DispatchResolution::Implementation(activation.implementation),
            _ => DispatchResolution::Failure(matching.len()),
        }
    }

    pub(super) fn live(&self, inventory: &Inventory) -> BTreeSet<usize> {
        let mut live = self.roots.clone();
        let mut pending: VecDeque<usize> = live.iter().copied().collect();
        loop {
            while let Some(caller) = pending.pop_front() {
                for &used in self.edges.get(&caller).into_iter().flatten() {
                    if live.insert(used) {
                        pending.push_back(used);
                    }
                }
            }
            let mut activated = false;
            for (&(file, site), evidence) in &self.dispatches {
                if !live.contains(&evidence.caller) {
                    continue;
                }
                let site = &inventory.files[file].dispatch_sites[site];
                if let DispatchResolution::Implementation(implementation) =
                    self.dispatch_resolution(inventory, site, evidence)
                    && live.insert(implementation)
                {
                    pending.push_back(implementation);
                    activated = true;
                }
            }
            for activation in &inventory.implicit_trait_activations {
                let triggered = match activation.trigger {
                    ImplicitTraitTrigger::ConstructedType(ty) => self
                        .constructions
                        .iter()
                        .any(|(caller, types)| live.contains(caller) && types.contains(&ty)),
                    ImplicitTraitTrigger::LiveNode(node) => live.contains(&node),
                };
                if triggered && live.insert(activation.implementation) {
                    pending.push_back(activation.implementation);
                    activated = true;
                }
            }
            if !activated {
                break;
            }
        }
        live
    }

    pub(super) fn dispatch_failures(&self, inventory: &Inventory) -> BTreeSet<DispatchFailure> {
        let live = self.live(inventory);
        let mut failures = BTreeSet::new();
        for (&(file, site), evidence) in &self.dispatches {
            if !live.contains(&evidence.caller) {
                continue;
            }
            let dispatch = &inventory.files[file].dispatch_sites[site];
            if let DispatchResolution::Failure(implementations) =
                self.dispatch_resolution(inventory, dispatch, evidence)
            {
                failures.insert(DispatchFailure {
                    file,
                    site,
                    implementations,
                });
            }
        }
        failures
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(super) struct DispatchFailure {
    file: usize,
    site: usize,
    pub(super) implementations: usize,
}

pub(super) fn dispatch_failure(inventory: &Inventory, failure: DispatchFailure) -> String {
    let dispatch = &inventory.files[failure.file].dispatch_sites[failure.site];
    format!(
        "live dispatch `{}` at {}:{} resolves to {} audited trait implementations",
        match &dispatch.kind {
            DispatchKind::Call { member, .. } => member,
        },
        inventory.files[failure.file].relative.display(),
        dispatch.range.start.line,
        failure.implementations
    )
}

#[derive(Default, Debug)]
pub(super) struct GraphUses {
    pub(super) graphs: Vec<Uses>,
}

impl GraphUses {
    pub(super) fn observe_cargo(
        &mut self,
        graph: usize,
        kind: RootKind,
        inventory: &Inventory,
        root: &Path,
        diagnostics: &[MarkerDiagnostic],
        instrumentation: &Instrumentation,
    ) -> Result<()> {
        if self.graphs.len() <= graph {
            self.graphs.resize_with(graph + 1, Uses::default);
        }
        self.graphs[graph].observe_cargo(kind, inventory, root, diagnostics, instrumentation)
    }

    pub(super) fn dead<'inventory>(
        &self,
        inventory: &'inventory Inventory,
    ) -> Vec<&'inventory Node> {
        let live = self
            .graphs
            .iter()
            .flat_map(|graph| graph.live(inventory))
            .collect::<BTreeSet<_>>();
        inventory
            .nodes
            .iter()
            .filter(|node| node.report && !live.contains(&node.id))
            .collect()
    }

    pub(super) fn validate_dispatches(&self, inventory: &Inventory) -> Result<()> {
        let mut failures: BTreeMap<DispatchFailure, BTreeSet<usize>> = BTreeMap::new();
        for (graph_index, graph) in self.graphs.iter().enumerate() {
            for failure in graph.dispatch_failures(inventory) {
                failures.entry(failure).or_default().insert(graph_index);
            }
        }
        ensure!(
            failures.is_empty(),
            "{}",
            failures
                .iter()
                .map(|(failure, graphs)| {
                    format!(
                        "{}; compiler graphs {}",
                        dispatch_failure(inventory, *failure),
                        graphs
                            .iter()
                            .map(usize::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        );
        Ok(())
    }
}
