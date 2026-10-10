# ntex-grpc-codegen

Code generator for [ntex-grpc](https://github.com/ntex-rs/ntex-grpc). It
installs the `ntex-grpc` command, which turns `.proto` files into Rust code:
messages, a typed client and the service definition used by the server.

```sh
cargo install ntex-grpc-codegen

ntex-grpc helloworld.proto helloworld.rs --out-dir ./src --include-dir ./
```

It needs `protoc`. If `protoc` isn't in `PATH`, point the `PROTOC` environment
variable at it.

Options:

* `--out-dir DIR`: where to write the generated file.
* `--include-dir DIR`: where to look for imported `.proto` files. Can be repeated.
* `--map NAME=TYPE`: use your own Rust type for a field, e.g.
  `--map HelloRequest.msg_id=crate::unique_id::UniqueId`. The type must
  implement `ntex_grpc::NativeType`.
* `--well-known-types`: generate the `google.protobuf` types instead of using
  the ones from `ntex_grpc::google_types`.
* `--rustfmt-path FILE`: rustfmt config used to format the output.
* `--optional-messages`: generate singular message fields as `Option<T>`, so an
  unset field is `None` and is not encoded. By default they are plain values,
  always encoded, and a missing field decodes as the default.
* `--open-enums`: generate enum fields as `i32`, so values unknown to the
  generated enum are kept. Typed accessors (`field()`/`set_field()`,
  `push_field()`, `get_field()`/`insert_field()` for maps) convert them. By
  default enum fields have the enum type and unknown values decode as the
  default variant.

See the [ntex-grpc README](https://github.com/ntex-rs/ntex-grpc#readme) for
how to use the generated code.
