//! The parsed shape of a catalog enum, with every rule checked.

use std::collections::HashSet;

use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::{
    Attribute, Data, DeriveInput, Error, Expr, ExprLit, Field, Fields, Ident, Lit, LitStr, Meta,
    Result, Variant, parse::Parse, parse::ParseStream,
};

use crate::template;

/// What a message is for; mirrors `slopvac_messages::Kind`.
#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Error,
    Notice,
    Text,
}

/// How a parameter is passed; mirrors `slopvac_messages::ParamKind`.
#[derive(Clone, Copy)]
pub(crate) enum ParamKind {
    Text,
    Count,
    Path,
}

pub(crate) struct Param {
    /// The field the value comes from.
    pub(crate) field: Ident,
    /// The name used in the template, without `r#`.
    pub(crate) name: String,
    pub(crate) kind: ParamKind,
}

pub(crate) struct Message {
    pub(crate) variant: Ident,
    pub(crate) code: String,
    pub(crate) kind: Kind,
    pub(crate) template: String,
    pub(crate) summary: String,
    pub(crate) params: Vec<Param>,
}

/// The one field of a variant that wraps another catalog.
pub(crate) enum Wrapped {
    Positional,
    Named(Ident),
}

pub(crate) struct Delegate {
    pub(crate) variant: Ident,
    pub(crate) field: Wrapped,
}

pub(crate) enum Item {
    Message(Message),
    Delegate(Delegate),
}

pub(crate) struct Catalog {
    pub(crate) ident: Ident,
    pub(crate) items: Vec<Item>,
}

enum CodeArg {
    Code(LitStr),
    Delegate,
}

impl Parse for CodeArg {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        if input.peek(Ident) {
            let word: Ident = input.parse()?;
            if word == "delegate" {
                return Ok(Self::Delegate);
            }
            return Err(Error::new(
                word.span(),
                "expected a string code, as in `#[code(\"my-code\")]`, or `delegate`",
            ));
        }
        Ok(Self::Code(input.parse()?))
    }
}

impl Catalog {
    pub(crate) fn parse(input: &DeriveInput) -> Result<Self> {
        let Data::Enum(data) = &input.data else {
            return Err(Error::new(
                input.ident.span(),
                "`Catalog` can only be derived for an enum",
            ));
        };
        if !input.generics.params.is_empty() {
            return Err(Error::new(
                input.generics.span(),
                "a catalog enum cannot be generic",
            ));
        }
        if data.variants.is_empty() {
            return Err(Error::new(input.ident.span(), "a catalog needs a variant"));
        }
        let items = data
            .variants
            .iter()
            .map(parse_variant)
            .collect::<Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        for item in &items {
            if let Item::Message(m) = item
                && !seen.insert(m.code.as_str())
            {
                return Err(Error::new(
                    m.variant.span(),
                    format!("the code `{}` is used twice in this enum", m.code),
                ));
            }
        }
        Ok(Self {
            ident: input.ident.clone(),
            items,
        })
    }
}

fn one_attr<'a>(variant: &'a Variant, name: &str) -> Result<Option<&'a Attribute>> {
    let mut found = variant.attrs.iter().filter(|a| a.path().is_ident(name));
    let first = found.next();
    match found.next() {
        Some(second) => Err(Error::new(
            second.span(),
            format!("`#[{name}]` is given twice"),
        )),
        None => Ok(first),
    }
}

fn parse_variant(variant: &Variant) -> Result<Item> {
    if let Some(attr) = variant.attrs.iter().find(|a| a.path().is_ident("error")) {
        return Err(Error::new(
            attr.span(),
            "remove `#[error]`: the catalog renders this variant, so give the text as \
             `#[message(\"…\")]`",
        ));
    }
    let Some(code_attr) = one_attr(variant, "code")? else {
        return Err(Error::new(
            variant.ident.span(),
            "missing `#[code(\"kebab-case\")]` or `#[code(delegate)]`",
        ));
    };
    match code_attr.parse_args::<CodeArg>()? {
        CodeArg::Delegate => parse_delegate(variant).map(Item::Delegate),
        CodeArg::Code(code) => parse_message(variant, &code).map(Item::Message),
    }
}

fn parse_delegate(variant: &Variant) -> Result<Delegate> {
    for name in ["message", "notice", "text"] {
        if let Some(attr) = one_attr(variant, name)? {
            return Err(Error::new(
                attr.span(),
                "a delegating variant renders through the catalog it wraps; remove the template",
            ));
        }
    }
    let field = match &variant.fields {
        Fields::Unnamed(f) if f.unnamed.len() == 1 => Wrapped::Positional,
        Fields::Named(f) if f.named.len() == 1 => {
            Wrapped::Named(f.named[0].ident.clone().expect("named field has an ident"))
        }
        _ => {
            return Err(Error::new(
                variant.ident.span(),
                "a `#[code(delegate)]` variant has exactly one field: the catalog it wraps",
            ));
        }
    };
    Ok(Delegate {
        variant: variant.ident.clone(),
        field,
    })
}

fn parse_message(variant: &Variant, code: &LitStr) -> Result<Message> {
    if let Some(problem) = template::code_problem(&code.value()) {
        return Err(Error::new(code.span(), problem));
    }
    let (kind, template_lit) = template_attr(variant)?;
    let params = params(variant)?;
    let template = template_lit.value();
    if template != template.trim() || template.is_empty() {
        return Err(Error::new(
            template_lit.span(),
            "the template must not be empty or start or end with whitespace",
        ));
    }
    let names = template::placeholders(&template)
        .map_err(|problem| Error::new(template_lit.span(), problem))?;
    for name in names {
        if !params.iter().any(|p| p.name == name) {
            return Err(Error::new(
                template_lit.span(),
                format!(
                    "the template uses `{{{name}}}` but this variant has no parameter field `{name}`"
                ),
            ));
        }
    }
    Ok(Message {
        variant: variant.ident.clone(),
        code: code.value(),
        kind,
        template,
        summary: summary(variant)?,
        params,
    })
}

fn template_attr(variant: &Variant) -> Result<(Kind, LitStr)> {
    let mut found = Vec::new();
    for (name, kind) in [
        ("message", Kind::Error),
        ("notice", Kind::Notice),
        ("text", Kind::Text),
    ] {
        if let Some(attr) = one_attr(variant, name)? {
            found.push((kind, attr.parse_args::<LitStr>()?));
        }
    }
    let mut found = found.into_iter();
    match (found.next(), found.next()) {
        (Some(one), None) => Ok(one),
        (None, _) => Err(Error::new(
            variant.ident.span(),
            "missing the template: add `#[message(\"…\")]`, `#[notice(\"…\")]` or `#[text(\"…\")]`",
        )),
        (Some(_), Some((_, extra))) => Err(Error::new(
            extra.span(),
            "give one of `#[message]`, `#[notice]` and `#[text]`, not several",
        )),
    }
}

fn params(variant: &Variant) -> Result<Vec<Param>> {
    let fields = match &variant.fields {
        Fields::Unit => return Ok(Vec::new()),
        Fields::Named(named) => &named.named,
        Fields::Unnamed(unnamed) => {
            return Err(Error::new(
                unnamed.span(),
                "tuple variants are not allowed: parameters are named, so use a struct variant",
            ));
        }
    };
    fields.iter().filter(|f| !is_source(f)).map(param).collect()
}

/// Fields thiserror treats as the error source: wrapped errors, not message parameters.
fn is_source(field: &Field) -> bool {
    field.attrs.iter().any(|a| {
        ["source", "from", "backtrace"]
            .iter()
            .any(|name| a.path().is_ident(name))
    })
}

fn param(field: &Field) -> Result<Param> {
    let ident = field.ident.clone().expect("named field has an ident");
    let name = ident.unraw().to_string();
    if !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Err(Error::new(
            ident.span(),
            "a parameter name starts with a letter, so it is a valid Fluent variable",
        ));
    }
    let count = field.attrs.iter().find(|a| a.path().is_ident("count"));
    let path = field
        .attrs
        .iter()
        .find(|a| a.path().is_ident("display_path"));
    let kind = match (count, path) {
        (Some(_), Some(path)) => {
            return Err(Error::new(
                path.span(),
                "a parameter is either `#[count]` or `#[display_path]`, not both",
            ));
        }
        (Some(_), None) => ParamKind::Count,
        (None, Some(_)) => ParamKind::Path,
        (None, None) => ParamKind::Text,
    };
    Ok(Param {
        field: ident,
        name,
        kind,
    })
}

/// The first paragraph of the doc comment, on one line.
fn summary(variant: &Variant) -> Result<String> {
    let mut lines: Vec<String> = Vec::new();
    for attr in &variant.attrs {
        if let Meta::NameValue(nv) = &attr.meta
            && nv.path.is_ident("doc")
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(text),
                ..
            }) = &nv.value
        {
            lines.push(text.value().trim().to_owned());
        }
    }
    let paragraph: Vec<&str> = lines
        .iter()
        .map(String::as_str)
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect();
    if paragraph.is_empty() {
        return Err(Error::new(
            variant.ident.span(),
            "add a doc comment: its first paragraph is the summary on the reference page",
        ));
    }
    Ok(paragraph.join(" "))
}
