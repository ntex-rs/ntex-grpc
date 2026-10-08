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

See the [ntex-grpc README](https://github.com/ntex-rs/ntex-grpc#readme) for
how to use the generated code.
