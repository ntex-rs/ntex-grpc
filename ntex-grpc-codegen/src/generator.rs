use proc_macro2::TokenStream;
use prost_build::{Method, Service, ServiceGenerator};
use quote::quote;

fn snake_case(s: &str) -> String {
    let mut ident = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() {
            if !ident.is_empty() {
                ident.push('_');
            }
            for ch in ch.to_lowercase() {
                ident.push(ch);
            }
        } else {
            ident.push(ch);
        }
    }
    ident
}

#[derive(Debug, Copy, Clone)]
pub(crate) struct GrpcServiceGenerator;

impl ServiceGenerator for GrpcServiceGenerator {
    fn generate(&mut self, service: Service, buf: &mut String) {
        log::trace!(
            "Generate client for service: {:?}\n{:#?}",
            service.name,
            service
        );

        buf.push_str(&format!(
            "\n/// `{}` service client definition\n",
            service.name
        ));
        generate_client(&service, buf);
    }

    fn finalize(&mut self, buf: &mut String) {
        buf.insert_str(0, "/// DO NOT MODIFY. Auto-generated file\n\n\n")
    }
}

fn generate_client(service: &Service, buf: &mut String) {
    let mod_ident = quote::format_ident!("{}_client", snake_case(&service.name));
    let service_ident = quote::format_ident!("{}", service.name);
    let methods: Vec<_> = service.methods.iter().map(gen_method).collect();
    let m_defs: Vec<_> = service
        .methods
        .iter()
        .map(|m| gen_method_def(m, service))
        .collect();
    let comments = &service.comments.leading;

    let stream = quote! {
        pub mod #mod_ident {
            use super::*;
            use ntex_grpc::codegen as __ng;

            #[doc = #(#comments)*]
            #[derive(Clone)]
            pub struct #service_ident<T>(T);

            impl<T> __ng::ClientInformation<T> for #service_ident<T> {
                #[inline]
                /// Create new client instance
                fn create(transport: T) -> Self {
                    Self(transport)
                }

                #[inline]
                /// Get referece to underlying transport
                fn transport(&self) -> &T {
                    &self.0
                }

                #[inline]
                /// Get mut referece to underlying transport
                fn transport_mut(&mut self) -> &mut T {
                    &mut self.0
                }

                #[inline]
                /// Consume client and return inner transport
                fn into_inner(self) -> T {
                    self.0
                }
            }

            #(#m_defs)*

            impl<T: __ng::Transport> #service_ident<T>
            {
                #(#methods)*
            }
        }
    };
    buf.push_str(&format!("{}", stream));
}

fn gen_method_def(method: &Method, service: &Service) -> TokenStream {
    let def_ident = quote::format_ident!("{}Def", method.proto_name);
    let proto_name = &method.proto_name;
    let path = format!(
        "/{}.{}/{}",
        service.package, service.proto_name, method.proto_name
    );
    let input_type = quote::format_ident!("{}", method.input_type);
    let output_type = quote::format_ident!("{}", method.output_type);

    quote! {
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct #def_ident;

        impl __ng::MethodDef for #def_ident {
            const NAME: &'static str = #proto_name;
            const PATH: __ng::ByteString = __ng::ByteString::from_static(#path);
            type Input = #input_type;
            type Output = #output_type;
        }
    }
}

fn gen_method(method: &Method) -> TokenStream {
    let method_ident = quote::format_ident!("{}", method.name);
    let def_ident = quote::format_ident!("{}Def", method.proto_name);
    let input_type = quote::format_ident!("{}", method.input_type);
    let comments = &method.comments.leading;

    quote! {
        #[doc = #(#comments)*]
        pub fn #method_ident(&self, req: #input_type) -> __ng::Request<'_, T, #def_ident> {
            __ng::Request::new(&self.0, req)
        }
    }
}
