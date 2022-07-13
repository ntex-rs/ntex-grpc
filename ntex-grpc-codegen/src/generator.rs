use ntex_prost_build::{Method, Service, ServiceGenerator};

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
}

fn generate_client(service: &Service, buf: &mut String) {
    let service_ident = service.name.to_string();
    let client_ident = format!("{}Client", service.name);
    let service_name = if service.package.is_empty() {
        service.proto_name.to_string()
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
            let name = m.proto_name.to_string();
            let m_name = format!("{}{}Method", service.proto_name, m.proto_name);
            format!(
                "{}::NAME => Some({}::{}({})),",
                m_name, service_methods_name, name, m_name
            )
        })
        .collect();
    service_methods_match.push("_ => None".to_string());
    let service_methods_match = service_methods_match.join("\n");

    let methods: Vec<_> = service
        .methods
        .iter()
        .map(|m| gen_method(m, service))
        .collect();
    let methods = methods.join("\n\n");

    let comments: Vec<_> = service
        .comments
        .leading
        .clone()
        .into_iter()
        .map(|s| format!("///{}", s))
        .collect();
    let comments = comments.join("");

    let stream = format!(
        "pub struct {};

        impl ::ntex_grpc::ServiceDef for {} {{
            const NAME: &'static str = \"{}\";
            type Methods = {};
        }}

        pub enum {} {{
            {}
        }}

        impl ::ntex_grpc::MethodsDef for {} {{
            #[inline]
            fn by_name(name: &str) -> Option<Self> {{
                use ::ntex_grpc::MethodDef;
                match name {{
                    {}
                }}
            }}
        }}

        #[derive(Clone)]
        {}
        pub struct {}<T>(T);

        impl<T> {}<T> {{
            #[inline]
            /// Create new client instance
            pub fn new(transport: T) -> Self {{
                Self(transport)
            }}
        }}

        impl<T> ::ntex_grpc::ClientInformation<T> for {}<T> {{
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

        {}",
        service_ident,
        service_ident,
        service_name,
        service_methods_name,
        service_methods_name,
        service_methods,
        service_methods_name,
        service_methods_match,
        comments,
        client_ident,
        client_ident,
        client_ident,
        methods,
    );
    buf.push_str(&stream);
}

fn gen_method(method: &Method, service: &Service) -> String {
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
    let method_ident = method.name.to_string();
    let def_ident = format!("{}{}Method", service.name, method.proto_name);
    let input_type = method.input_type.to_string();
    let output_type = method.output_type.to_string();
    let comments: Vec<_> = method
        .comments
        .leading
        .clone()
        .into_iter()
        .map(|s| format!("///{}", s))
        .collect();
    let comments = comments.join("");

    format!(
        "#[derive(Debug, Copy, Clone, PartialEq, Eq)]
        pub struct {};

        impl ::ntex_grpc::MethodDef for {} {{
            const NAME: &'static str = \"{}\";
            const PATH: ::ntex_grpc::ByteString = ::ntex_grpc::ByteString::from_static(\"{}\");
            type Input = {};
            type Output = {};
        }}

        impl<T: ::ntex_grpc::Transport<{}>> {}<T> {{
            {}
            pub fn {}<'a>(&'a self, req: &'a {}) -> ::ntex_grpc::Request<'a, T, {}> {{
                ::ntex_grpc::Request::new(&self.0, req)
            }}
        }}",
        def_ident,
        def_ident,
        proto_name,
        path,
        input_type,
        output_type,
        def_ident,
        service_ident,
        comments,
        method_ident,
        input_type,
        def_ident
    )
}
