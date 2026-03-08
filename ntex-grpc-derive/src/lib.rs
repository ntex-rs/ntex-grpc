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
        syn::Type::Path(tp) => {
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
                use ::ntex_grpc::MethodDef;
                let req = ::ntex_grpc::server::Request {
                    message: method.decode(&mut req.payload)?,
                    name: req.name,
                    headers: req.headers
                };

                let result = #ty::#fn_name(self, ::ntex_grpc::server::FromRequest::from(req)).await;

                let res = method.server_result(result);
                let response = ::ntex_grpc::server::Response::from(res);
                let mut buf = ::ntex_grpc::BytesMut::new();
                method.encode(response.message, &mut buf);

                Ok(::ntex_grpc::server::ServerResponse::with_headers(buf.freeze(), response.headers))
            }
        });
    }

    let service = quote::quote! {
        mod #modname {
            use super::*;

            impl ::ntex_grpc::Service<::ntex_grpc::server::ServerRequest> for #ty {
                type Response = ::ntex_grpc::server::ServerResponse;
                type Error = ::ntex_grpc::server::ServerError;

                async fn call(&self, mut req: ::ntex_grpc::server::ServerRequest, _: ::ntex_grpc::ServiceCtx<'_, Self>) -> Result<Self::Response, Self::Error> {
                    use ::ntex_grpc::{ServiceDef, MethodDef};

                    match #srvpath::method_by_name(&req.name) {
                        #(#methods)*
                        Some(_) => Err(::ntex_grpc::server::ServerError::new(
                            ::ntex_grpc::GrpcStatus::Unimplemented,
                            ::ntex_grpc::HeaderValue::from_shared(
                                ::ntex_grpc::ByteString::from(format!("Service method is not implemented: {0}", req.name)).into_bytes()
                            ).unwrap(),
                            None
                        )),
                        None => Err(::ntex_grpc::server::ServerError::new(
                            ::ntex_grpc::GrpcStatus::NotFound,
                            ::ntex_grpc::HeaderValue::from_shared(
                                ::ntex_grpc::ByteString::from(format!("Service method is not found: {0}", req.name)).into_bytes()
                            ).unwrap(),
                            None
                        ))
                    }
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
    fn fold_impl_item_fn(&mut self, mut m: syn::ImplItemFn) -> syn::ImplItemFn {
        for idx in 0..m.attrs.len() {
            let attr = &m.attrs[idx];
            if attr.path().is_ident("method") {
                let lst = if let syn::Meta::List(ref lst) = attr.meta {
                    lst
                } else {
                    panic!("{}", ERR_M_MESSAGE)
                };

                let name: syn::Path = lst.parse_args().expect(ERR_M_MESSAGE);
                let m_name = if let Some(ident) = name.get_ident() {
                    ident.clone()
                } else {
                    panic!("only simple identifiers are supported: {:?}", name);
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
