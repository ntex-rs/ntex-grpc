use anyhow::Error;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Meta;

use crate::field::{set_option, tag_attr};

#[derive(Clone)]
pub struct Field {
    pub tag: u32,
}

impl Field {
    pub fn new(attrs: &[Meta], inferred_tag: Option<u32>) -> Result<Option<Field>, Error> {
        let mut tag = None;

        for attr in attrs {
            if let Some(t) = tag_attr(attr)? {
                set_option(&mut tag, t, "duplicate tag attributes")?;
            } else {
                return Ok(None);
            }
        }

        Ok(tag.or(inferred_tag).map(|tag| Field { tag }))
    }

    pub fn new_oneof(attrs: &[Meta]) -> Result<Option<Field>, Error> {
        Field::new(attrs, None)
    }

    /// Returns a statement which encodes the map field.
    pub fn encode(&self, ident: TokenStream) -> TokenStream {
        let tag = self.tag;
        quote!(NativeType::serialize(&#ident, #tag, buf);)
    }

    /// Returns an expression which evaluates to the encoded length of the map.
    pub fn encoded_len(&self, ident: TokenStream) -> TokenStream {
        let tag = self.tag;
        quote!(NativeType::field_len(&#ident, #tag))
    }
}
