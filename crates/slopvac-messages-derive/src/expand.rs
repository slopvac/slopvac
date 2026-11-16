//! Code generation for the parsed catalog.

use proc_macro2::TokenStream;
use quote::quote;
use syn::Ident;

use crate::model::{Catalog, Delegate, Item, Kind, Message, ParamKind, Wrapped};
use crate::template;

pub(crate) fn expand(catalog: &Catalog) -> TokenStream {
    let ident = &catalog.ident;
    let messages: Vec<&Message> = catalog
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Message(m) => Some(m),
            Item::Delegate(_) => None,
        })
        .collect();
    let count = messages.len();
    let entries = messages.iter().map(|m| entry(ident, m));
    let entry_arms = catalog
        .items
        .iter()
        .map(|item| entry_arm(ident, &messages, item));
    let args_arms = catalog.items.iter().map(|item| args_arm(ident, item));
    let as_static = as_static(catalog, &messages);

    quote! {
        const _: () = {
            static ENTRIES: [::slopvac_messages::Entry; #count] = [#(#entries),*];

            impl ::slopvac_messages::Catalog for #ident {
                fn entries() -> &'static [::slopvac_messages::Entry] {
                    &ENTRIES
                }

                fn entry(&self) -> &'static ::slopvac_messages::Entry {
                    match self { #(#entry_arms)* }
                }

                fn args(&self) -> ::std::vec::Vec<::slopvac_messages::Arg> {
                    match self { #(#args_arms)* }
                }
            }

            impl ::core::fmt::Display for #ident {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    ::slopvac_messages::write_message(self, f)
                }
            }

            #as_static
        };
    }
}

fn entry(enum_ident: &Ident, m: &Message) -> TokenStream {
    let Message {
        variant,
        code,
        kind,
        template,
        summary,
        params,
    } = m;
    let kind = match kind {
        Kind::Error => quote!(Error),
        Kind::Notice => quote!(Notice),
        Kind::Text => quote!(Text),
    };
    let key = format!("{enum_ident}::{variant}");
    let params = params.iter().map(|p| {
        let name = &p.name;
        let kind = param_kind(p.kind);
        quote!(::slopvac_messages::Param::new(#name, ::slopvac_messages::ParamKind::#kind))
    });
    quote! {
        ::slopvac_messages::Entry::new(
            #code,
            ::slopvac_messages::Kind::#kind,
            #key,
            #template,
            &[#(#params),*],
            #summary,
        )
    }
}

fn param_kind(kind: ParamKind) -> TokenStream {
    match kind {
        ParamKind::Text => quote!(Text),
        ParamKind::Count => quote!(Count),
        ParamKind::Path => quote!(Path),
    }
}

fn entry_arm(enum_ident: &Ident, messages: &[&Message], item: &Item) -> TokenStream {
    match item {
        Item::Message(m) => {
            let variant = &m.variant;
            let index = messages
                .iter()
                .position(|other| other.code == m.code)
                .expect("every message is in the list");
            quote!(#enum_ident::#variant { .. } => &ENTRIES[#index],)
        }
        Item::Delegate(d) => {
            let pattern = delegate_pattern(enum_ident, d);
            quote!(#pattern => ::slopvac_messages::Catalog::entry(__inner),)
        }
    }
}

fn delegate_pattern(enum_ident: &Ident, d: &Delegate) -> TokenStream {
    let variant = &d.variant;
    match &d.field {
        Wrapped::Positional => quote!(#enum_ident::#variant(__inner)),
        Wrapped::Named(field) => quote!(#enum_ident::#variant { #field: __inner }),
    }
}

fn args_arm(enum_ident: &Ident, item: &Item) -> TokenStream {
    match item {
        Item::Message(m) => {
            let variant = &m.variant;
            let fields = m.params.iter().map(|p| &p.field);
            let args = m.params.iter().map(|p| {
                let (name, field) = (&p.name, &p.field);
                let value = match p.kind {
                    ParamKind::Text => quote! {
                        ::slopvac_messages::Value::Text(::std::string::ToString::to_string(#field))
                    },
                    ParamKind::Count => quote! {
                        ::slopvac_messages::Value::Count(::slopvac_messages::ToCount::to_count(#field))
                    },
                    ParamKind::Path => quote! {
                        ::slopvac_messages::Value::Path(
                            ::std::path::Path::display(
                                ::core::convert::AsRef::<::std::path::Path>::as_ref(#field),
                            )
                            .to_string(),
                        )
                    },
                };
                quote!(::slopvac_messages::Arg::new(#name, #value))
            });
            quote!(#enum_ident::#variant { #(#fields,)* .. } => ::std::vec![#(#args),*],)
        }
        Item::Delegate(d) => {
            let pattern = delegate_pattern(enum_ident, d);
            quote!(#pattern => ::slopvac_messages::Catalog::args(__inner),)
        }
    }
}

/// `as_static` exists when no variant takes a parameter or wraps another catalog, so every
/// message is a fixed string (clap help, labels).
fn as_static(catalog: &Catalog, messages: &[&Message]) -> TokenStream {
    let fixed =
        messages.len() == catalog.items.len() && messages.iter().all(|m| m.params.is_empty());
    if !fixed {
        return TokenStream::new();
    }
    let ident = &catalog.ident;
    let arms = messages.iter().map(|m| {
        let variant = &m.variant;
        let text = template::unescape(&m.template);
        quote!(Self::#variant { .. } => #text,)
    });
    quote! {
        impl #ident {
            /// The message as a fixed string, for APIs that take `&'static str`.
            #[must_use]
            pub const fn as_static(&self) -> &'static str {
                match self { #(#arms)* }
            }
        }
    }
}
