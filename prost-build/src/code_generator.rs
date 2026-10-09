use std::{collections::HashMap, collections::HashSet, iter};

use heck::ToSnekCase;
use itertools::{Either, Itertools};
use log::debug;
use multimap::MultiMap;
use prost_types::field_descriptor_proto::{Label, Type};
use prost_types::source_code_info::Location;
use prost_types::{
    DescriptorProto, EnumDescriptorProto, EnumValueDescriptorProto, FieldDescriptorProto,
    FileDescriptorProto, OneofDescriptorProto, ServiceDescriptorProto, SourceCodeInfo,
};

use crate::ast::{Comments, Method, Service};
use crate::ident::{to_snake, to_upper_camel};
use crate::{extern_paths::ExternPaths, Config};

#[derive(PartialEq)]
enum Syntax {
    Proto2,
    Proto3,
}

pub struct CodeGenerator<'a> {
    config: &'a mut Config,
    name: String,
    package: String,
    source_info: SourceCodeInfo,
    syntax: Syntax,
    extern_paths: &'a ExternPaths,
    depth: u8,
    path: Vec<i32>,
    buf: &'a mut String,
    priv_buf: String,
    /// Fields that are part of a message cycle and are stored in a `Box`,
    /// `(message, field)` with message names without the leading dot
    boxed: HashSet<(String, String)>,
}

/// Singular message fields that lead back to their own message, directly or
/// through other messages. Such fields need a `Box`, otherwise the generated
/// struct has infinite size. Proto imports cannot be cyclic, so a cycle never
/// leaves the file.
fn boxed_fields(package: &str, messages: &[DescriptorProto]) -> HashSet<(String, String)> {
    fn collect(
        prefix: &str,
        messages: &[DescriptorProto],
        edges: &mut HashMap<String, Vec<(String, String)>>,
    ) {
        for msg in messages {
            let name = if prefix.is_empty() {
                msg.name().to_string()
            } else {
                format!("{prefix}.{}", msg.name())
            };
            let fields = msg
                .field
                .iter()
                .filter(|f| {
                    f.label() != Label::Repeated
                        && matches!(f.r#type(), Type::Message | Type::Group)
                })
                .map(|f| {
                    let ty = f.type_name().trim_start_matches('.').to_string();
                    (f.name().to_string(), ty)
                })
                .collect();
            edges.insert(name.clone(), fields);
            collect(&name, &msg.nested_type, edges);
        }
    }

    fn reaches(
        from: &str,
        to: &str,
        edges: &HashMap<String, Vec<(String, String)>>,
        seen: &mut HashSet<String>,
    ) -> bool {
        if from == to {
            return true;
        }
        if !seen.insert(from.to_string()) {
            return false;
        }
        edges
            .get(from)
            .is_some_and(|fields| fields.iter().any(|(_, ty)| reaches(ty, to, edges, seen)))
    }

    let mut edges = HashMap::new();
    collect(package, messages, &mut edges);

    let mut boxed = HashSet::new();
    for (msg, fields) in &edges {
        for (field, ty) in fields {
            if reaches(ty, msg, &edges, &mut HashSet::new()) {
                boxed.insert((msg.clone(), field.clone()));
            }
        }
    }
    boxed
}

fn push_indent(buf: &mut String, depth: u8) {
    for _ in 0..depth {
        buf.push_str("    ");
    }
}

impl CodeGenerator<'_> {
    pub fn generate(
        config: &mut Config,
        extern_paths: &ExternPaths,
        file: FileDescriptorProto,
        buf: &mut String,
    ) {
        let name = file
            .name
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("")
            .strip_suffix(".proto")
            .unwrap_or("")
            .replace(".", "_")
            .replace("-", "_")
            .replace("/", "_");

        let mut source_info = file
            .source_code_info
            .expect("no source code info in request");
        source_info.location.retain(|location| {
            let len = location.path.len();
            len > 0 && len % 2 == 0
        });
        source_info
            .location
            .sort_by_key(|location| location.path.clone());

        let syntax = match file.syntax.as_ref().map(String::as_str) {
            None | Some("proto2") => Syntax::Proto2,
            Some("proto3") => Syntax::Proto3,
            Some(s) => panic!("unknown syntax: {s}"),
        };

        let boxed = boxed_fields(file.package.as_deref().unwrap_or(""), &file.message_type);

        let mut code_gen = CodeGenerator {
            boxed,
            name,
            config,
            source_info,
            syntax,
            extern_paths,
            buf,
            depth: 0,
            path: Vec::new(),
            priv_buf: String::new(),
            package: file.package.unwrap_or_default(),
        };

        debug!(
            "file: {:?}, package: {:?}",
            file.name.as_ref().unwrap(),
            code_gen.package
        );

        code_gen.path.push(4);
        for (idx, message) in file.message_type.into_iter().enumerate() {
            code_gen.path.push(idx as i32);
            code_gen.append_message(message);
            code_gen.path.pop();
        }
        code_gen.path.pop();

        code_gen.path.push(5);
        for (idx, desc) in file.enum_type.into_iter().enumerate() {
            code_gen.path.push(idx as i32);
            code_gen.append_enum(to_upper_camel(desc.name()), desc);
            code_gen.path.pop();
        }
        code_gen.path.pop();

        if code_gen.config.service_generator.is_some() {
            code_gen.path.push(6);
            for (idx, service) in file.service.into_iter().enumerate() {
                code_gen.path.push(idx as i32);
                code_gen.push_service(service);
                code_gen.path.pop();
            }

            if let Some(service_generator) = code_gen.config.service_generator.as_mut() {
                service_generator.finalize(code_gen.buf);
            }

            code_gen.path.pop();
        }

        code_gen.buf.push_str("\n\n\n");

        code_gen
            .buf
            .push_str(&format!("mod _priv_impl_{} {{\n", code_gen.name));
        code_gen.buf.push_str("use super::*;\n\n");
        code_gen.buf.push_str(&code_gen.priv_buf);
        code_gen.buf.push('}');
    }

    fn append_message(&mut self, message: DescriptorProto) {
        debug!("  message: {:?}", message.name());

        let message_name = message.name().to_string();
        let fq_message_name = format!(
            "{}{}.{}",
            if self.package.is_empty() { "" } else { "." },
            self.package,
            message.name()
        );

        // Skip external types.
        if self.extern_paths.resolve_ident(&fq_message_name).is_some() {
            return;
        }

        // Split the nested message types into a vector of normal nested message types, and a map
        // of the map field entry types. The path index of the nested message types is preserved so
        // that comments can be retrieved.
        type NestedTypes = Vec<(DescriptorProto, usize)>;
        type MapTypes = HashMap<String, (FieldDescriptorProto, FieldDescriptorProto)>;
        let (nested_types, map_types): (NestedTypes, MapTypes) = message
            .nested_type
            .into_iter()
            .enumerate()
            .partition_map(|(idx, nested_type)| {
                if nested_type
                    .options
                    .as_ref()
                    .and_then(|options| options.map_entry)
                    .unwrap_or(false)
                {
                    let key = nested_type.field[0].clone();
                    let value = nested_type.field[1].clone();
                    assert_eq!("key", key.name());
                    assert_eq!("value", value.name());

                    let name = format!("{fq_message_name}.{}", nested_type.name());
                    Either::Right((name, (key, value)))
                } else {
                    Either::Left((nested_type, idx))
                }
            });

        // Split the fields into a vector of the normal fields, and oneof fields.
        // Path indexes are preserved so that comments can be retrieved.
        type Fields = Vec<(FieldDescriptorProto, usize)>;
        type OneofFields = MultiMap<i32, (FieldDescriptorProto, usize)>;
        let (fields, mut oneof_fields): (Fields, OneofFields) = message
            .field
            .into_iter()
            .enumerate()
            .partition_map(|(idx, field)| {
                if field.proto3_optional.unwrap_or(false) {
                    Either::Left((field, idx))
                } else if let Some(oneof_index) = field.oneof_index {
                    Either::Right((oneof_index, (field, idx)))
                } else {
                    Either::Left((field, idx))
                }
            });

        self.append_doc(&fq_message_name, None);
        self.append_type_attributes(&fq_message_name);
        self.push_indent();
        self.buf.push_str("#[derive(Clone, PartialEq, Debug)]\n");
        self.push_indent();
        self.buf.push_str("pub struct ");
        self.buf.push_str(&to_upper_camel(&message_name));
        self.buf.push_str(" {\n");

        self.priv_buf.push_str("impl ::ntex_grpc::Message for ");
        self.priv_buf.push_str(&to_upper_camel(&message_name));
        self.priv_buf.push_str(" {\n");

        let mut has_fields = false;
        let mut write = String::new();
        let mut read = String::new();
        let mut encoded_len = String::new();
        let mut default = String::new();
        let mut accessors = String::new();

        self.depth += 1;
        self.path.push(2);
        for (field, idx) in fields {
            let field_no = field.number();
            let field_name = to_snake(field.name());

            self.path.push(idx as i32);

            let map_entry = field
                .type_name
                .as_ref()
                .and_then(|type_name| map_types.get(type_name));
            let ops = match map_entry {
                Some((key, value)) => self.map_ops(&fq_message_name, key, value),
                None => self.field_ops(&fq_message_name, &field),
            };

            has_fields = true;
            write.push_str(&format!(
                "{ops}serialize(&self.{field_name}, {field_no}, ::ntex_grpc::types::DefaultValue::Default, dst);",
            ));
            read.push_str(&format!(
                "{field_no} => {ops}deserialize(&mut self.{field_name}, tag, wire_type, src)
                    .map_err(|err| err.push(STRUCT_NAME, \"{field_name}\"))?,",
            ));
            encoded_len.push_str(&format!(
                " + {ops}serialized_len(&self.{field_name}, {field_no}, ::ntex_grpc::types::DefaultValue::Default)",
            ));
            default.push_str(&format!(
                "{field_name}: ::core::default::Default::default(),\n",
            ));

            self.enum_accessors(&fq_message_name, &field, map_entry, &mut accessors);
            match map_entry {
                Some((key, value)) => self.append_map_field(&fq_message_name, field, key, value),
                None => self.append_field(&fq_message_name, field),
            }

            self.path.pop();
        }
        self.path.pop();

        self.path.push(8);
        for (idx, oneof) in message.oneof_decl.iter().enumerate() {
            let idx = idx as i32;

            let fields = match oneof_fields.get_vec(&idx) {
                Some(fields) => fields,
                None => continue,
            };

            has_fields = true;
            write.push_str(&format!(
                "::ntex_grpc::NativeType::serialize(&self.{}, 0, ::ntex_grpc::types::DefaultValue::Default, dst);",
                to_snake(oneof.name()),
            ));
            read.push_str(&format!(
                "
               {} => ::ntex_grpc::NativeType::deserialize(&mut self.{}, tag, wire_type, src)
                        .map_err(|err| err.push(STRUCT_NAME, \"{}\"))?,",
                fields.iter().map(|(field, _)| field.number()).join("| "),
                to_snake(oneof.name()),
                to_snake(oneof.name()),
            ));
            encoded_len.push_str(&format!(
                " + ::ntex_grpc::NativeType::serialized_len(&self.{}, 0, ::ntex_grpc::types::DefaultValue::Default)",
                to_snake(oneof.name()),
            ));
            default.push_str(&format!(
                "{}: ::core::default::Default::default(),\n",
                to_snake(oneof.name()),
            ));

            self.path.push(idx);
            self.append_oneof_field(&message_name, &fq_message_name, oneof);
            self.path.pop();
        }
        self.path.pop();

        self.depth -= 1;
        self.push_indent();
        self.buf.push_str("}\n\n");

        if !accessors.is_empty() {
            self.push_indent();
            self.buf.push_str(&format!(
                "impl {} {{\n{accessors}}}\n\n",
                to_upper_camel(&message_name)
            ));
        }

        // message impl =============================
        self.priv_buf.push_str(&format!(
            "#[inline]
              fn write(&self, dst: &mut ::ntex_grpc::BytePages) {{
                {write}
             }}\n\n"
        ));

        let read = if has_fields {
            format!(
                "match tag {{
                 {read}
                 _ => ::ntex_grpc::encoding::skip_field(wire_type, tag, src)?,
             }}"
            )
        } else {
            "::ntex_grpc::encoding::skip_field(wire_type, tag, src)?;".to_string()
        };

        self.priv_buf.push_str(&format!(
            "#[inline]
             fn read(src: &mut ::ntex_grpc::Bytes) -> ::std::result::Result<Self, ::ntex_grpc::DecodeError> {{
                 let mut msg = Self::default();
                 ::ntex_grpc::Message::merge_from(&mut msg, src)?;
                 Ok(msg)
             }}

             #[inline]
             fn merge_from(&mut self, src: &mut ::ntex_grpc::Bytes) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {{
                 const STRUCT_NAME: &str = \"{}\";
                 while !src.is_empty() {{
                    let (tag, wire_type) = ::ntex_grpc::encoding::decode_key(src)?;
                    {read}
                 }}
                 Ok(())
             }}\n\n",
            to_upper_camel(&message_name)
        ));
        self.priv_buf.push_str(&format!(
            "#[inline]
             fn encoded_len(&self) -> usize {{
                 0 {encoded_len}
             }}\n\n"
        ));
        self.priv_buf.push_str("}\n\n");

        // default
        self.priv_buf.push_str(&format!(
            "impl ::std::default::Default for {} {{
                 #[inline]
                 fn default() -> Self {{
                     Self {{ {default} }}
                 }}
             }}\n\n
        ",
            to_upper_camel(&message_name)
        ));
        // ==========================================

        if !message.enum_type.is_empty() || !nested_types.is_empty() || !oneof_fields.is_empty() {
            self.push_mod(&message_name);
            self.path.push(3);
            for (nested_type, idx) in nested_types {
                self.path.push(idx as i32);
                self.append_message(nested_type);
                self.path.pop();
            }
            self.path.pop();

            self.path.push(4);
            for (idx, nested_enum) in message.enum_type.into_iter().enumerate() {
                self.path.push(idx as i32);
                let enum_name = format!(
                    "{}::{}",
                    to_snake(&message_name),
                    to_upper_camel(nested_enum.name())
                );
                self.append_enum(enum_name, nested_enum);
                self.path.pop();
            }
            self.path.pop();

            for (idx, oneof) in message.oneof_decl.into_iter().enumerate() {
                let idx = idx as i32;
                // optional fields create a synthetic oneof that we want to skip
                let fields = match oneof_fields.remove(&idx) {
                    Some(fields) => fields,
                    None => continue,
                };
                self.append_oneof(&message_name, &fq_message_name, oneof, idx, fields);
            }

            self.pop_mod();
        }
    }

    fn append_type_attributes(&mut self, fq_message_name: &str) {
        assert_eq!(b'.', fq_message_name.as_bytes()[0]);
        for attribute in self.config.type_attributes.get(fq_message_name) {
            push_indent(self.buf, self.depth);
            self.buf.push_str(attribute);
            self.buf.push('\n');
        }
    }

    fn append_field_attributes(&mut self, fq_message_name: &str, field_name: &str) {
        assert_eq!(b'.', fq_message_name.as_bytes()[0]);
        for attribute in self
            .config
            .field_attributes
            .get_field(fq_message_name, field_name)
        {
            push_indent(self.buf, self.depth);
            self.buf.push_str(attribute);
            self.buf.push('\n');
        }
    }

    fn append_field(&mut self, fq_message_name: &str, field: FieldDescriptorProto) {
        let repeated = field.label == Some(Label::Repeated as i32);
        let optional = self.optional(&field);
        let ty = self.resolve_type(&field, fq_message_name);

        debug!("    field: {:?}, type: {:?}", field.name(), ty);

        self.append_doc(fq_message_name, Some(field.name()));

        self.push_indent();
        self.append_field_attributes(fq_message_name, field.name());
        self.push_indent();
        self.buf.push_str("pub ");
        self.buf.push_str(&to_snake(field.name()));
        self.buf.push_str(": ");
        if repeated {
            self.buf.push_str("Vec<");
        } else if optional {
            self.buf.push_str("Option<");
        }
        self.buf.push_str(&ty);
        if repeated || optional {
            self.buf.push('>');
        }
        self.buf.push_str(",\n");
    }

    fn append_map_field(
        &mut self,
        fq_message_name: &str,
        field: FieldDescriptorProto,
        key: &FieldDescriptorProto,
        value: &FieldDescriptorProto,
    ) {
        let key_ty = self.resolve_type(key, fq_message_name);
        let value_ty = self.resolve_type(value, fq_message_name);

        debug!(
            "    map field: {:?}, key type: {:?}, value type: {:?}",
            field.name(),
            key_ty,
            value_ty
        );

        self.append_doc(fq_message_name, Some(field.name()));
        self.push_indent();

        let map_type = self
            .config
            .types_map
            .get_first_field(fq_message_name, field.name())
            .cloned()
            .unwrap_or_else(|| "::ntex_grpc::HashMap".to_string());

        self.append_field_attributes(fq_message_name, field.name());
        self.push_indent();
        self.buf.push_str(&format!(
            "pub {}: {}<{}, {}>,\n",
            to_snake(field.name()),
            map_type,
            key_ty,
            value_ty
        ));
    }

    fn append_oneof_field(
        &mut self,
        message_name: &str,
        fq_message_name: &str,
        oneof: &OneofDescriptorProto,
    ) {
        let name = format!(
            "{}::{}",
            to_snake(message_name),
            to_upper_camel(oneof.name())
        );
        self.append_doc(fq_message_name, None);
        self.push_indent();
        self.append_field_attributes(fq_message_name, oneof.name());
        self.push_indent();
        self.buf.push_str(&format!(
            "pub {}: Option<{}>,\n",
            to_snake(oneof.name()),
            name
        ));
    }

    fn append_oneof(
        &mut self,
        message_name: &str,
        fq_message_name: &str,
        oneof: OneofDescriptorProto,
        idx: i32,
        fields: Vec<(FieldDescriptorProto, usize)>,
    ) {
        self.path.push(8);
        self.path.push(idx);
        self.append_doc(fq_message_name, None);
        self.path.pop();
        self.path.pop();

        let name = format!(
            "{}::{}",
            to_snake(message_name),
            to_upper_camel(oneof.name())
        );

        let oneof_name = format!("{}.{}", fq_message_name, oneof.name());
        self.append_type_attributes(&oneof_name);
        self.push_indent();
        self.buf.push_str("#[derive(Clone, PartialEq, Debug)]\n");
        self.push_indent();
        self.buf.push_str("pub enum ");
        self.buf.push_str(&to_upper_camel(oneof.name()));
        self.buf.push_str(" {\n");

        let mut write = String::new();
        let mut read = String::new();
        let mut encoded_len = String::new();

        self.path.push(2);
        self.depth += 1;
        for (field, idx) in &fields {
            let field_no = field.number();
            let field_name = to_upper_camel(field.name());

            let ops = self.field_ops(fq_message_name, field);

            write.push_str(&format!(
                "{name}::{field_name}(ref value) => {ops}serialize(value, {field_no}, ::ntex_grpc::types::DefaultValue::Unknown, dst),",
            ));
            read.push_str(&format!(
                "{field_no} => if let {name}::{field_name}(ref mut value) = *self {{
                    {ops}deserialize(value, {field_no}, wire_type, src)?;
                }} else {{
                    *self = {name}::{field_name}({ops}deserialize_default({field_no}, wire_type, src)?);
                }},\n",
            ));
            encoded_len.push_str(&format!(
                "{name}::{field_name}(ref value) => {ops}serialized_len(value, {field_no}, ::ntex_grpc::types::DefaultValue::Unknown),",
            ));

            self.path.push(*idx as i32);
            self.append_doc(fq_message_name, Some(field.name()));
            self.path.pop();

            self.push_indent();
            self.append_field_attributes(&oneof_name, field.name());

            self.push_indent();
            let ty = self.resolve_type(field, fq_message_name);

            debug!("    oneof: {:?}, type: {ty:?}", field.name());

            self.buf.push_str(&format!("{field_name}({ty}),\n"));
        }
        self.depth -= 1;
        self.path.pop();

        self.push_indent();
        self.buf.push_str("}\n");

        self.priv_buf.push_str(&format!(
            "
        impl ::ntex_grpc::NativeType for {name} {{
            const TYPE: ::ntex_grpc::WireType = ::ntex_grpc::WireType::LengthDelimited;

            fn merge(&mut self, _: &mut ::ntex_grpc::Bytes) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {{
                panic!(\"Not supported\")
            }}

            fn encode_value(&self, _: &mut ::ntex_grpc::BytePages) {{
                panic!(\"Not supported\")
            }}
        "));

        self.priv_buf.push_str(&format!(
            "
            #[inline]
            /// Encodes the message to a buffer.
            fn serialize(&self, _: u32, _: ::ntex_grpc::types::DefaultValue<&Self>, dst: &mut ::ntex_grpc::BytePages) {{
                match *self {{ {write} }}
            }}\n",
        ));
        self.priv_buf.push_str(&format!("
            #[inline]
            /// Decodes an instance of the message from a buffer, and merges it into self.
            fn deserialize(&mut self, tag: u32, wire_type: ::ntex_grpc::WireType, src: &mut ::ntex_grpc::Bytes) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {{
                match tag {{
                    {}
                    _ => unreachable!(\"invalid {}, tag: {{}}\", tag),
                }}
                Ok(())
            }}\n", read.trim_end(), to_upper_camel(oneof.name())));

        self.priv_buf.push_str(&format!(
            "
            #[inline]
            /// Returns the encoded length of the message without a length delimiter.
            fn serialized_len(&self, _: u32, _: ::ntex_grpc::types::DefaultValue<&Self>) -> usize {{
                match *self {{
                    {encoded_len}
                }}
            }}
        }}\n\n"));

        let val = to_upper_camel(fields[0].0.name());
        self.priv_buf.push_str(&format!(
            "
        impl ::std::default::Default for {name} {{
            #[inline]
            fn default() -> Self {{
                {name}::{val}(::std::default::Default::default())
            }}
        }}\n\n"
        ));
    }

    fn location(&self) -> &Location {
        let idx = self
            .source_info
            .location
            .binary_search_by_key(&&self.path[..], |location| &location.path[..])
            .unwrap();

        &self.source_info.location[idx]
    }

    fn append_doc(&mut self, fq_name: &str, field_name: Option<&str>) {
        let append_doc = if let Some(field_name) = field_name {
            self.config
                .disable_comments
                .get_first_field(fq_name, field_name)
                .is_none()
        } else {
            self.config.disable_comments.get(fq_name).next().is_none()
        };
        if append_doc {
            Comments::from_location(self.location()).append_with_indent(self.depth, self.buf)
        }
    }

    fn append_enum(&mut self, full_name: String, desc: EnumDescriptorProto) {
        debug!("  enum: {:?}", desc.name());

        let proto_enum_name = desc.name();
        let enum_name = to_upper_camel(proto_enum_name);

        let enum_values = &desc.value;
        let fq_proto_enum_name = format!(
            "{}{}.{}",
            if self.package.is_empty() { "" } else { "." },
            self.package,
            proto_enum_name
        );
        if self
            .extern_paths
            .resolve_ident(&fq_proto_enum_name)
            .is_some()
        {
            return;
        }

        self.append_doc(&fq_proto_enum_name, None);
        self.append_type_attributes(&fq_proto_enum_name);
        self.push_indent();
        self.buf
            .push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]\n");
        self.push_indent();
        self.buf.push_str("#[repr(i32)]\n");
        self.push_indent();
        self.buf.push_str("pub enum ");
        self.buf.push_str(&enum_name);
        self.buf.push_str(" {\n");

        let variant_mappings =
            build_enum_value_mappings(&enum_name, self.config.strip_enum_prefix, enum_values);

        self.depth += 1;
        self.path.push(2);
        for variant in variant_mappings.iter() {
            self.path.push(variant.path_idx as i32);

            self.append_doc(&fq_proto_enum_name, Some(variant.proto_name));
            self.append_field_attributes(&fq_proto_enum_name, variant.proto_name);
            self.push_indent();
            self.buf.push_str(&variant.generated_variant_name);
            self.buf.push_str(" = ");
            self.buf.push_str(&variant.proto_number.to_string());
            self.buf.push_str(",\n");

            self.path.pop();
        }

        self.path.pop();
        self.depth -= 1;

        self.push_indent();
        self.buf.push_str("}\n\n");

        self.push_indent();
        self.buf.push_str("impl ");
        self.buf.push_str(&enum_name);
        self.buf.push_str(" {\n");
        self.depth += 1;
        self.path.push(2);

        // generate to_str_name()
        self.push_indent();
        self.buf.push_str(
            "/// String value of the enum field names used in the ProtoBuf definition with stripped prefix.\n",
        );
        self.push_indent();

        self.buf
            .push_str("pub fn to_str_name(self) -> &'static str {\n");
        self.depth += 1;

        self.push_indent();
        self.buf.push_str("match self {\n");
        self.depth += 1;

        for variant in variant_mappings.iter() {
            self.push_indent();
            self.buf.push_str(&enum_name);
            self.buf.push_str("::");
            self.buf.push_str(&variant.generated_variant_name);
            self.buf.push_str(" => \"");
            self.buf.push_str(variant.proto_value);
            self.buf.push_str("\",\n");
        }

        self.depth -= 1;
        self.push_indent();
        self.buf.push_str("}\n"); // End of match

        self.depth -= 1;
        self.push_indent();
        self.buf.push_str("}\n\n"); // End of to_str_name()

        // generate to_origin_name()
        self.push_indent();
        self.buf.push_str(
            "/// String value of the enum field names used in the ProtoBuf definition.\n",
        );
        self.push_indent();
        self.buf.push_str("///\n");
        self.push_indent();
        self.buf.push_str(
            "/// The values are not transformed in any way and thus are considered stable\n",
        );
        self.push_indent();
        self.buf.push_str(
            "/// (if the ProtoBuf definition does not change) and safe for programmatic use.\n",
        );
        self.push_indent();

        self.buf
            .push_str("pub fn to_origin_name(self) -> &'static str {\n");
        self.depth += 1;

        self.push_indent();
        self.buf.push_str("match self {\n");
        self.depth += 1;

        for variant in variant_mappings.iter() {
            self.push_indent();
            self.buf.push_str(&enum_name);
            self.buf.push_str("::");
            self.buf.push_str(&variant.generated_variant_name);
            self.buf.push_str(" => \"");
            self.buf.push_str(variant.proto_name);
            self.buf.push_str("\",\n");
        }

        self.depth -= 1;
        self.push_indent();
        self.buf.push_str("}\n"); // End of match

        self.depth -= 1;
        self.push_indent();
        self.buf.push_str("}\n\n"); // End of to_origin_name()

        self.path.pop();
        self.depth -= 1;
        self.push_indent();

        self.buf.push_str(
            "pub fn from_i32(value: i32) -> ::std::option::Option<Self> {
                match value {
            ",
        );

        for variant in variant_mappings.iter() {
            self.buf.push_str(&format!(
                "{} => Some({}::{}),\n",
                variant.proto_number, enum_name, variant.generated_variant_name
            ));
        }
        self.buf.push_str(
            "    _ => ::std::option::Option::None,
            }
        }",
        );
        self.buf.push_str("}\n\n"); // End of impl

        // NativeType impl
        self.priv_buf.push_str(&format!(
            "impl ::ntex_grpc::NativeType for {} {{
                 const TYPE: ::ntex_grpc::WireType = ::ntex_grpc::WireType::Varint;

                 #[inline]
                 fn merge(&mut self, src: &mut ::ntex_grpc::Bytes) -> ::std::result::Result<(), ::ntex_grpc::DecodeError> {{
                     *self = ::ntex_grpc::encoding::decode_varint(src).map(|val| Self::from_i32(val as i32).unwrap_or_default())?;
                     Ok(())
                 }}

                 #[inline]
                 fn encode_value(&self, dst: &mut ::ntex_grpc::BytePages) {{
                    ::ntex_grpc::encoding::encode_varint(*self as i32 as u64, dst);
                 }}

                 #[inline]
                 fn encoded_len(&self, tag: u32) -> usize {{
                     ::ntex_grpc::encoding::key_len(tag) + ::ntex_grpc::encoding::encoded_len_varint(*self as i32 as u64)
                 }}

                 #[inline]
                 fn value_len(&self) -> usize {{
                     ::ntex_grpc::encoding::encoded_len_varint(*self as i32 as u64)
                 }}

                 #[inline]
                 fn is_default(&self) -> bool {{
                     self == &{}::{}
                 }}
            }}

            impl ::std::default::Default for {} {{
                #[inline]
                fn default() -> Self {{
                    {}::{}
                }}
            }}

            impl ::std::convert::From<{}> for i32 {{
                #[inline]
                fn from(value: {}) -> i32 {{
                    value as i32
                }}
            }}\n\n",
            full_name,
            full_name,
            variant_mappings[0].generated_variant_name,
            full_name,
            full_name,
            variant_mappings[0].generated_variant_name,
            full_name,
            full_name,
        ));
    }

    fn push_service(&mut self, service: ServiceDescriptorProto) {
        let name = service.name().to_owned();
        debug!("  service: {name:?}");

        let comments = Comments::from_location(self.location());

        self.path.push(2);
        let methods = service
            .method
            .into_iter()
            .enumerate()
            .map(|(idx, mut method)| {
                debug!("  method: {:?}", method.name());
                self.path.push(idx as i32);
                let comments = Comments::from_location(self.location());
                self.path.pop();

                let name = method.name.take().unwrap();
                let input_proto_type = method.input_type.take().unwrap();
                let output_proto_type = method.output_type.take().unwrap();
                let input_type = self.resolve_ident(&input_proto_type);
                let input_type_extern = self.is_extern_ident(&input_proto_type);
                let output_type = self.resolve_ident(&output_proto_type);
                let client_streaming = method.client_streaming();
                let server_streaming = method.server_streaming();

                Method {
                    name: to_snake(&name),
                    proto_name: name,
                    options: method.options.unwrap_or_default(),
                    comments,
                    input_type,
                    output_type,
                    input_proto_type,
                    output_proto_type,
                    client_streaming,
                    server_streaming,
                    input_type_extern,
                }
            })
            .collect();
        self.path.pop();

        let service = Service {
            name: to_upper_camel(&name),
            proto_name: name,
            package: self.package.clone(),
            comments,
            methods,
            options: service.options.unwrap_or_default(),
        };

        if let Some(service_generator) = self.config.service_generator.as_mut() {
            service_generator.generate(service, self.buf, &mut self.priv_buf)
        }
    }

    fn push_indent(&mut self) {
        push_indent(self.buf, self.depth);
    }

    fn push_mod(&mut self, module: &str) {
        self.push_indent();
        self.buf.push_str("/// Nested message and enum types in `");
        self.buf.push_str(module);
        self.buf.push_str("`.\n");

        self.push_indent();
        self.buf.push_str("pub mod ");
        self.buf.push_str(&to_snake(module));
        self.buf.push_str(" {\n");

        self.package.push('.');
        self.package.push_str(module);

        self.depth += 1;
    }

    fn pop_mod(&mut self) {
        self.depth -= 1;

        let idx = self.package.rfind('.').unwrap();
        self.package.truncate(idx);

        self.push_indent();
        self.buf.push_str("}\n\n");
    }

    fn resolve_type(&self, field: &FieldDescriptorProto, fq_message_name: &str) -> String {
        if let Some(tp) = self
            .config
            .types_map
            .get_first_field(fq_message_name, field.name())
        {
            tp.clone()
        } else {
            match field.r#type() {
                Type::Group | Type::Message => {
                    let ty = self.resolve_ident(field.type_name());
                    let key = (
                        fq_message_name.trim_start_matches('.').to_string(),
                        field.name().to_string(),
                    );
                    if self.boxed.contains(&key) {
                        format!("::std::boxed::Box<{ty}>")
                    } else {
                        ty
                    }
                }
                // enums are open, the field keeps values unknown to the generated enum
                Type::Enum => "i32".to_string(),
                _ => to_rust_type(field.r#type()),
            }
        }
    }

    /// Typed accessors for an enum field, the field itself holds the raw `i32` value
    fn enum_accessors(
        &self,
        fq_message_name: &str,
        field: &FieldDescriptorProto,
        map_entry: Option<&(FieldDescriptorProto, FieldDescriptorProto)>,
        out: &mut String,
    ) {
        let (enum_field, key_ty) = match map_entry {
            Some((key, value)) => (value, Some(self.resolve_type(key, fq_message_name))),
            None => (field, None),
        };
        if enum_field.r#type() != Type::Enum
            || self.resolve_type(enum_field, fq_message_name) != "i32"
        {
            return;
        }

        let ty = self.resolve_ident(enum_field.type_name());
        let name = to_snake(field.name());
        let base = name.trim_start_matches("r#");
        let proto = field.name();

        let code = if let Some(key_ty) = key_ty {
            format!(
                "/// Returns the enum value of `{proto}` for `key`, `None` if the key is
                 /// missing or the value is unknown.
                 pub fn get_{base}(&self, key: &{key_ty}) -> ::std::option::Option<{ty}> {{
                     self.{name}.get(key).copied().and_then({ty}::from_i32)
                 }}

                 /// Inserts the enum value of `{proto}` for `key`, returns the previous value.
                 pub fn insert_{base}(&mut self, key: {key_ty}, value: {ty}) -> ::std::option::Option<{ty}> {{
                     self.{name}.insert(key, value as i32).and_then({ty}::from_i32)
                 }}\n\n"
            )
        } else if field.label() == Label::Repeated {
            format!(
                "/// Returns an iterator over the enum values of `{proto}`, unknown values are skipped.
                 pub fn {name}(&self) -> impl ::std::iter::Iterator<Item = {ty}> + '_ {{
                     self.{name}.iter().filter_map(|value| {ty}::from_i32(*value))
                 }}

                 /// Appends an enum value to `{proto}`.
                 pub fn push_{base}(&mut self, value: {ty}) {{
                     self.{name}.push(value as i32);
                 }}\n\n"
            )
        } else if self.optional(field) {
            format!(
                "/// Returns the enum value of `{proto}`, or the default if the field is
                 /// unset or has an unknown value.
                 pub fn {name}(&self) -> {ty} {{
                     self.{name}.and_then({ty}::from_i32).unwrap_or_default()
                 }}

                 /// Sets `{proto}` to the enum value.
                 pub fn set_{base}(&mut self, value: {ty}) {{
                     self.{name} = ::std::option::Option::Some(value as i32);
                 }}\n\n"
            )
        } else {
            format!(
                "/// Returns the enum value of `{proto}`, or the default if the field has
                 /// an unknown value.
                 pub fn {name}(&self) -> {ty} {{
                     {ty}::from_i32(self.{name}).unwrap_or_default()
                 }}

                 /// Sets `{proto}` to the enum value.
                 pub fn set_{base}(&mut self, value: {ty}) {{
                     self.{name} = value as i32;
                 }}\n\n"
            )
        };
        out.push_str(&code);
    }

    /// Wire format of a field whose encoding differs from the `NativeType`
    /// encoding of its Rust type. Fields with a type from `types_map` use
    /// the `NativeType` impl of that type.
    fn field_format(
        &self,
        field: &FieldDescriptorProto,
        fq_message_name: &str,
    ) -> Option<&'static str> {
        if self
            .config
            .types_map
            .get_first_field(fq_message_name, field.name())
            .is_some()
        {
            return None;
        }
        match field.r#type() {
            Type::Sint32 | Type::Sint64 => Some("::ntex_grpc::types::ZigZag"),
            Type::Fixed32 | Type::Fixed64 | Type::Sfixed32 | Type::Sfixed64 => {
                Some("::ntex_grpc::types::Fixed")
            }
            Type::Group => Some("::ntex_grpc::types::Group"),
            _ => None,
        }
    }

    /// Path prefix of the encoding functions for a field
    fn field_ops(&self, fq_message_name: &str, field: &FieldDescriptorProto) -> String {
        let format = self.field_format(field, fq_message_name);
        if self.unpacked(field) {
            return format!(
                "<::ntex_grpc::types::Unpacked<{}> as ::ntex_grpc::types::FieldFormat<_>>::",
                format.unwrap_or("::ntex_grpc::types::Native")
            );
        }
        match format {
            Some(format) => format!("<{format} as ::ntex_grpc::types::FieldFormat<_>>::"),
            None => "::ntex_grpc::NativeType::".to_string(),
        }
    }

    /// Repeated scalar field written unpacked. Scalar fields are packed by
    /// default in proto3 and unpacked in proto2, the `packed` option overrides it.
    fn unpacked(&self, field: &FieldDescriptorProto) -> bool {
        field.label() == Label::Repeated
            && !matches!(
                field.r#type(),
                Type::String | Type::Bytes | Type::Message | Type::Group
            )
            && !field
                .options
                .as_ref()
                .and_then(|opts| opts.packed)
                .unwrap_or(self.syntax == Syntax::Proto3)
    }

    /// Path prefix of the encoding functions for a map field
    fn map_ops(
        &self,
        fq_message_name: &str,
        key: &FieldDescriptorProto,
        value: &FieldDescriptorProto,
    ) -> String {
        let key = self.field_format(key, fq_message_name);
        let value = self.field_format(value, fq_message_name);
        if key.is_none() && value.is_none() {
            "::ntex_grpc::NativeType::".to_string()
        } else {
            let native = "::ntex_grpc::types::Native";
            format!(
                "<::ntex_grpc::types::Map<{}, {}> as ::ntex_grpc::types::FieldFormat<_>>::",
                key.unwrap_or(native),
                value.unwrap_or(native)
            )
        }
    }

    fn is_extern_ident(&self, pb_ident: &str) -> bool {
        self.extern_paths.is_extern_ident(pb_ident)
    }

    fn resolve_ident(&self, pb_ident: &str) -> String {
        // protoc should always give fully qualified identifiers.
        assert_eq!(".", &pb_ident[..1]);

        if let Some(proto_ident) = self.extern_paths.resolve_ident(pb_ident) {
            return proto_ident;
        }

        let mut local_path = self.package.split('.').peekable();

        // If no package is specified the start of the package name will be '.'
        // and split will return an empty string ("") which breaks resolution
        // The fix to this is to ignore the first item if it is empty.
        if local_path.peek().is_some_and(|s| s.is_empty()) {
            local_path.next();
        }

        let mut ident_path = pb_ident[1..].split('.');
        let ident_type = ident_path.next_back().unwrap();
        let mut ident_path = ident_path.peekable();

        // Skip path elements in common.
        while local_path.peek().is_some() && local_path.peek() == ident_path.peek() {
            local_path.next();
            ident_path.next();
        }

        local_path
            .map(|_| "super".to_string())
            .chain(ident_path.map(to_snake))
            .chain(iter::once(to_upper_camel(ident_type)))
            .join("::")
    }

    fn optional(&self, field: &FieldDescriptorProto) -> bool {
        if field.proto3_optional.unwrap_or(false) {
            return true;
        }

        if field.label() != Label::Optional {
            return false;
        }

        match field.r#type() {
            Type::Message => true,
            _ => self.syntax == Syntax::Proto2,
        }
    }
}

/// Strip an enum's type name from the prefix of an enum value.
///
/// This function assumes that both have been formatted to Rust's
/// upper camel case naming conventions.
///
/// It also tries to handle cases where the stripped name would be
/// invalid - for example, if it were to begin with a number.
fn strip_enum_prefix(prefix: &str, name: &str) -> String {
    let stripped = if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
        name.split_at(prefix.len()).1
    } else {
        name
    };

    // If the next character after the stripped prefix is not
    // uppercase, then it means that we didn't have a true prefix -
    // for example, "Foo" should not be stripped from "Foobar".
    if stripped
        .chars()
        .next()
        .map(char::is_uppercase)
        .unwrap_or(false)
    {
        stripped.to_owned()
    } else {
        name.to_owned()
    }
}

#[derive(Debug)]
struct EnumVariantMapping<'a> {
    path_idx: usize,
    proto_name: &'a str,
    proto_number: i32,
    proto_value: &'a str,
    generated_variant_name: String,
}

fn build_enum_value_mappings<'a>(
    generated_enum_name: &str,
    do_strip_enum_prefix: bool,
    enum_values: &'a [EnumValueDescriptorProto],
) -> Vec<EnumVariantMapping<'a>> {
    let mut numbers = HashSet::new();
    let mut generated_names = HashMap::new();
    let mut mappings = Vec::new();
    let enum_name_lowercase = generated_enum_name.to_lowercase();
    let enum_name_snek = generated_enum_name.to_snek_case().to_lowercase();

    for (idx, value) in enum_values.iter().enumerate() {
        // Skip duplicate enum values. Protobuf allows this when the
        // 'allow_alias' option is set.
        if !numbers.insert(value.number()) {
            continue;
        }

        let mut generated_variant_name = to_upper_camel(value.name());
        if do_strip_enum_prefix {
            generated_variant_name =
                strip_enum_prefix(generated_enum_name, &generated_variant_name);
        }

        if let Some(old_v) =
            generated_names.insert(generated_variant_name.to_owned(), value.name())
        {
            panic!("Generated enum variant names overlap: `{}` variant name to be used both by `{}` and `{}` ProtoBuf enum values",
                generated_variant_name, old_v, value.name());
        }

        let val = if do_strip_enum_prefix {
            let mut val = value.name();
            let val_lower = val.to_lowercase();

            if val_lower.starts_with(&enum_name_lowercase) {
                val.split_at(generated_enum_name.len()).1
            } else if val_lower.starts_with(&enum_name_snek) {
                val = val.split_at(enum_name_snek.len()).1;
                val = val.strip_prefix('_').unwrap_or(val);
                if val
                    .chars()
                    .next()
                    .map(char::is_alphanumeric)
                    .unwrap_or(false)
                {
                    val
                } else {
                    value.name()
                }
            } else {
                val
            }
        } else {
            value.name()
        };

        mappings.push(EnumVariantMapping {
            path_idx: idx,
            proto_name: value.name(),
            proto_number: value.number(),
            proto_value: val,
            generated_variant_name,
        })
    }
    mappings
}

fn to_rust_type(tp: Type) -> String {
    match tp {
        Type::Double => String::from("f64"),
        Type::Float => String::from("f32"),
        Type::Uint32 | Type::Fixed32 => String::from("u32"),
        Type::Uint64 | Type::Fixed64 => String::from("u64"),
        Type::Int32 | Type::Sfixed32 | Type::Sint32 => String::from("i32"),
        Type::Int64 | Type::Sfixed64 | Type::Sint64 => String::from("i64"),
        Type::Bool => String::from("bool"),
        Type::String => String::from("::ntex_grpc::ByteString"),
        Type::Bytes => String::from("::ntex_grpc::Bytes"),

        Type::Group | Type::Message | Type::Enum => panic!("Unsupported"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_enum_prefix() {
        assert_eq!(strip_enum_prefix("Foo", "FooBar"), "Bar");
        assert_eq!(strip_enum_prefix("Foo", "Foobar"), "Foobar");
        assert_eq!(strip_enum_prefix("Foo", "Foo"), "Foo");
        assert_eq!(strip_enum_prefix("Foo", "Bar"), "Bar");
        assert_eq!(strip_enum_prefix("Foo", "Foo1"), "Foo1");
    }

    fn field(name: &str, ty: &str, label: Label) -> FieldDescriptorProto {
        let mut f = FieldDescriptorProto {
            name: Some(name.to_string()),
            type_name: Some(ty.to_string()),
            ..Default::default()
        };
        f.set_type(Type::Message);
        f.set_label(label);
        f
    }

    fn message(
        name: &str,
        field: Vec<FieldDescriptorProto>,
        nested_type: Vec<DescriptorProto>,
    ) -> DescriptorProto {
        DescriptorProto {
            name: Some(name.to_string()),
            field,
            nested_type,
            ..Default::default()
        }
    }

    #[test]
    fn test_boxed_fields() {
        let messages = vec![
            message(
                "Tree",
                vec![
                    field("left", ".pkg.Tree", Label::Optional),
                    field("children", ".pkg.Tree", Label::Repeated),
                    field("leaf", ".pkg.Leaf", Label::Optional),
                    field("inner", ".pkg.Tree.Inner", Label::Optional),
                ],
                vec![message(
                    "Inner",
                    vec![field("back", ".pkg.Tree", Label::Optional)],
                    vec![],
                )],
            ),
            message("Leaf", vec![], vec![]),
            message(
                "Ping",
                vec![field("pong", ".pkg.Pong", Label::Optional)],
                vec![],
            ),
            message(
                "Pong",
                vec![field("ping", ".pkg.Ping", Label::Optional)],
                vec![],
            ),
        ];
        let boxed = boxed_fields("pkg", &messages);

        let mut expected: Vec<_> = [
            ("pkg.Tree", "left"),
            ("pkg.Tree", "inner"),
            ("pkg.Tree.Inner", "back"),
            ("pkg.Ping", "pong"),
            ("pkg.Pong", "ping"),
        ]
        .iter()
        .map(|(m, f)| (m.to_string(), f.to_string()))
        .collect();
        expected.sort();
        let mut boxed: Vec<_> = boxed.into_iter().collect();
        boxed.sort();
        assert_eq!(boxed, expected);
    }
}
