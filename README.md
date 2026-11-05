# ntex-grpc

gRPC client and server for the [ntex](https://github.com/ntex-rs/ntex) framework.

[![crates.io](https://img.shields.io/crates/v/ntex-grpc.svg)](https://crates.io/crates/ntex-grpc)
[![Documentation](https://docs.rs/ntex-grpc/badge.svg)](https://docs.rs/ntex-grpc)
[![MSRV](https://img.shields.io/badge/rustc-1.97+-lightgray.svg)](https://blog.rust-lang.org/)
[![codecov](https://codecov.io/gh/ntex-rs/ntex-grpc/branch/main/graph/badge.svg)](https://codecov.io/gh/ntex-rs/ntex-grpc/tree/main)

You describe a service in a `.proto` file and generate Rust code from it once.
The generated code has the messages, a typed client and the service definition
used by the server. Messages are encoded by generated code that writes straight
into ntex buffers. ntex-grpc doesn't depend on `prost` at runtime.

The [gRPC chapter](https://docs.ntex.rs/11-grpc.html) of the ntex guide walks
through code generation, custom field types, the client and its transports,
and the server, with runnable examples in
[ntex-rs/examples](https://github.com/ntex-rs/examples/tree/main/protocols).

| Crate | What it is |
|-------|------------|
| [`ntex-grpc`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc) | Runtime: client, server, message encoding, well-known types |
| [`ntex-grpc-codegen`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc-codegen) | `ntex-grpc` command line tool, turns `.proto` files into Rust code |
| [`ntex-grpc-derive`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc-derive) | `#[server]` macro, re-exported by `ntex-grpc` |

## Example

Install the code generator. It needs `protoc`; if `protoc` isn't in `PATH`,
point the `PROTOC` environment variable at it:

```sh
cargo install ntex-grpc-codegen
```

Describe the service in `helloworld.proto`:

```protobuf
syntax = "proto3";

package helloworld;

service Greeter {
  rpc SayHello (HelloRequest) returns (HelloReply) {}
}

message HelloRequest {
  string name = 1;
}

message HelloReply {
  string message = 1;
}
```

Generate `src/helloworld.rs` once and commit it; nothing runs at build time:

```sh
ntex-grpc helloworld.proto helloworld.rs --out-dir ./src --include-dir ./
```

Add the runtime crates to `Cargo.toml`:

```toml
[dependencies]
ntex = "4"
ntex-grpc = "4"
ntex-h2 = "4"
```

A server implements the service with `#[server]`:

```rust
use std::convert::Infallible;

use ntex::{ServiceFactory, SharedCfg, server::Server};
use ntex_grpc::server;

mod helloworld;
use crate::helloworld::{HelloReply, HelloRequest};

#[derive(Clone)]
pub struct GreeterServer;

#[server(crate::helloworld::Greeter)]
impl GreeterServer {
    #[method(SayHello)]
    async fn say_hello(&self, req: HelloRequest) -> HelloReply {
        HelloReply {
            message: format!("Hello {}!", req.name).into(),
        }
    }
}

// The server creates one service instance per connection
impl ServiceFactory<(), server::ServerRequest> for GreeterServer {
    type Res = server::ServerResponse;
    type Error = server::ServerError;
    type Service = GreeterServer;
    type InitError = Infallible;

    async fn create(&self, _: &()) -> Result<Self::Service, Self::InitError> {
        Ok(self.clone())
    }
}

#[ntex::main]
async fn main() -> std::io::Result<()> {
    Server::builder()
        .bind("grpc", "0.0.0.0:50051", SharedCfg::new("GRPC"), async |_| {
            server::GrpcServer::new(GreeterServer)
        })?
        .run()
        .await
}
```

A client calls it over an `ntex-h2` connection pool:

```rust
use ntex::SharedCfg;
use ntex_grpc::client::Client;
use ntex_h2::client as h2;

mod helloworld;
use self::helloworld::{GreeterClient, HelloRequest};

#[ntex::main]
async fn main() {
    let h2client = h2::Client::builder("127.0.0.1:50051").build(SharedCfg::default());
    let client = GreeterClient::new(Client::new(h2client));

    let res = client
        .say_hello(&HelloRequest { name: "world".into() })
        .send()
        .await
        .unwrap();
    println!("reply: {}", res.message);
}
```

## Learn more

The [gRPC chapter](https://docs.ntex.rs/11-grpc.html) of the ntex guide covers
the rest:

* [From .proto to Rust](https://docs.ntex.rs/11-grpc.html#from-proto-to-rust):
  generator options, what gets generated, type mapping and custom field types.
* [Calling services](https://docs.ntex.rs/11-grpc.html#calling-services):
  connections, headers, compression, message limits, streaming calls, custom
  transports, errors and deadlines.
* [Serving gRPC](https://docs.ntex.rs/11-grpc.html#serving-grpc): streaming
  methods, error statuses, several services on one port, tuning and TLS.
* [Testing your services](https://docs.ntex.rs/11-grpc.html#testing-your-services)
  and the [production checklist](https://docs.ntex.rs/11-grpc.html#production-checklist).

## Examples

* [`examples/helloworld`](https://github.com/ntex-rs/ntex-grpc/tree/main/examples/helloworld): a server and a multi-threaded load-generating client.
* [`examples/custom`](https://github.com/ntex-rs/ntex-grpc/tree/main/examples/custom): a custom field type.
* [ntex-rs/examples](https://github.com/ntex-rs/examples/tree/main/protocols): the `grpc-*` examples from the [guide chapter](https://docs.ntex.rs/11-grpc.html), covering streaming, custom transports, server configuration and TLS.

Run the helloworld pair from the repository root, each in its own terminal:

```sh
cargo run -p helloworld --bin server -- 50051
cargo run -p helloworld --bin client -- 127.0.0.1:50051
```

## License

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/ntex-rs/ntex-grpc/blob/main/LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/ntex-rs/ntex-grpc/blob/main/LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.
