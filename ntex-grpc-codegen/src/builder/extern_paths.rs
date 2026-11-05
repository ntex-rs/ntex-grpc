use std::collections::HashMap;

use itertools::Itertools;

use super::ident::{to_snake, to_upper_camel};

#[derive(Debug)]
pub(crate) struct ExternPaths {
    extern_paths: HashMap<String, String>,
}

impl ExternPaths {
    /// Maps the well-known types to `ntex_grpc::google_types` if `prost_types` is set.
    pub(crate) fn new(prost_types: bool) -> ExternPaths {
        let mut extern_paths = ExternPaths {
            extern_paths: HashMap::new(),
        };

        if prost_types {
            extern_paths.insert(
                ".google.protobuf".to_string(),
                "::ntex_grpc::google_types".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.Duration".to_string(),
                "::ntex_grpc::google_types::Duration".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.Timestamp".to_string(),
                "::ntex_grpc::google_types::Timestamp".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.BoolValue".to_string(),
                "::ntex_grpc::google_types::BoolValue".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.BytesValue".to_string(),
                "::ntex_grpc::google_types::BytesValue".into(),
            );
            extern_paths.insert(
                ".google.protobuf.DoubleValue".to_string(),
                "::ntex_grpc::google_types::DoubleValue".to_string(),
            );
            extern_paths.insert(".google.protobuf.Empty".to_string(), "()".to_string());
            extern_paths.insert(
                ".google.protobuf.FloatValue".to_string(),
                "::ntex_grpc::google_types::FloatValue".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.Int32Value".to_string(),
                "::ntex_grpc::google_types::Int32Value".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.Int64Value".to_string(),
                "::ntex_grpc::google_types::Int64Value".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.StringValue".to_string(),
                "::ntex_grpc::google_types::StringValue".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.UInt32Value".to_string(),
                "::ntex_grpc::google_types::UInt32Value".to_string(),
            );
            extern_paths.insert(
                ".google.protobuf.UInt64Value".to_string(),
                "::ntex_grpc::google_types::UInt64Value".to_string(),
            );
        }

        extern_paths
    }

    fn insert(&mut self, proto_path: String, rust_path: String) {
        self.extern_paths.insert(proto_path, rust_path);
    }

    pub(crate) fn is_extern_ident(&self, pb_ident: &str) -> bool {
        // protoc should always give fully qualified identifiers.
        assert_eq!(".", &pb_ident[..1]);

        self.extern_paths.contains_key(pb_ident)
    }

    pub(crate) fn resolve_ident(&self, pb_ident: &str) -> Option<String> {
        // protoc should always give fully qualified identifiers.
        assert_eq!(".", &pb_ident[..1]);

        if let Some(rust_path) = self.extern_paths.get(pb_ident) {
            return Some(rust_path.clone());
        }

        // TODO(danburkert): there must be a more efficient way to do this, maybe a trie?
        for (idx, _) in pb_ident.rmatch_indices('.') {
            if let Some(rust_path) = self.extern_paths.get(&pb_ident[..idx]) {
                let mut segments = pb_ident[idx + 1..].split('.');
                let ident_type = segments.next_back().map(to_upper_camel);

                return Some(
                    rust_path
                        .split("::")
                        .chain(segments)
                        .map(to_snake)
                        .chain(ident_type)
                        .join("::"),
                );
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// STATEMENT: Well-known types resolve to `ntex_grpc::google_types`, nested
    /// types through the `.google.protobuf` prefix, and other packages stay local.
    ///
    /// HOW:
    /// - Build `ExternPaths` with `prost_types` set.
    /// - Resolve an exact entry, `Empty`, a type covered only by the package
    ///   prefix and a nested type, and check the Rust paths.
    /// - Check that identifiers outside `.google.protobuf`, and any identifier
    ///   when `prost_types` is not set, do not resolve.
    #[test]
    fn test_well_known_types() {
        let paths = ExternPaths::new(true);

        let case = |proto_ident: &str, resolved_ident: &str| {
            assert_eq!(paths.resolve_ident(proto_ident).unwrap(), resolved_ident);
        };

        case(".google.protobuf.Value", "::ntex_grpc::google_types::Value");
        case(
            ".google.protobuf.Duration",
            "::ntex_grpc::google_types::Duration",
        );
        case(".google.protobuf.Empty", "()");
        case(
            ".google.protobuf.FooBar.Baz",
            "::ntex_grpc::google_types::foo_bar::Baz",
        );

        assert!(paths.resolve_ident(".google").is_none());
        assert!(paths.resolve_ident(".foo.Bar").is_none());
        assert!(
            ExternPaths::new(false)
                .resolve_ident(".google.protobuf.Value")
                .is_none()
        );
    }
}
