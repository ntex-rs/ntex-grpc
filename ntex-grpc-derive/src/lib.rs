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

    let errors = srv.errors.iter().map(syn::Error::to_compile_error);

    let mut methods = Vec::new();
    let mut streaming = Vec::new();
    for (m_name, fn_name, kind, span) in srv.methods {
        let body = match kind {
            Kind::Unary => quote::quote_spanned! {span=>
                use ::ntex_grpc::MethodDef;
                ::ntex_grpc::server::__private::check_kind::<_, false, false>(&method);
                let req = ::ntex_grpc::server::Request {
                    message: method.decode(&mut req.payload)?,
                    service: req.service,
                    name: req.name,
                    headers: req.headers
                };

                let result = #ty::#fn_name(self, ::ntex_grpc::server::FromRequest::from(req)).await;

                let res = method.server_result(result);
                let response = ::ntex_grpc::server::Response::from(res);
                let mut buf = ::ntex_grpc::BytePages::default();
                method.encode(response.message, &mut buf);

                Ok(::ntex_grpc::server::ServerResponse::with_headers(buf, response.headers))
            },
            Kind::ServerStreaming => quote::quote_spanned! {span=>
                ::ntex_grpc::server::__private::check_kind::<_, false, true>(&method);
                let mut streams = ::ntex_grpc::server::__private::streams(&mut req)?;
                let rx = ::ntex_grpc::server::__private::recv(&method, &mut streams);
                let tx = ::ntex_grpc::server::__private::send(&method, &mut streams);
                let req = ::ntex_grpc::server::Request {
                    message: rx.single().await?,
                    service: req.service,
                    name: req.name,
                    headers: req.headers
                };

                let result = #ty::#fn_name(self, ::ntex_grpc::server::FromRequest::from(req), tx).await;
                ::ntex_grpc::server::StreamResult::into_result(result)?;
                Ok(::ntex_grpc::server::ServerResponse::new(::ntex_grpc::BytePages::default()))
            },
            Kind::ClientStreaming => quote::quote_spanned! {span=>
                use ::ntex_grpc::MethodDef;
                ::ntex_grpc::server::__private::check_kind::<_, true, false>(&method);
                let mut streams = ::ntex_grpc::server::__private::streams(&mut req)?;
                let rx = ::ntex_grpc::server::__private::recv(&method, &mut streams);
                let tx = ::ntex_grpc::server::__private::send(&method, &mut streams);

                let result = #ty::#fn_name(self, rx).await;
                tx.send(&method.server_result(result)).await?;
                Ok(::ntex_grpc::server::ServerResponse::new(::ntex_grpc::BytePages::default()))
            },
            Kind::Bidi => quote::quote_spanned! {span=>
                ::ntex_grpc::server::__private::check_kind::<_, true, true>(&method);
                let mut streams = ::ntex_grpc::server::__private::streams(&mut req)?;
                let rx = ::ntex_grpc::server::__private::recv(&method, &mut streams);
                let tx = ::ntex_grpc::server::__private::send(&method, &mut streams);

                let result = #ty::#fn_name(self, rx, tx).await;
                ::ntex_grpc::server::StreamResult::into_result(result)?;
                Ok(::ntex_grpc::server::ServerResponse::new(::ntex_grpc::BytePages::default()))
            },
        };
        if kind != Kind::Unary {
            streaming.push(quote::quote_spanned! {span=>
                Some(#methods_path::#m_name(_)) => true,
            });
        }
        methods.push(quote::quote_spanned! {span=>
            Some(#methods_path::#m_name(method)) => {
                #body
            }
        });
    }

    let is_streaming = if streaming.is_empty() {
        quote::quote! {}
    } else {
        quote::quote! {
            fn is_streaming(method: &str) -> bool {
                use ::ntex_grpc::ServiceDef;
                match #srvpath::method_by_name(method) {
                    #(#streaming)*
                    _ => false,
                }
            }
        }
    };

    let service = quote::quote! {
        mod #modname {
            use super::*;

            impl ::ntex_grpc::server::NamedService for #ty {
                const NAME: &'static str = <#srvpath as ::ntex_grpc::ServiceDef>::NAME;

                #is_streaming
            }

            impl<St> ::ntex_grpc::Service<St, ::ntex_grpc::server::ServerRequest> for #ty {
                type Res = ::ntex_grpc::server::ServerResponse;
                type Error = ::ntex_grpc::server::ServerError;

                async fn call(&self, mut req: ::ntex_grpc::server::ServerRequest, _: ::ntex_grpc::Ctx<'_, Self, St>) -> Result<Self::Res, Self::Error> {
                    use ::ntex_grpc::{ServiceDef, MethodDef};

                    if &*req.service != <#srvpath as ServiceDef>::NAME {
                        return Err(::ntex_grpc::server::ServerError::new(
                            ::ntex_grpc::GrpcStatus::Unimplemented,
                            ::ntex_grpc::encode_grpc_message(&format!("Service is not found: {0}", req.service)),
                            None
                        ));
                    }
                    match #srvpath::method_by_name(&req.name) {
                        #(#methods)*
                        Some(_) => Err(::ntex_grpc::server::ServerError::new(
                            ::ntex_grpc::GrpcStatus::Unimplemented,
                            ::ntex_grpc::encode_grpc_message(&format!("Service method is not implemented: {0}", req.name)),
                            None
                        )),
                        None => Err(::ntex_grpc::server::ServerError::new(
                            ::ntex_grpc::GrpcStatus::Unimplemented,
                            ::ntex_grpc::encode_grpc_message(&format!("Service method is not found: {0}", req.name)),
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
        #(#errors)*
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
    methods: Vec<(syn::Ident, syn::Ident, Kind, proc_macro2::Span)>,
    errors: Vec<syn::Error>,
}

/// The kind of a method, inferred from its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// `(req)`
    Unary,
    /// `(req, tx: SendStream<_>)`
    ServerStreaming,
    /// `(rx: RecvStream<_>)`
    ClientStreaming,
    /// `(rx: RecvStream<_>, tx: SendStream<_>)`
    Bidi,
}

impl Kind {
    fn from_sig(sig: &syn::Signature) -> syn::Result<Kind> {
        let args: Vec<_> = sig
            .inputs
            .iter()
            .filter_map(|arg| match arg {
                syn::FnArg::Typed(arg) => Some(&*arg.ty),
                syn::FnArg::Receiver(_) => None,
            })
            .collect();

        match args.as_slice() {
            [rx] if is_type(rx, "RecvStream") => Ok(Kind::ClientStreaming),
            [req] if !is_type(req, "SendStream") => Ok(Kind::Unary),
            [rx, tx] if is_type(rx, "RecvStream") && is_type(tx, "SendStream") => Ok(Kind::Bidi),
            [req, tx] if !is_type(req, "RecvStream") && is_type(tx, "SendStream") => {
                Ok(Kind::ServerStreaming)
            }
            _ => Err(syn::Error::new_spanned(
                &sig.inputs,
                "expected method arguments: (req), (req, tx: SendStream<_>), \
                 (rx: RecvStream<_>) or (rx: RecvStream<_>, tx: SendStream<_>)",
            )),
        }
    }
}

/// Checks the last segment of a type path, e.g. `ntex_grpc::RecvStream<T>`.
fn is_type(ty: &syn::Type, name: &str) -> bool {
    match ty {
        syn::Type::Path(tp) => tp.path.segments.last().is_some_and(|s| s.ident == name),
        syn::Type::Paren(ty) => is_type(&ty.elem, name),
        syn::Type::Group(ty) => is_type(&ty.elem, name),
        _ => false,
    }
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
            errors: Vec::new(),
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
                match Kind::from_sig(&m.sig) {
                    Ok(kind) => {
                        self.methods
                            .push((m_name, m.sig.ident.clone(), kind, m.sig.fn_token.span))
                    }
                    Err(e) => self.errors.push(e),
                }
                break;
            }
        }

        m
    }
}
