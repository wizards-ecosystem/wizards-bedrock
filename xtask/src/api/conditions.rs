use syn::visit::{self, Visit};

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Condition {
    Always,
    Never,
    Maybe,
}

impl Condition {
    fn not(self) -> Self {
        match self {
            Self::Always => Self::Never,
            Self::Never => Self::Always,
            Self::Maybe => Self::Maybe,
        }
    }

    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::Never, _) | (_, Self::Never) => Self::Never,
            (Self::Always, Self::Always) => Self::Always,
            _ => Self::Maybe,
        }
    }

    fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Always, _) | (_, Self::Always) => Self::Always,
            (Self::Never, Self::Never) => Self::Never,
            _ => Self::Maybe,
        }
    }
}

pub(super) fn verification_only(attributes: &[Attribute]) -> bool {
    attributes.iter().any(attribute_disables_production)
}

fn attribute_disables_production(attribute: &Attribute) -> bool {
    if attribute.path().is_ident("cfg") {
        return attribute
            .parse_args::<Meta>()
            .is_ok_and(|meta| cfg_condition(&meta) == Condition::Never);
    }
    if !attribute.path().is_ident("cfg_attr") {
        return false;
    }
    let Ok(arguments) = attribute.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return false;
    };
    let mut arguments = arguments.iter();
    let Some(predicate) = core::iter::Iterator::next(&mut arguments) else {
        return false;
    };
    cfg_condition(predicate) == Condition::Always && arguments.any(meta_disables_production)
}

fn meta_disables_production(meta: &Meta) -> bool {
    if meta.path().is_ident("cfg") {
        let Meta::List(list) = meta else {
            return false;
        };
        return list
            .parse_args::<Meta>()
            .is_ok_and(|condition| cfg_condition(&condition) == Condition::Never);
    }
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    let Ok(arguments) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
    else {
        return false;
    };
    let mut arguments = arguments.iter();
    let Some(predicate) = core::iter::Iterator::next(&mut arguments) else {
        return false;
    };
    cfg_condition(predicate) == Condition::Always && arguments.any(meta_disables_production)
}

fn cfg_condition(meta: &Meta) -> Condition {
    match meta {
        Meta::Path(path) if path.is_ident("test") => Condition::Never,
        Meta::Path(_) => Condition::Maybe,
        Meta::NameValue(value) if value.path.is_ident("feature") => match &value.value {
            Expr::Lit(expression) if matches!(&expression.lit, Lit::Str(feature) if feature.value() == "selftest") => {
                Condition::Never
            }
            _ => Condition::Maybe,
        },
        Meta::NameValue(_) => Condition::Maybe,
        Meta::List(list) if list.path.is_ident("not") => list
            .parse_args::<Meta>()
            .map_or(Condition::Maybe, |condition| {
                cfg_condition(&condition).not()
            }),
        Meta::List(list) if list.path.is_ident("all") || list.path.is_ident("any") => {
            let Ok(arguments) =
                list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return Condition::Maybe;
            };
            if list.path.is_ident("all") {
                arguments.iter().fold(Condition::Always, |value, meta| {
                    value.and(cfg_condition(meta))
                })
            } else {
                arguments.iter().fold(Condition::Never, |value, meta| {
                    value.or(cfg_condition(meta))
                })
            }
        }
        Meta::List(_) => Condition::Maybe,
    }
}

pub(super) fn reject_deprecation_suppression(root: &Path) -> Result<()> {
    let mut paths = Vec::new();
    for directory in COPY_DIRECTORIES {
        let directory = root.join(directory);
        collect_optional_rust_files(&directory, &mut paths)?;
    }
    for path in paths {
        let source = read_bounded_authority_text(&path, "API audit lint-policy source")?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parsing {} for lint suppression", path.display()))?;
        let mut visitor = Suppression::default();
        visitor.visit_file(&syntax);
        let token_suppression = suppression_in_tokens(
            <TokenStream as core::str::FromStr>::from_str(&source).map_err(|reason| {
                anyhow::anyhow!(
                    "tokenizing {} for lint suppression: {reason}",
                    path.display()
                )
            })?,
        );
        if let Some(at) = visitor.at.or(token_suppression) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            bail!(
                "{}:{} suppresses deprecation diagnostics required by the API audit",
                relative.display(),
                at.start().line
            );
        }
    }
    Ok(())
}

fn suppression_in_tokens(stream: TokenStream) -> Option<Span> {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    for (index, token) in tokens.iter().enumerate() {
        if matches!(token, TokenTree::Punct(punctuation) if punctuation.as_char() == '#') {
            let mut attribute = index + 1;
            if matches!(tokens.get(attribute), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '!')
            {
                attribute += 1;
            }
            if let Some(TokenTree::Group(group)) = tokens.get(attribute)
                && group.delimiter() == Delimiter::Bracket
                && attribute_may_suppress(group.stream())
            {
                return Some(token.span());
            }
        }
        if let TokenTree::Group(group) = token
            && let Some(at) = suppression_in_tokens(group.stream())
        {
            return Some(at);
        }
    }
    None
}

fn attribute_may_suppress(stream: TokenStream) -> bool {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    if matches!(tokens.first(), Some(TokenTree::Punct(punctuation)) if punctuation.as_char() == '$')
    {
        return true;
    }
    let Some(TokenTree::Ident(name)) = tokens.first() else {
        return false;
    };
    let Some(TokenTree::Group(arguments)) = tokens.get(1) else {
        return false;
    };
    if arguments.delimiter() != Delimiter::Parenthesis {
        return false;
    }
    if name == "allow" || name == "expect" {
        return lint_arguments_may_suppress(arguments.stream());
    }
    if name == "cfg_attr" {
        return cfg_attr_tokens_may_suppress(arguments.stream());
    }
    false
}

fn cfg_attr_tokens_may_suppress(stream: TokenStream) -> bool {
    let tokens = stream.into_iter().collect::<Vec<_>>();
    tokens.iter().enumerate().any(|(index, token)| {
        matches!(token, TokenTree::Punct(punctuation) if punctuation.as_char() == '$')
            || matches!(token, TokenTree::Ident(name)
                if (name == "allow" || name == "expect")
                    && matches!(tokens.get(index + 1), Some(TokenTree::Group(arguments))
                        if arguments.delimiter() == Delimiter::Parenthesis
                            && lint_arguments_may_suppress(arguments.stream())))
            || matches!(token, TokenTree::Group(group) if cfg_attr_tokens_may_suppress(group.stream()))
    })
}

fn lint_arguments_may_suppress(stream: TokenStream) -> bool {
    stream.into_iter().any(|token| match token {
        TokenTree::Ident(name) => name == "deprecated" || name == "warnings",
        TokenTree::Punct(punctuation) => punctuation.as_char() == '$',
        TokenTree::Group(group) => lint_arguments_may_suppress(group.stream()),
        _ => false,
    })
}

#[derive(Default)]
struct Suppression {
    at: Option<Span>,
}

impl<'ast> Visit<'ast> for Suppression {
    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if self.at.is_none() && suppresses_deprecation(&attribute.meta) {
            self.at = Some(attribute.span());
        }
        visit::visit_attribute(self, attribute);
    }
}

fn suppresses_deprecation(meta: &Meta) -> bool {
    if meta.path().is_ident("allow") || meta.path().is_ident("expect") {
        let Meta::List(list) = meta else {
            return false;
        };
        return list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|lints| {
                lints.iter().any(|lint| {
                    matches!(lint, Meta::Path(path) if path.is_ident("deprecated") || path.is_ident("warnings"))
                })
            });
    }
    if !meta.path().is_ident("cfg_attr") {
        return false;
    }
    let Meta::List(list) = meta else {
        return false;
    };
    list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        .is_ok_and(|arguments| arguments.iter().skip(1).any(suppresses_deprecation))
}

pub(super) fn has_attribute(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident(name))
}
