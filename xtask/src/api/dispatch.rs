use syn::visit::{self, Visit};

use super::*;

#[derive(Debug, Clone)]
pub(super) struct DispatchSite {
    pub(super) range: SourceRange,
    pub(super) callee_evidence: Vec<SourceRange>,
    pub(super) self_evidence: Vec<SourceRange>,
    pub(super) exact_self_syntax: bool,
    pub(super) exact_callee_self_syntax: bool,
    pub(super) explicit_qself: bool,
    pub(super) generic_qself_trait: Option<String>,
    pub(super) kind: DispatchKind,
}

impl DispatchSite {
    pub(super) fn callee_names(&self, callee_nodes: &BTreeSet<usize>, self_type: usize) -> bool {
        self.exact_callee_self_syntax && !self.explicit_qself && callee_nodes.contains(&self_type)
    }
}

#[derive(Debug, Clone)]
pub(super) struct ValueConstruction {
    pub(super) range: SourceRange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DispatchKind {
    Call {
        member: String,
        trait_qualifier: Option<String>,
    },
}

#[derive(Default)]
pub(super) struct DispatchSites {
    pub(super) sites: Vec<DispatchSite>,
    generic_bounds: Vec<BTreeMap<String, BTreeSet<String>>>,
}

impl DispatchSites {
    fn push_generics(&mut self, generics: &syn::Generics, inherit: bool) {
        let mut bounds = if inherit {
            self.generic_bounds.last().cloned().unwrap_or_default()
        } else {
            BTreeMap::new()
        };
        for parameter in &generics.params {
            let syn::GenericParam::Type(parameter) = parameter else {
                continue;
            };
            bounds.insert(
                parameter.ident.to_string(),
                direct_trait_bounds(&parameter.bounds),
            );
        }
        if let Some(where_clause) = &generics.where_clause {
            for predicate in &where_clause.predicates {
                let syn::WherePredicate::Type(predicate) = predicate else {
                    continue;
                };
                let Some(parameter) = direct_type_parameter(&predicate.bounded_ty) else {
                    continue;
                };
                let Some(bounds) = bounds.get_mut(&parameter) else {
                    continue;
                };
                bounds.extend(direct_trait_bounds(&predicate.bounds));
            }
        }
        self.generic_bounds.push(bounds);
    }

    fn pop_generics(&mut self) {
        self.generic_bounds
            .pop()
            .expect("a visited generic scope has a matching environment");
    }

    fn generic_qself_trait(&self, expression: &syn::ExprPath) -> Option<String> {
        let qself = expression.qself.as_ref()?;
        qself.as_token?;
        if qself.position == 0 || qself.position + 1 != expression.path.segments.len() {
            return None;
        }
        let parameter = direct_type_parameter(&qself.ty)?;
        let trait_path = direct_path_prefix(&expression.path, qself.position)?;
        self.generic_bounds
            .iter()
            .rev()
            .find_map(|bounds| bounds.get(&parameter))
            .is_some_and(|bounds| bounds.contains(&trait_path))
            .then_some(trait_path)
    }
}

fn direct_trait_bounds(bounds: &Punctuated<syn::TypeParamBound, Token![+]>) -> BTreeSet<String> {
    bounds
        .iter()
        .filter_map(|bound| {
            let syn::TypeParamBound::Trait(bound) = bound else {
                return None;
            };
            if bound.paren_token.is_some()
                || bound.lifetimes.is_some()
                || !matches!(bound.modifier, syn::TraitBoundModifier::None)
            {
                return None;
            }
            direct_path(&bound.path)
        })
        .collect()
}

fn qself_trait_path(expression: &syn::ExprPath) -> Option<String> {
    let qself = expression.qself.as_ref()?;
    qself.as_token?;
    if qself.position == 0 || qself.position + 1 != expression.path.segments.len() {
        return None;
    }
    Some(
        expression
            .path
            .segments
            .iter()
            .take(qself.position)
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    )
}

fn direct_type_parameter(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(ty) = ty else {
        return None;
    };
    if ty.qself.is_some() || ty.path.leading_colon.is_some() || ty.path.segments.len() != 1 {
        return None;
    }
    let parameter = ty.path.segments.first()?;
    matches!(parameter.arguments, syn::PathArguments::None).then(|| parameter.ident.to_string())
}

fn direct_path(path: &syn::Path) -> Option<String> {
    direct_path_prefix(path, path.segments.len())
}

fn direct_path_prefix(path: &syn::Path, count: usize) -> Option<String> {
    if path.leading_colon.is_some()
        || count == 0
        || path
            .segments
            .iter()
            .take(count)
            .take(count.saturating_sub(1))
            .any(|segment| !matches!(segment.arguments, syn::PathArguments::None))
    {
        return None;
    }
    Some(
        path.segments
            .iter()
            .take(count)
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
    )
}

impl<'ast> Visit<'ast> for DispatchSites {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.push_generics(&item.sig.generics, false);
        visit::visit_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.push_generics(&item.generics, false);
        visit::visit_item_impl(self, item);
        self.pop_generics();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.push_generics(&item.sig.generics, true);
        visit::visit_impl_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.push_generics(&item.generics, false);
        visit::visit_item_trait(self, item);
        self.pop_generics();
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.push_generics(&item.sig.generics, true);
        visit::visit_trait_item_fn(self, item);
        self.pop_generics();
    }

    fn visit_expr_call(&mut self, expression: &'ast syn::ExprCall) {
        if let Expr::Path(path) = &*expression.func
            && let Some(member) = path.path.segments.last()
        {
            let generic_qself_trait = self.generic_qself_trait(path);
            let trait_qualifier = qself_trait_path(path);
            let callee_receiver = if let Some(qself) = &path.qself {
                Some(ExactReceiver {
                    range: range(qself.ty.span()),
                    exact_nominal: exact_nominal_type_path(&qself.ty).is_some(),
                })
            } else {
                path_prefix_receiver(&path.path)
            };
            let argument_receiver = path.qself.is_none().then(|| {
                expression
                    .args
                    .first()
                    .and_then(exact_dispatch_receiver_range)
            });
            let argument_receiver = argument_receiver.flatten();
            self.sites.push(DispatchSite {
                range: range(expression.span()),
                callee_evidence: vec![range(expression.func.span())],
                self_evidence: callee_receiver
                    .iter()
                    .chain(&argument_receiver)
                    .map(|receiver| receiver.range)
                    .collect(),
                exact_self_syntax: argument_receiver.is_some_and(|receiver| receiver.exact_nominal),
                exact_callee_self_syntax: callee_receiver
                    .is_some_and(|receiver| receiver.exact_nominal),
                explicit_qself: path.qself.is_some(),
                generic_qself_trait,
                kind: DispatchKind::Call {
                    member: member.ident.to_string(),
                    trait_qualifier: path.qself.as_ref().and(trait_qualifier),
                },
            });
        }
        visit::visit_expr_call(self, expression);
    }

    fn visit_expr_method_call(&mut self, expression: &'ast syn::ExprMethodCall) {
        let receiver = exact_dispatch_receiver_range(&expression.receiver);
        self.sites.push(DispatchSite {
            range: range(expression.span()),
            callee_evidence: vec![range(expression.method.span())],
            self_evidence: receiver.iter().map(|receiver| receiver.range).collect(),
            exact_self_syntax: receiver.is_some_and(|receiver| receiver.exact_nominal),
            exact_callee_self_syntax: false,
            explicit_qself: false,
            generic_qself_trait: None,
            kind: DispatchKind::Call {
                member: expression.method.to_string(),
                trait_qualifier: None,
            },
        });
        visit::visit_expr_method_call(self, expression);
    }
}

#[derive(Debug, Clone, Copy)]
struct ExactReceiver {
    range: SourceRange,
    exact_nominal: bool,
}

fn path_prefix_receiver(path: &syn::Path) -> Option<ExactReceiver> {
    let prefix = path.segments.len().checked_sub(1)?;
    let receiver = path.segments.iter().nth(prefix.checked_sub(1)?)?;
    Some(ExactReceiver {
        range: range(receiver.ident.span()),
        exact_nominal: path.leading_colon.is_none()
            && path
                .segments
                .iter()
                .take(prefix)
                .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
    })
}

fn exact_dispatch_receiver_range(expression: &Expr) -> Option<ExactReceiver> {
    match expression {
        Expr::Path(path) if path.qself.is_none() => {
            path.path.segments.last().map(|segment| ExactReceiver {
                range: range(segment.ident.span()),
                exact_nominal: path.path.leading_colon.is_none()
                    && path
                        .path
                        .segments
                        .iter()
                        .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
            })
        }
        Expr::Struct(structure) if structure.qself.is_none() => {
            structure.path.segments.last().map(|segment| ExactReceiver {
                range: range(segment.ident.span()),
                exact_nominal: structure.path.leading_colon.is_none()
                    && structure
                        .path
                        .segments
                        .iter()
                        .all(|segment| matches!(segment.arguments, syn::PathArguments::None)),
            })
        }
        Expr::Reference(reference) => exact_dispatch_receiver_range(&reference.expr),
        Expr::Paren(parenthesized) => exact_dispatch_receiver_range(&parenthesized.expr),
        Expr::Group(grouped) => exact_dispatch_receiver_range(&grouped.expr),
        Expr::Unary(unary) => exact_dispatch_receiver_range(&unary.expr),
        _ => None,
    }
}

#[derive(Default)]
pub(super) struct ValueConstructions {
    pub(super) sites: Vec<ValueConstruction>,
}

impl ValueConstructions {
    fn record(&mut self, path: &syn::Path) {
        if let Some(segment) = path.segments.last() {
            self.sites.push(ValueConstruction {
                range: range(segment.ident.span()),
            });
        }
    }
}

impl<'ast> Visit<'ast> for ValueConstructions {
    fn visit_item_static(&mut self, _item: &'ast syn::ItemStatic) {}

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        if expression.qself.is_none() {
            self.record(&expression.path);
        }
        visit::visit_expr_path(self, expression);
    }

    fn visit_expr_struct(&mut self, expression: &'ast syn::ExprStruct) {
        if expression.qself.is_none() {
            self.record(&expression.path);
        }
        visit::visit_expr_struct(self, expression);
    }
}
