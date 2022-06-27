use ntex_prost_build::{Method, Service, ServiceGenerator};
use proc_macro2::TokenStream;
use quote::quote;

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
        buf.insert_str(
            0,
            "#![allow(dead_code)]\n/// DO NOT MODIFY. Auto-generated file\n\n",
        )
    }
}

fn generate_client(service: &Service, buf: &mut String) {
    let service_ident = quote::format_ident!("{}Client", service.name);
    let methods: Vec<_> = service
        .methods
        .iter()
        .map(|m| gen_method(m, service))
        .collect();
    let mut comments = service.comments.leading.clone();
    if comments.is_empty() {
        comments.push("".to_string());
    }

    let stream = quote! {
        #[doc = #(#comments)*]
        #[derive(Clone)]
        pub struct #service_ident<T>(T);

        impl<T> #service_ident<T> {
            #[inline]
            /// Create new client instance
            pub fn new(transport: T) -> Self {
                Self(transport)
            }
        }

        impl<T> ::ntex_grpc::ClientInformation<T> for #service_ident<T> {
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

        #(#methods)*
    };
    buf.push_str(&format!("{}", stream));
}

fn gen_method(method: &Method, service: &Service) -> TokenStream {
    let proto_name = &method.proto_name;
    let path = format!(
        "/{}.{}/{}",
        service.package, service.proto_name, method.proto_name
    );

    let service_ident = quote::format_ident!("{}Client", service.name);
    let method_ident = quote::format_ident!("{}", method.name);
    let def_ident = quote::format_ident!("{}{}Method", service.name, method.proto_name);
    let input_type = quote::format_ident!("{}", method.input_type);
    let output_type = quote::format_ident!("{}", method.output_type);
    let mut comments = method.comments.leading.clone();
    if comments.is_empty() {
        comments.push("".to_string());
    }

    quote! {
        #[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct #def_ident;

        impl ::ntex_grpc::MethodDef for #def_ident {
            const NAME: &'static str = #proto_name;
            const PATH: ::ntex_grpc::types::ByteString = ::ntex_grpc::types::ByteString::from_static(#path);
            type Input = #input_type;
            type Output = #output_type;
        }

        impl<T: ::ntex_grpc::Transport<#def_ident>> #service_ident<T> {
            #[doc = #(#comments)*]
            pub fn #method_ident<'a>(&'a self, req: &'a #input_type) -> ::ntex_grpc::Request<'a, T, #def_ident> {
                ::ntex_grpc::Request::new(&self.0, req)
            }
        }
    }
}
