#![allow(clippy::option_as_ref_deref)]
mod ast;
mod code_generator;
mod extern_paths;
mod ident;
mod path;

use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::{collections::HashMap, default, env, fmt, fs, process::Command};

use log::trace;
use prost::Message;
use prost_types::{FileDescriptorProto, FileDescriptorSet};

pub(crate) use self::ast::{Comments, Method, Service};
use self::code_generator::{CodeGenerator, enum_values};
use self::extern_paths::ExternPaths;
use self::ident::to_snake;
use self::path::PathMap;

/// A service generator takes a service descriptor and generates Rust code.
///
/// Service generators are registered using `Config::service_generator`.
pub(crate) trait ServiceGenerator {
    /// Generates a Rust interface or implementation for a service, writing the
    /// result to `buf`.
    fn generate(&mut self, service: Service, buf: &mut String, priv_buf: &mut String);

    /// Finalizes generation for a `.proto` file.
    fn finalize(&mut self, _buf: &mut String) {}

    /// Finalizes generation for an entire protobuf package.
    fn finalize_package(&mut self, _package: &str, _buf: &mut String) {}
}

/// Configuration options for Protobuf code generation.
pub(crate) struct Config {
    service_generator: Option<Box<dyn ServiceGenerator>>,
    types_map: PathMap<String>,
    prost_types: bool,
    optional_messages: bool,
    open_enums: bool,
    out_dir: Option<PathBuf>,
    default_package_filename: String,
}

impl Config {
    /// Configure Rust types for matching Protobuf fields.
    ///
    /// Types must implement `NativeType`. Paths use Protobuf names, before
    /// conversion to Rust naming conventions. A leading `.` makes a path fully
    /// qualified; other paths match suffixes. Package paths match their fields,
    /// and `.` matches all fields.
    pub(crate) fn map_field_type<I, S>(&mut self, paths: I, tp: &str) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for matcher in paths {
            self.types_map
                .insert(matcher.as_ref().to_string(), tp.to_string());
        }
        self
    }

    /// Configures the code generator to use the provided service generator.
    pub(crate) fn service_generator(
        &mut self,
        service_generator: Box<dyn ServiceGenerator>,
    ) -> &mut Self {
        self.service_generator = Some(service_generator);
        self
    }

    /// Generates well-known types from their `.proto` definitions instead of
    /// using `ntex_grpc::google_types`.
    pub(crate) fn compile_well_known_types(&mut self) -> &mut Self {
        self.prost_types = false;
        self
    }

    /// Configures the code generator to generate singular message fields as `Option<T>`.
    ///
    /// By default a message field is a plain value: presence is not tracked and the field
    /// is always encoded, as an empty message when it holds the default. With this option
    /// an unset field is `None` and is not encoded. Recursive message fields are always
    /// generated as `Option<Box<T>>`.
    pub(crate) fn optional_messages(&mut self, enabled: bool) -> &mut Self {
        self.optional_messages = enabled;
        self
    }

    /// Configures the code generator to generate enum fields as `i32`.
    ///
    /// By default an enum field has the generated enum type, and a value unknown to the
    /// generated enum is decoded as the default variant. With this option the field
    /// keeps the raw value, typed accessors (`field()`/`set_field()`, `push_field()`,
    /// `get_field()`/`insert_field()` for maps) convert it to the generated enum.
    pub(crate) fn open_enums(&mut self, enabled: bool) -> &mut Self {
        self.open_enums = enabled;
        self
    }

    /// Configures the output directory where generated Rust files will be written.
    ///
    /// If unset, defaults to the `OUT_DIR` environment variable.
    pub(crate) fn out_dir<P>(&mut self, path: P) -> &mut Self
    where
        P: Into<PathBuf>,
    {
        self.out_dir = Some(path.into());
        self
    }

    /// Configures what filename protobufs with no package definition are written to.
    pub(crate) fn default_package_filename<S>(&mut self, filename: S) -> &mut Self
    where
        S: Into<String>,
    {
        self.default_package_filename = filename.into();
        self
    }

    /// Compiles `.proto` files and their imports into Rust files in the output directory.
    pub(crate) fn compile_protos(
        &mut self,
        protos: &[impl AsRef<Path>],
        includes: &[impl AsRef<Path>],
    ) -> Result<()> {
        let target: PathBuf = self.out_dir.clone().map_or_else(
            || {
                env::var_os("OUT_DIR")
                    .ok_or_else(|| Error::other("OUT_DIR environment variable is not set"))
                    .map(Into::into)
            },
            Ok,
        )?;

        let tmp = tempfile::Builder::new().prefix("prost-build").tempdir()?;
        let file_descriptor_set_path = tmp.path().join("prost-descriptor-set");

        let mut cmd = Command::new(protoc());
        cmd.arg("--include_imports")
            .arg("--include_source_info")
            .arg("-o")
            .arg(&file_descriptor_set_path);

        for include in includes {
            cmd.arg("-I").arg(include.as_ref());
        }

        // User includes take precedence over the built-in .protos.
        cmd.arg("-I").arg(protoc_include());

        for proto in protos {
            cmd.arg(proto.as_ref());
        }

        let output = cmd.output().map_err(|error| {
            Error::new(
                error.kind(),
                format!("failed to invoke protoc (hint: https://docs.rs/prost-build/#sourcing-protoc): {error}"),
            )
        })?;

        if !output.status.success() {
            return Err(Error::other(format!(
                "protoc failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        let buf = fs::read(file_descriptor_set_path)?;
        let file_descriptor_set = FileDescriptorSet::decode(&*buf).map_err(|error| {
            Error::new(
                ErrorKind::InvalidInput,
                format!("invalid FileDescriptorSet: {error}"),
            )
        })?;

        let requests = file_descriptor_set
            .file
            .into_iter()
            .map(|descriptor| {
                (
                    Module::from_protobuf_package_name(descriptor.package()),
                    descriptor,
                )
            })
            .collect::<Vec<_>>();

        let file_names = requests
            .iter()
            .map(|req| {
                (
                    req.0.clone(),
                    req.0.to_file_name_or(&self.default_package_filename),
                )
            })
            .collect::<HashMap<Module, String>>();

        let modules = self.generate(requests)?;

        for (module, content) in &modules {
            let mut file_name = file_names
                .get(module)
                .expect("every module should have a filename");
            if file_name == "google.protobuf.rs" || file_name == "google_protobuf.rs" {
                if self.prost_types {
                    continue;
                }
                file_name = &self.default_package_filename;
            }
            let output_path = target.join(file_name);
            let previous_content = fs::read(&output_path);

            if previous_content
                .is_ok_and(|previous_content| previous_content == content.as_bytes())
            {
                trace!("unchanged: {file_name:?}");
            } else {
                trace!("writing: {file_name:?}");
                fs::write(output_path, content)?;
            }
        }

        Ok(())
    }

    /// Processes file descriptors, returning generated code grouped by module.
    pub(crate) fn generate(
        &mut self,
        requests: Vec<(Module, FileDescriptorProto)>,
    ) -> Result<HashMap<Module, String>> {
        let mut modules = HashMap::new();
        let mut packages = HashMap::new();

        let extern_paths = ExternPaths::new(&[], self.prost_types)
            .map_err(|error| Error::new(ErrorKind::InvalidInput, error))?;
        let enums = enum_values(requests.iter().map(|request| &request.1));

        for request in requests {
            // Only record packages that have services
            if !request.1.service.is_empty() {
                packages.insert(request.0.clone(), request.1.package().to_string());
            }

            if !modules.contains_key(&request.0) {
                let mut buf = String::new();
                buf.insert_str(
                    0,
                    "#![allow(dead_code, unused_mut, unused_variables, clippy::identity_op, clippy::too_many_lines, clippy::derivable_impls, clippy::unit_arg, clippy::derive_partial_eq_without_eq, clippy::manual_range_patterns, clippy::default_trait_access, clippy::semicolon_if_nothing_returned, clippy::doc_markdown, clippy::wildcard_imports)]\n//! DO NOT MODIFY. Auto-generated file\n\n",
                );
                modules.insert(request.0.clone(), buf);
            }
            let buf = modules.get_mut(&request.0).unwrap();

            CodeGenerator::generate(self, &extern_paths, &enums, request.1, buf);
        }

        if let Some(ref mut service_generator) = self.service_generator {
            for (module, package) in packages {
                let buf = modules.get_mut(&module).unwrap();
                service_generator.finalize_package(&package, buf);
            }
        }

        Ok(modules)
    }
}

impl default::Default for Config {
    fn default() -> Config {
        Config {
            service_generator: None,
            types_map: PathMap::default(),
            prost_types: true,
            optional_messages: false,
            open_enums: false,
            out_dir: None,
            default_package_filename: "_".to_string(),
        }
    }
}

impl fmt::Debug for Config {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Config")
            .field("service_generator", &self.service_generator.is_some())
            .field("types_map", &self.types_map)
            .field("prost_types", &self.prost_types)
            .field("optional_messages", &self.optional_messages)
            .field("open_enums", &self.open_enums)
            .field("out_dir", &self.out_dir)
            .field("default_package_filename", &self.default_package_filename)
            .finish()
    }
}

/// A Rust module path for a Protobuf package.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Module {
    components: Vec<String>,
}

impl Module {
    /// Construct a module path from a Protobuf package name.
    ///
    /// Constituent parts are automatically converted to snake case in order to follow
    /// Rust module naming conventions.
    pub(crate) fn from_protobuf_package_name(name: &str) -> Self {
        Self {
            components: name
                .split('.')
                .filter(|s| !s.is_empty())
                .map(to_snake)
                .collect(),
        }
    }

    /// Format the module path into a filename for generated Rust code.
    ///
    /// If the module path is empty, `default` is used to provide the root of the filename.
    /// Dots are replaced with underscores, keeping a trailing `.rs` extension.
    pub(crate) fn to_file_name_or(&self, default: &str) -> String {
        let root = if self.components.is_empty() {
            default.to_owned()
        } else {
            self.components.join(".")
        };

        let mut root = root.strip_suffix(".rs").unwrap_or(&root).replace('.', "_");
        root.push_str(".rs");

        root
    }
}

/// Returns the path to the `protoc` binary.
pub(crate) fn protoc() -> PathBuf {
    match env::var_os("PROTOC") {
        Some(protoc) => PathBuf::from(protoc),
        None => PathBuf::from(env!("PROTOC")),
    }
}

/// Returns the path to the Protobuf include directory.
pub(crate) fn protoc_include() -> PathBuf {
    match env::var_os("PROTOC_INCLUDE") {
        Some(include) => PathBuf::from(include),
        None => PathBuf::from(env!("PROTOC_INCLUDE")),
    }
}

#[cfg(test)]
mod tests {
    use super::Module;

    #[test]
    fn test_module_file_name() {
        for (package, default, expected) in [
            ("", "fallback", "fallback.rs"),
            ("", "fallback.rs", "fallback.rs"),
            ("", "fallback.RS", "fallback_RS.rs"),
            ("", "foo.bar.rs", "foo_bar.rs"),
            ("", "foo.rs.rs", "foo_rs.rs"),
            ("", "", ".rs"),
            ("", ".rs", ".rs"),
            ("", ".", "_.rs"),
            ("", "foo.rs/child", "foo_rs/child.rs"),
            ("foo.Bar", "ignored.rs", "foo_bar.rs"),
            (".google.protobuf", "ignored.rs", "google_protobuf.rs"),
            ("foo.rs", "ignored.rs", "foo.rs"),
        ] {
            assert_eq!(
                Module::from_protobuf_package_name(package).to_file_name_or(default),
                expected,
                "package={package:?}, default={default:?}"
            );
        }
    }
}
