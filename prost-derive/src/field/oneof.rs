use proc_macro2::TokenStream;
use quote::quote;
use syn::Meta;

use crate::field::{set_option, tags_attr};

#[derive(Clone)]
pub struct Field {
    pub tags: Vec<u32>,
}

impl Field {
    pub fn new(attrs: &[Meta]) -> Option<Field> {
        let mut tags = None;
        let mut unknown_attrs = Vec::new();

        for attr in attrs {
            if attr.path().is_ident("oneof") {
                continue;
            } else if let Some(t) = tags_attr(attr).unwrap() {
                set_option(&mut tags, t, "duplicate tags attributes").unwrap();
            } else {
                unknown_attrs.push(attr);
            }
        }

        match unknown_attrs.len() {
            0 => (),
            1 => panic!(
                "unknown attribute for message field: {:?}",
                unknown_attrs[0]
            ),
            _ => panic!("unknown attributes for message field: {:?}", unknown_attrs),
        }

        let tags = match tags {
            Some(tags) => tags,
            None => panic!("oneof field is missing a tags attribute"),
        };

        Some(Field { tags })
    }

    /// Returns a statement which encodes the oneof field.
    pub fn encode(&self, ident: TokenStream) -> TokenStream {
        quote! {
            OneofType::encode(&#ident, buf);
        }
    }

    /// Returns an expression which evaluates to the encoded length of the oneof field.
    pub fn encoded_len(&self, ident: TokenStream) -> TokenStream {
        quote! {
            OneofType::encoded_len(&#ident)
        }
    }
}
