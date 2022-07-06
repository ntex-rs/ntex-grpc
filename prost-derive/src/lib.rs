use proc_macro::TokenStream;
use syn::{fold::Fold, parse::Parse, parse::ParseStream, punctuated::Punctuated};

const ERR_M_MESSAGE: &str = "invalid method definition, expected: #[method(name)]";

#[proc_macro_attribute]
pub fn server(attr: TokenStream, item: TokenStream) -> TokenStream {
    server_impl(attr, item)
}

fn server_impl(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut srv = syn::parse_macro_input!(attr as GrpcService);
    let input: syn::ItemImpl = syn::parse2(item.into()).unwrap();

    match input.self_ty.as_ref() {
        syn::Type::Path(ref tp) => {
            srv.self_ty = tp.path.clone();
            if let Some(s) = tp.path.segments.last() {
                srv.name = format!("{}", s.ident);
            } else {
                panic!("struct name is required");
            }
        }
        _ => panic!("struct impl block is supported only"),
    }

    let input = srv.fold_item_impl(input);

    let ty = srv.self_ty;
    let srvpath = srv.service;
    let srvname = srv.service_name;
    let srvmod = srv.service_mod;
    let modname = quote::format_ident!("_priv_{}", srv.name);
    let methods_prefix = quote::format_ident!("{}Methods", srvname);
    let mut methods_path = srvmod;
    methods_path.segments.push(methods_prefix.into());

    let mut methods = Vec::new();
    for (m_name, fn_name, span) in srv.methods {
        methods.push(quote::quote_spanned! {span=>
            Some(#methods_path::#m_name(method)) => {
                let input = method.decode(&mut req.payload)?;
                let output = #ty::#fn_name(&slf, input).await;
                method.encode(output, &mut buf);
                Ok(())
            }
        });
    }

    let service = quote::quote! {
        mod #modname {
            use super::*;

            impl ::ntex_grpc::server::Service<::ntex_grpc::server::Request> for #ty {
                type Response = ::ntex_grpc::server::Response;
                type Error = ::ntex_grpc::server::ServerError;
                type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response, Self::Error>>>>;

                #[inline]
                fn poll_ready(&self, _: &mut std::task::Context<'_>) -> std::task::Poll<Result<(), Self::Error>> {
                    std::task::Poll::Ready(Ok(()))
                }

                fn call(&self, mut req: ::ntex_grpc::server::Request) -> Self::Future {
                    use ::ntex_grpc::{ServiceDef, MethodDef};

                    let slf = self.clone();
                    Box::pin(async move {
                        let mut buf = ::ntex_grpc::types::BytesMut::new();

                        match #srvpath::method_by_name(&req.name) {
                            #(#methods)*
                            Some(_) => Err(::ntex_grpc::server::ServerError::NotImplemented(req.name)),
                            None => Err(::ntex_grpc::server::ServerError::NotFound(req.name)),
                        }?;
                        Ok(::ntex_grpc::server::Response::new(buf.freeze()))
                    })
                }
            }
        }
    };

    let tokens = quote::quote! {
        #input
        #service
    };
    tokens.into()
}

#[derive(Debug)]
struct GrpcService {
    name: String,
    self_ty: syn::Path,
    service: syn::Path,
    service_mod: syn::Path,
    service_name: syn::Ident,
    methods: Vec<(syn::Ident, syn::Ident, proc_macro2::Span)>,
}

impl Parse for GrpcService {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let parsed: Punctuated<syn::Path, syn::Token![,]> = Punctuated::parse_terminated(input)?;
        let path = parsed.first().unwrap().clone();
        let service = parsed.first().unwrap().clone();
        let mut service_mod = service.clone();
        service_mod.segments.pop();
        let service_name = path.segments.last().unwrap().ident.clone();
        Ok(GrpcService {
            service,
            service_mod,
            service_name,
            methods: Vec::new(),
            name: String::new(),
            self_ty: path,
        })
    }
}

impl Fold for GrpcService {
    fn fold_impl_item_method(&mut self, mut m: syn::ImplItemMethod) -> syn::ImplItemMethod {
        for idx in 0..m.attrs.len() {
            let attr = &m.attrs[idx];
            if attr.path.is_ident("method") {
                let args = attr.parse_meta().map_err(|_| ERR_M_MESSAGE).unwrap();
                let lst = if let syn::Meta::List(lst) = args {
                    lst
                } else {
                    panic!("{}", ERR_M_MESSAGE)
                };
                if lst.nested.len() != 1 {
                    panic!("{}", ERR_M_MESSAGE)
                }

                let m_name = match lst.nested[0] {
                    syn::NestedMeta::Meta(syn::Meta::Path(ref name)) => {
                        if let Some(name) = name.get_ident() {
                            name.clone()
                        } else {
                            panic!("only `Path` literals are supported: {:?}", lst.nested[0]);
                        }
                    }
                    _ => panic!("only `Path` literals are supported: {:?}", lst.nested[0]),
                };

                let _ = m.attrs.remove(idx);
                self.methods
                    .push((m_name, m.sig.ident.clone(), m.sig.fn_token.span));
                break;
            }
        }

        m
    }
}
