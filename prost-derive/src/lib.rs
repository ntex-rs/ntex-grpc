#![doc(html_root_url = "https://docs.rs/ntex-prost-derive/0.10.3")]
// The `quote!` macro requires deep recursion.
#![recursion_limit = "4096"]

extern crate alloc;
extern crate proc_macro;

use anyhow::{bail, Error};
use itertools::Itertools;
use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::{
    punctuated::Punctuated, Data, DataEnum, DataStruct, DeriveInput, Expr, Fields, FieldsNamed,
    FieldsUnnamed, Ident, Variant,
};

mod field;
mod server;

use crate::field::Field;

#[proc_macro_derive(Message, attributes(prost))]
pub fn message(input: TokenStream) -> TokenStream {
    try_message(input).unwrap()
}

#[proc_macro_derive(Enumeration, attributes(prost))]
pub fn enumeration(input: TokenStream) -> TokenStream {
    try_enumeration(input).unwrap()
}

#[proc_macro_derive(Oneof, attributes(prost))]
pub fn oneof(input: TokenStream) -> TokenStream {
    try_oneof(input).unwrap()
}

#[proc_macro_attribute]
pub fn server(attr: TokenStream, item: TokenStream) -> TokenStream {
    server::server(attr, item)
}

fn try_message(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse(input)?;

    let ident = input.ident;

    let variant_data = match input.data {
        Data::Struct(variant_data) => variant_data,
        Data::Enum(..) => bail!("Message can not be derived for an enum"),
        Data::Union(..) => bail!("Message can not be derived for a union"),
    };

    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let fields = match variant_data {
        DataStruct {
            fields: Fields::Named(FieldsNamed { named: fields, .. }),
            ..
        }
        | DataStruct {
            fields:
                Fields::Unnamed(FieldsUnnamed {
                    unnamed: fields, ..
                }),
            ..
        } => fields.into_iter().collect(),
        DataStruct {
            fields: Fields::Unit,
            ..
        } => Vec::new(),
    };

    let mut next_tag: u32 = 1;
    let mut fields = fields
        .into_iter()
        .enumerate()
        .flat_map(|(idx, field)| {
            let field_ident = field
                .ident
                .unwrap_or_else(|| Ident::new(&idx.to_string(), Span::call_site()));
            match Field::new(field.attrs, Some(next_tag)) {
                Ok(Some(field)) => {
                    next_tag = field.tags().iter().max().map(|t| t + 1).unwrap_or(next_tag);
                    Some(Ok((field_ident, field)))
                }
                Ok(None) => None,
                Err(err) => Some(Err(
                    err.context(format!("invalid message field {}.{}", ident, field_ident))
                )),
            }
        })
        .collect::<Result<Vec<_>, _>>()?;

    // We want Debug to be in declaration order
    let unsorted_fields = fields.clone();

    // Sort the fields by tag number so that fields will be encoded in tag order.
    // TODO: This encodes oneof fields in the position of their lowest tag,
    // regardless of the currently occupied variant, is that consequential?
    // See: https://developers.google.com/protocol-buffers/docs/encoding#order
    fields.sort_by_key(|&(_, ref field)| field.tags().into_iter().min().unwrap());
    let fields = fields;

    let mut tags = fields
        .iter()
        .flat_map(|&(_, ref field)| field.tags())
        .collect::<Vec<_>>();
    let num_tags = tags.len();
    tags.sort_unstable();
    tags.dedup();
    if tags.len() != num_tags {
        bail!("message {} has fields with duplicate tags", ident);
    }

    let encoded_len = fields
        .iter()
        .map(|&(ref field_ident, ref field)| field.encoded_len(quote!(self.#field_ident)));

    let encode = fields
        .iter()
        .map(|&(ref field_ident, ref field)| field.encode(quote!(self.#field_ident)));

    let merge = fields.iter().map(|&(ref field_ident, ref field)| {
        let tags = field.tags().into_iter().map(|tag| quote!(#tag));
        let tags = Itertools::intersperse(tags, quote!(|));

        if field.is_oneof() {
            let val = field.merge(quote!(#field_ident));
            quote! {
                #(#tags)* => msg.#field_ident = #val.map_err(
                    |err| err.push(STRUCT_NAME, stringify!(#field_ident))
                )?,
            }
        } else {
            quote! {
                #(#tags)* => NativeType::deserialize(&mut msg.#field_ident, wire_type, buf)
                .map_err(|err| err.push(STRUCT_NAME, stringify!(#field_ident)))?,
            }
        }
    });

    let struct_name = if fields.is_empty() {
        quote!()
    } else {
        quote!(
            const STRUCT_NAME: &'static str = stringify!(#ident);
        )
    };

    let default = fields.iter().map(|&(ref field_ident, ref field)| {
        let value = field.default();
        quote!(#field_ident: #value,)
    });

    let debugs = unsorted_fields.iter().map(|&(ref field_ident, _)| {
        quote!(builder.field(stringify!(#field_ident), &self.#field_ident))
    });
    let debug_builder = quote!(f.debug_struct(stringify!(#ident)));

    let expanded = quote! {
        impl ::ntex_grpc::Message for #ident #ty_generics #where_clause {
            #[allow(unused_variables)]
            fn write(&self, buf: &mut ::ntex_grpc::types::BytesMut) {
                use ::ntex_grpc::NativeType;

                #(#encode)*
            }

            #[allow(unused_variables)]
            fn read(buf: &mut ::ntex_grpc::types::Bytes) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {
                use ::ntex_grpc::NativeType;

                #struct_name

                let mut msg = Self::default();

                while !buf.is_empty() {
                    let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(buf)?;

                    match tag {
                        #(#merge)*
                        _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, buf)?,
                    }
                }

                Ok(msg)
            }

            #[inline]
            fn encoded_len(&self) -> usize {
                use ::ntex_grpc::NativeType;

                0 #(+ #encoded_len)*
            }
        }

        impl #impl_generics ::std::default::Default for #ident #ty_generics #where_clause {
            fn default() -> Self {
                #ident {
                    #(#default)*
                }
            }
        }

        impl #impl_generics ::std::fmt::Debug for #ident #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                let mut builder = #debug_builder;
                #(#debugs;)*
                builder.finish()
            }
        }
    };

    Ok(expanded.into())
}

fn try_enumeration(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse(input)?;
    let ident = input.ident;

    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let punctuated_variants = match input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        Data::Struct(_) => bail!("Enumeration can not be derived for a struct"),
        Data::Union(..) => bail!("Enumeration can not be derived for a union"),
    };

    // Map the variants into 'fields'.
    let mut variants: Vec<(Ident, Expr)> = Vec::new();
    for Variant {
        ident,
        fields,
        discriminant,
        ..
    } in punctuated_variants
    {
        match fields {
            Fields::Unit => (),
            Fields::Named(_) | Fields::Unnamed(_) => {
                bail!("Enumeration variants may not have fields")
            }
        }

        match discriminant {
            Some((_, expr)) => variants.push((ident, expr)),
            None => bail!("Enumeration variants must have a disriminant"),
        }
    }

    if variants.is_empty() {
        panic!("Enumeration must have at least one variant");
    }

    let default = variants[0].0.clone();

    let is_valid = variants
        .iter()
        .map(|&(_, ref value)| quote!(#value => true));
    let from = variants.iter().map(
        |&(ref variant, ref value)| quote!(#value => ::std::option::Option::Some(#ident::#variant)),
    );

    let is_valid_doc = format!("Returns `true` if `value` is a variant of `{}`.", ident);
    let from_i32_doc = format!(
        "Converts an `i32` to a `{}`, or `None` if `value` is not a valid variant.",
        ident
    );

    let expanded = quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            #[doc=#is_valid_doc]
            pub fn is_valid(value: i32) -> bool {
                match value {
                    #(#is_valid,)*
                    _ => false,
                }
            }

            #[doc=#from_i32_doc]
            pub fn from_i32(value: i32) -> ::std::option::Option<#ident> {
                match value {
                    #(#from,)*
                    _ => ::std::option::Option::None,
                }
            }
        }

        impl #impl_generics ::std::default::Default for #ident #ty_generics #where_clause {
            fn default() -> #ident {
                #ident::#default
            }
        }

        impl #impl_generics ::std::convert::From::<#ident> for i32 #ty_generics #where_clause {
            fn from(value: #ident) -> i32 {
                value as i32
            }
        }
    };

    Ok(expanded.into())
}

fn try_oneof(input: TokenStream) -> Result<TokenStream, Error> {
    let input: DeriveInput = syn::parse(input)?;

    let ident = input.ident;

    let variants = match input.data {
        Data::Enum(DataEnum { variants, .. }) => variants,
        Data::Struct(..) => bail!("Oneof can not be derived for a struct"),
        Data::Union(..) => bail!("Oneof can not be derived for a union"),
    };

    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Map the variants into 'fields'.
    let mut fields: Vec<(Ident, Field)> = Vec::new();
    for Variant {
        attrs,
        ident: variant_ident,
        fields: variant_fields,
        ..
    } in variants
    {
        let variant_fields = match variant_fields {
            Fields::Unit => Punctuated::new(),
            Fields::Named(FieldsNamed { named: fields, .. })
            | Fields::Unnamed(FieldsUnnamed {
                unnamed: fields, ..
            }) => fields,
        };
        if variant_fields.len() != 1 {
            bail!("Oneof enum variants must have a single field");
        }
        match Field::new_oneof(attrs)? {
            Some(field) => fields.push((variant_ident, field)),
            None => bail!("invalid oneof variant: oneof variants may not be ignored"),
        }
    }

    let mut tags = fields
        .iter()
        .flat_map(|&(ref variant_ident, ref field)| -> Result<u32, Error> {
            if field.tags().len() > 1 {
                bail!(
                    "invalid oneof variant {}::{}: oneof variants may only have a single tag",
                    ident,
                    variant_ident
                );
            }
            Ok(field.tags()[0])
        })
        .collect::<Vec<_>>();
    tags.sort_unstable();
    tags.dedup();
    if tags.len() != fields.len() {
        panic!("invalid oneof {}: variants have duplicate tags", ident);
    }

    let encode = fields.iter().map(|&(ref variant_ident, ref field)| {
        let encode = field.encode(quote!(*value));
        quote!(#ident::#variant_ident(ref value) => { #encode })
    });

    let merge = fields.iter().map(|&(ref variant_ident, ref field)| {
        let tag = field.tags()[0];
        quote! {
            #tag => {
                #ident::#variant_ident(NativeType::deserialize_default(wire_type, buf)?)
            }
        }
    });

    let encoded_len = fields.iter().map(|&(ref variant_ident, ref field)| {
        let encoded_len = field.encoded_len(quote!(*value));
        quote!(#ident::#variant_ident(ref value) => #encoded_len)
    });

    let debug = fields.iter().map(|&(ref variant_ident, _)| {
        quote!(#ident::#variant_ident(ref value) => {
            f.debug_tuple(stringify!(#variant_ident))
                .field(value)
                .finish()
        })
    });

    let expanded = quote! {
        impl #impl_generics #ident #ty_generics #where_clause {
            /// Encodes the message to a buffer.
            pub fn encode(&self, buf: &mut ::ntex_grpc::types::BytesMut) {
                use ::ntex_grpc::NativeType;

                match *self {
                    #(#encode,)*
                }
            }

            /// Decodes an instance of the message from a buffer, and merges it into self.
            pub fn decode(tag: u32, wire_type: ::ntex_grpc::types::WireType, buf: &mut ::ntex_grpc::types::Bytes) -> ::std::result::Result<::std::option::Option<Self>, ::ntex_grpc::DecodeError> {
                use ::ntex_grpc::NativeType;

                Ok(Some(match tag {
                    #(#merge,)*
                    _ => unreachable!(concat!("invalid ", stringify!(#ident), " tag: {}"), tag),
                }))
            }

            /// Returns the encoded length of the message without a length delimiter.
            #[inline]
            pub fn encoded_len(&self) -> usize {
                use ::ntex_grpc::NativeType;

                match *self {
                    #(#encoded_len,)*
                }
            }
        }

        impl #impl_generics ::std::fmt::Debug for #ident #ty_generics #where_clause {
            fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
                match *self {
                    #(#debug,)*
                }
            }
        }
    };

    Ok(expanded.into())
}
