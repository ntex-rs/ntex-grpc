use std::fmt::Write;

use crate::builder::{Comments, Method, Service, ServiceGenerator};

#[derive(Debug, Copy, Clone)]
pub(crate) struct GrpcServiceGenerator;

impl ServiceGenerator for GrpcServiceGenerator {
    fn generate(&mut self, service: Service, buf: &mut String, priv_buf: &mut String) {
        log::trace!(
            "Generate client for service: {:?}\n{:#?}",
            service.name,
            service
        );

        writeln!(buf, "\n/// `{}` service definition", service.name).unwrap();
        generate_client(&service, buf, priv_buf);
    }
}

fn generate_client(service: &Service, buf: &mut String, priv_buf: &mut String) {
    let service_ident = service.name.clone();
    let client_ident = format!("{}Client", service.name);
    let service_name = if service.package.is_empty() {
        service.proto_name.clone()
    } else {
        format!("{}.{}", service.package, service.proto_name)
    };
    let service_methods_name = format!("{}Methods", service.name);

    let service_methods: Vec<_> = service
        .methods
        .iter()
        .map(|m| {
            format!(
                "{}({}{}Method),",
                m.proto_name, service.proto_name, m.proto_name
            )
        })
        .collect();
    let service_methods = service_methods.join("\n");

    let mut service_methods_match: Vec<_> = service
        .methods
        .iter()
        .map(|m| {
            let name = m.proto_name.clone();
            let m_name = format!("{}{}Method", service.proto_name, m.proto_name);
            format!("{m_name}::NAME => Some({service_methods_name}::{name}({m_name})),")
        })
        .collect();
    service_methods_match.push("_ => None".to_string());
    let service_methods_match = service_methods_match.join("\n");

    let all_methods: Vec<_> = service
        .methods
        .iter()
        .map(|m| gen_method(m, service))
        .collect();
    let methods = all_methods
        .clone()
        .into_iter()
        .map(|(b, _)| b)
        .collect::<Vec<_>>()
        .join("\n\n");
    let priv_methods = all_methods
        .into_iter()
        .map(|(_, p)| p)
        .collect::<Vec<_>>()
        .join("\n\n");

    let comments = doc_comments(&service.comments);

    let stream = format!(
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct {service_ident};

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum {service_methods_name} {{
            {service_methods}
        }}

        {comments}
        #[derive(Debug, Clone)]
        pub struct {client_ident}<T>(T);

        {methods}",
    );
    buf.push_str(&stream);

    let impl_stream = format!(
        "impl ::ntex_grpc::ServiceDef for {service_ident} {{
            const NAME: &'static str = \"{service_name}\";
            type Methods = {service_methods_name};

            #[inline]
            fn method_by_name(name: &str) -> Option<Self::Methods> {{
                use ::ntex_grpc::MethodDef;
                match name {{
                    {service_methods_match}
                }}
            }}
        }}

        impl<T> {client_ident}<T> {{
            #[inline]
            /// Create new client instance
            pub fn new(transport: T) -> Self {{
                Self(transport)
            }}
        }}

        impl<T> ::ntex_grpc::client::ClientInformation<T> for {client_ident}<T> {{
            #[inline]
            /// Create new client instance
            fn create(transport: T) -> Self {{
                Self(transport)
            }}

            #[inline]
            /// Get referece to underlying transport
            fn transport(&self) -> &T {{
                &self.0
            }}

            #[inline]
            /// Get mut referece to underlying transport
            fn transport_mut(&mut self) -> &mut T {{
                &mut self.0
            }}

            #[inline]
            /// Consume client and return inner transport
            fn into_inner(self) -> T {{
                self.0
            }}
        }}

        {priv_methods}"
    );
    priv_buf.push_str(&impl_stream);
}

fn gen_method(method: &Method, service: &Service) -> (String, String) {
    let proto_name = &method.proto_name;
    let path = if service.package.is_empty() {
        format!("/{}/{}", service.proto_name, method.proto_name)
    } else {
        format!(
            "/{}.{}/{}",
            service.package, service.proto_name, method.proto_name
        )
    };

    let service_ident = format!("{}Client", service.name);
    let method_ident = method.name.clone();
    let def_ident = format!("{}{}Method", service.name, method.proto_name);
    let input_type = method.input_type.clone();
    let req_input_type = if method.input_type_extern {
        method.input_type.clone()
    } else {
        format!("super::{}", method.input_type)
    };
    let output_type = method.output_type.clone();
    let comments = doc_comments(&method.comments);

    let streaming = format!(
        "const CLIENT_STREAMING: bool = {};
            const SERVER_STREAMING: bool = {};",
        method.client_streaming, method.server_streaming
    );
    let client_method = match (method.client_streaming, method.server_streaming) {
        (false, false) => format!(
            "pub fn {method_ident}<'a>(&'a self, req: &'a {req_input_type}) -> ::ntex_grpc::client::Request<'a, T, {def_ident}> {{
                ::ntex_grpc::client::Request::new(&self.0, req)
            }}"
        ),
        (false, true) => format!(
            "pub fn {method_ident}<'a>(&'a self, req: &'a {req_input_type}) -> ::ntex_grpc::client::StreamRequest<'a, T, {def_ident}, ::ntex_grpc::client::ServerStreaming<'a, {req_input_type}>> {{
                ::ntex_grpc::client::StreamRequest::server_streaming(&self.0, req)
            }}"
        ),
        (true, false) => format!(
            "pub fn {method_ident}(&self, tx: &::ntex_grpc::SendStream<{req_input_type}>) -> ::ntex_grpc::client::StreamRequest<'_, T, {def_ident}, ::ntex_grpc::client::ClientStreaming> {{
                ::ntex_grpc::client::StreamRequest::client_streaming(&self.0, tx)
            }}"
        ),
        (true, true) => format!(
            "pub fn {method_ident}(&self) -> ::ntex_grpc::client::StreamRequest<'_, T, {def_ident}, ::ntex_grpc::client::Bidi> {{
                ::ntex_grpc::client::StreamRequest::bidi(&self.0)
            }}"
        ),
    };

    (
        format!(
            "#[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct {def_ident};

        impl ::ntex_grpc::MethodDef for {def_ident} {{
            const NAME: &'static str = \"{proto_name}\";
            const PATH: ::ntex_grpc::ByteString = ::ntex_grpc::ByteString::from_static(\"{path}\");
            type Input = {input_type};
            type Output = {output_type};
            {streaming}
        }}"
        ),
        format!(
            "impl<T> {service_ident}<T> {{
            {comments}
            {client_method}
        }}"
        ),
    )
}

/// Proto comments as rustdoc, one `///` line per comment line.
fn doc_comments(comments: &Comments) -> String {
    let mut buf = String::new();
    comments.append_with_indent(0, &mut buf);
    buf.truncate(buf.trim_end().len());
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comments(lines: &[&str]) -> Comments {
        Comments {
            leading_detached: Vec::new(),
            leading: lines.iter().map(|s| (*s).to_string()).collect(),
            trailing: Vec::new(),
        }
    }

    #[test]
    fn multiline_comments() {
        let method = Method {
            name: "hello".into(),
            proto_name: "Hello".into(),
            comments: comments(&[" Says hello.", " Second line.", "", "     example"]),
            input_type: "Req".into(),
            output_type: "Req".into(),
            input_type_extern: false,
            client_streaming: false,
            server_streaming: false,
        };
        let service = Service {
            name: "Demo".into(),
            proto_name: "Demo".into(),
            package: "demo".into(),
            comments: comments(&[" The demo service.", "", " Second paragraph."]),
            methods: vec![method],
        };

        let (mut buf, mut priv_buf) = (String::new(), String::new());
        generate_client(&service, &mut buf, &mut priv_buf);
        let all = format!("{buf}{priv_buf}");
        let docs: Vec<_> = all
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with("///"))
            .collect();

        for lines in [
            &["///  The demo service.", "///", "///  Second paragraph."][..],
            &[
                "///  Says hello.",
                "///  Second line.",
                "///",
                "///  ```text",
                "///  example",
                "///  ```",
            ][..],
        ] {
            assert!(docs.windows(lines.len()).any(|w| w == lines), "{docs:#?}");
        }
    }

    #[test]
    fn streaming_methods() {
        let method = |name: &str, client_streaming, server_streaming| Method {
            name: name.to_lowercase(),
            proto_name: name.into(),
            comments: comments(&[]),
            input_type: "Req".into(),
            output_type: "Res".into(),
            input_type_extern: false,
            client_streaming,
            server_streaming,
        };
        let service = Service {
            name: "Demo".into(),
            proto_name: "Demo".into(),
            package: "demo".into(),
            comments: comments(&[]),
            methods: vec![
                method("Unary", false, false),
                method("Server", false, true),
                method("Client", true, false),
                method("Bidi", true, true),
            ],
        };

        let (mut buf, mut priv_buf) = (String::new(), String::new());
        generate_client(&service, &mut buf, &mut priv_buf);
        let all = format!("{buf}{priv_buf}")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");

        for code in [
            "pub fn unary<'a>(&'a self, req: &'a super::Req) -> ::ntex_grpc::client::Request<'a, T, DemoUnaryMethod>",
            "pub fn server<'a>(&'a self, req: &'a super::Req) -> ::ntex_grpc::client::StreamRequest<'a, T, DemoServerMethod, ::ntex_grpc::client::ServerStreaming<'a, super::Req>>",
            "pub fn client(&self, tx: &::ntex_grpc::SendStream<super::Req>) -> ::ntex_grpc::client::StreamRequest<'_, T, DemoClientMethod, ::ntex_grpc::client::ClientStreaming>",
            "pub fn bidi(&self) -> ::ntex_grpc::client::StreamRequest<'_, T, DemoBidiMethod, ::ntex_grpc::client::Bidi>",
            "const NAME: &'static str = \"Unary\"; const PATH: ::ntex_grpc::ByteString = ::ntex_grpc::ByteString::from_static(\"/demo.Demo/Unary\"); type Input = Req; type Output = Res; const CLIENT_STREAMING: bool = false; const SERVER_STREAMING: bool = false;",
            "const CLIENT_STREAMING: bool = false; const SERVER_STREAMING: bool = true;",
            "const CLIENT_STREAMING: bool = true; const SERVER_STREAMING: bool = false;",
            "const CLIENT_STREAMING: bool = true; const SERVER_STREAMING: bool = true;",
        ] {
            assert!(all.contains(code), "{code}\n{all}");
        }
    }
}
