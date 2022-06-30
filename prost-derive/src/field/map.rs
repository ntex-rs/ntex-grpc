use anyhow::{bail, Error};
use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Ident, Lit, Meta, MetaNameValue, NestedMeta};

use crate::field::{scalar, set_option, tag_attr};

#[derive(Clone, Debug)]
pub enum MapTy {
    HashMap,
    BTreeMap,
}

impl MapTy {
    fn from_str(s: &str) -> Option<MapTy> {
        match s {
            "map" | "hash_map" => Some(MapTy::HashMap),
            "btree_map" => Some(MapTy::BTreeMap),
            _ => None,
        }
    }

    fn lib(&self) -> TokenStream {
        match self {
            MapTy::HashMap => quote! { std },
            MapTy::BTreeMap => quote! { prost::alloc },
        }
    }
}

fn fake_scalar(ty: scalar::Ty) -> scalar::Field {
    let kind = scalar::Kind::Plain(scalar::DefaultValue::new(&ty));
    scalar::Field {
        ty,
        kind,
        tag: 0, // Not used here
    }
}

#[derive(Clone)]
pub struct Field {
    pub map_ty: MapTy,
    pub key_ty: scalar::Ty,
    pub value_ty: ValueTy,
    pub tag: u32,
}

impl Field {
    pub fn new(attrs: &[Meta], inferred_tag: Option<u32>) -> Result<Option<Field>, Error> {
        let mut types = None;
        let mut tag = None;

        for attr in attrs {
            if let Some(t) = tag_attr(attr)? {
                set_option(&mut tag, t, "duplicate tag attributes")?;
            } else if let Some(map_ty) = attr
                .path()
                .get_ident()
                .and_then(|i| MapTy::from_str(&i.to_string()))
            {
                let (k, v): (String, String) = match &*attr {
                    Meta::NameValue(MetaNameValue {
                        lit: Lit::Str(lit), ..
                    }) => {
                        let items = lit.value();
                        let mut items = items.split(',').map(ToString::to_string);
                        let k = items.next().unwrap();
                        let v = match items.next() {
                            Some(k) => k,
                            None => bail!("invalid map attribute: must have key and value types"),
                        };
                        if items.next().is_some() {
                            bail!("invalid map attribute: {:?}", attr);
                        }
                        (k, v)
                    }
                    Meta::List(meta_list) => {
                        // TODO(rustlang/rust#23121): slice pattern matching would make this much nicer.
                        if meta_list.nested.len() != 2 {
                            bail!("invalid map attribute: must contain key and value types");
                        }
                        let k = match &meta_list.nested[0] {
                            NestedMeta::Meta(Meta::Path(k)) if k.get_ident().is_some() => {
                                k.get_ident().unwrap().to_string()
                            }
                            _ => bail!("invalid map attribute: key must be an identifier"),
                        };
                        let v = match &meta_list.nested[1] {
                            NestedMeta::Meta(Meta::Path(v)) if v.get_ident().is_some() => {
                                v.get_ident().unwrap().to_string()
                            }
                            _ => bail!("invalid map attribute: value must be an identifier"),
                        };
                        (k, v)
                    }
                    _ => return Ok(None),
                };
                set_option(
                    &mut types,
                    (map_ty, key_ty_from_str(&k)?, ValueTy::from_str(&v)?),
                    "duplicate map type attribute",
                )?;
            } else {
                return Ok(None);
            }
        }

        Ok(match (types, tag.or(inferred_tag)) {
            (Some((map_ty, key_ty, value_ty)), Some(tag)) => Some(Field {
                map_ty,
                key_ty,
                value_ty,
                tag,
            }),
            _ => None,
        })
    }

    pub fn new_oneof(attrs: &[Meta]) -> Result<Option<Field>, Error> {
        Field::new(attrs, None)
    }

    /// Returns a statement which encodes the map field.
    pub fn encode(&self, ident: TokenStream) -> TokenStream {
        let tag = self.tag;
        quote!(NativeType::serialize_field(&#ident, #tag, buf);)
    }

    /// Returns an expression which evaluates to the encoded length of the map.
    pub fn encoded_len(&self, ident: TokenStream) -> TokenStream {
        let tag = self.tag;
        quote!(NativeType::field_len(&#ident, #tag))
    }

    /// Returns a newtype wrapper around the map, implementing nicer Debug
    ///
    /// The Debug tries to convert any enumerations met into the variants if possible, instead of
    /// outputting the raw numbers.
    pub fn debug(&self, wrapper_name: TokenStream) -> TokenStream {
        let type_name = match self.map_ty {
            MapTy::HashMap => Ident::new("HashMap", Span::call_site()),
            MapTy::BTreeMap => Ident::new("BTreeMap", Span::call_site()),
        };

        // A fake field for generating the debug wrapper
        let key_wrapper = fake_scalar(self.key_ty.clone()).debug(quote!(KeyWrapper));
        let key = self.key_ty.rust_type();
        let value_wrapper = self.value_ty.debug();
        let libname = self.map_ty.lib();
        let fmt = quote! {
            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                #key_wrapper
                #value_wrapper
                let mut builder = f.debug_map();
                for (k, v) in self.0 {
                    builder.entry(&KeyWrapper(k), &ValueWrapper(v));
                }
                builder.finish()
            }
        };
        match &self.value_ty {
            ValueTy::Scalar(ty) => {
                if let scalar::Ty::Bytes = *ty {
                    return quote! {
                        struct #wrapper_name<'a>(&'a dyn ::core::fmt::Debug);
                        impl<'a> ::core::fmt::Debug for #wrapper_name<'a> {
                            fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
                                self.0.fmt(f)
                            }
                        }
                    };
                }

                let value = ty.rust_type();
                quote! {
                    struct #wrapper_name<'a>(&'a ::#libname::collections::#type_name<#key, #value>);
                    impl<'a> ::core::fmt::Debug for #wrapper_name<'a> {
                        #fmt
                    }
                }
            }
            ValueTy::Message => quote! {
                struct #wrapper_name<'a, V: 'a>(&'a ::#libname::collections::#type_name<#key, V>);
                impl<'a, V> ::core::fmt::Debug for #wrapper_name<'a, V>
                where
                    V: ::core::fmt::Debug + 'a,
                {
                    #fmt
                }
            },
        }
    }
}

fn key_ty_from_str(s: &str) -> Result<scalar::Ty, Error> {
    let ty = scalar::Ty::from_str(s)?;
    match ty {
        scalar::Ty::Int32
        | scalar::Ty::Int64
        | scalar::Ty::Uint32
        | scalar::Ty::Uint64
        | scalar::Ty::Sint32
        | scalar::Ty::Sint64
        | scalar::Ty::Fixed32
        | scalar::Ty::Fixed64
        | scalar::Ty::Sfixed32
        | scalar::Ty::Sfixed64
        | scalar::Ty::Bool
        | scalar::Ty::String => Ok(ty),
        _ => bail!("invalid map key type: {}", s),
    }
}

/// A map value type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueTy {
    Scalar(scalar::Ty),
    Message,
}

impl ValueTy {
    fn from_str(s: &str) -> Result<ValueTy, Error> {
        if let Ok(ty) = scalar::Ty::from_str(s) {
            Ok(ValueTy::Scalar(ty))
        } else if s.trim() == "message" {
            Ok(ValueTy::Message)
        } else {
            bail!("invalid map value type: {}", s);
        }
    }

    /// Returns a newtype wrapper around the ValueTy for nicer debug.
    ///
    /// If the contained value is enumeration, it tries to convert it to the variant. If not, it
    /// just forwards the implementation.
    fn debug(&self) -> TokenStream {
        match self {
            ValueTy::Scalar(ty) => fake_scalar(ty.clone()).debug(quote!(ValueWrapper)),
            ValueTy::Message => quote!(
                fn ValueWrapper<T>(v: T) -> T {
                    v
                }
            ),
        }
    }
}
