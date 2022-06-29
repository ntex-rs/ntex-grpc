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

        buf.push_str(&format!("\n/// `{}` service definition\n", service.name));
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
    let service_ident = quote::format_ident!("{}", service.name);
    let client_ident = quote::format_ident!("{}Client", service.name);
    let service_name = format!("{}.{}", service.package, service.proto_name);
    let service_methods_name = quote::format_ident!("{}Methods", service.name);
    let service_methods: Vec<_> = service
        .methods
        .iter()
        .map(|m| {
            let name = quote::format_ident!("{}", m.proto_name);
            let m_name = quote::format_ident!("{}{}Method", service.proto_name, m.proto_name);
            quote! {
                #name(#m_name)
            }
        })
        .collect();
    let mut service_methods_match: Vec<_> = service
        .methods
        .iter()
        .map(|m| {
            let name = quote::format_ident!("{}", m.proto_name);
            let m_name = quote::format_ident!("{}{}Method", service.proto_name, m.proto_name);
            quote! {
                #m_name::NAME => Some(#service_methods_name::#name(#m_name))
            }
        })
        .collect();
    service_methods_match.push(quote!(_ => None));

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

        pub struct #service_ident;

        impl ::ntex_grpc::ServiceDef for #service_ident {
            const NAME: &'static str = #service_name;

            type Methods = #service_methods_name;
        }

        pub enum #service_methods_name {
            #(#service_methods),*
        }
        impl ::ntex_grpc::MethodsDef for #service_methods_name {
            #[inline]
            fn by_name(name: &str) -> Option<Self> {
                use ::ntex_grpc::MethodDef;

                match name {
                    #(#service_methods_match),*
                }
            }
        }

        #[doc = #(#comments)*]
        #[derive(Clone)]
        pub struct #client_ident<T>(T);

        impl<T> #client_ident<T> {
            #[inline]
            /// Create new client instance
            pub fn new(transport: T) -> Self {
                Self(transport)
            }
        }

        impl<T> ::ntex_grpc::ClientInformation<T> for #client_ident<T> {
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
