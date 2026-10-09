# ntex-grpc

gRPC client and server for the [ntex](https://github.com/ntex-rs/ntex) framework.

[![crates.io](https://img.shields.io/crates/v/ntex-grpc.svg)](https://crates.io/crates/ntex-grpc)
[![Documentation](https://docs.rs/ntex-grpc/badge.svg)](https://docs.rs/ntex-grpc)
[![MSRV](https://img.shields.io/badge/rustc-1.97+-lightgray.svg)](https://blog.rust-lang.org/)

You describe a service in a `.proto` file and generate Rust code from it once.
The generated code has the messages, a typed client and the service definition
used by the server. Messages are encoded by generated code that writes straight
into ntex buffers. ntex-grpc doesn't depend on `prost` at runtime.

| Crate | What it is |
|-------|------------|
| [`ntex-grpc`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc) | Runtime: client, server, message encoding, well-known types |
| [`ntex-grpc-codegen`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc-codegen) | `ntex-grpc` command line tool, turns `.proto` files into Rust code |
| [`ntex-grpc-derive`](https://github.com/ntex-rs/ntex-grpc/tree/main/ntex-grpc-derive) | `#[server]` macro, re-exported by `ntex-grpc` |
| [`ntex-prost-build`](https://github.com/ntex-rs/ntex-grpc/tree/main/prost-build) | Fork of `prost-build` used by the code generator |

## Generating code

Install the code generator:

```sh
cargo install ntex-grpc-codegen
```

It needs `protoc`. If `protoc` isn't in `PATH`, point the `PROTOC` environment
variable at it.

Take a service like this one:

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

and generate `src/helloworld.rs`:

```sh
ntex-grpc helloworld.proto helloworld.rs --out-dir ./src --include-dir ./
```

The output is plain Rust, so commit it next to the `.proto` file and run the
tool again whenever the `.proto` file changes. Nothing runs at build time.
`ntex-grpc --help` lists all options.

For `helloworld.proto` you get:

* `HelloRequest` and `HelloReply` structs. Their string fields are `ByteString`.
* `GreeterClient<T>`, with a method for each rpc.
* `Greeter`, the service definition the server uses to route requests.

Add the runtime crates to `Cargo.toml`:

```toml
[dependencies]
ntex = "4"
ntex-grpc = "3"
ntex-h2 = "4"
```

## Client

The client runs over an HTTP/2 connection pool from `ntex-h2`. Calling a method
builds a request, and nothing goes over the network until you call `send()`:

```rust
use ntex::SharedCfg;
use ntex_grpc::client::Client;
use ntex_h2::client as h2;

mod helloworld;
use self::helloworld::{GreeterClient, HelloRequest};

#[ntex::main]
async fn main() {
    // h2 connection pool, connections are opened on demand
    let h2client = h2::Client::builder("127.0.0.1:50051").build(SharedCfg::default());
    let client = GreeterClient::new(Client::new(h2client));

    let res = client
        .say_hello(&HelloRequest { name: "world".into() })
        .send()
        .await
        .unwrap();

    // `res` derefs to `HelloReply`
    println!("reply: {}", res.message);
}
```

The client is cheap to clone, so you can hand copies to other tasks. Clones
share the connection pool.

You can add metadata and a deadline before sending. A request borrows its
message, and `header()` and `timeout()` take `&mut self`, so keep both in
variables:

```rust
use std::time::Duration;
use ntex_grpc::client::ClientError;

let msg = HelloRequest { name: "world".into() };
let mut req = client.say_hello(&msg);
req.header("x-request-id", "42")
    .timeout(Duration::from_secs(1)); // sent as `grpc-timeout`, enforced locally too

match req.send().await {
    Ok(res) => println!("trailers: {:?}", res.trailers()),
    // the error derefs to `ClientError`
    Err(err) => match &*err {
        ClientError::GrpcStatus(status, trailers, _) => {
            println!("failed with {status:?}, trailers: {trailers:?}")
        }
        _ => println!("request failed: {err}"),
    },
}
```

A non-OK status from the server comes back as `ClientError::GrpcStatus`.
If the timeout runs out, you get `ClientError::DeadlineExceeded`, whether the
server reported it or the client gave up first.
Connection, HTTP/2 and decoding failures have their own `ClientError` variants.

## Server

Implement the service on your own type and mark it with `#[server]`. Each rpc
maps to a method marked with `#[method(Name)]`, where `Name` is the rpc name
from the `.proto` file:

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

A method can take `server::Request<HelloRequest>` instead of the bare message
when it needs the request headers. `Request` derefs to the message, but its own
`name`, `headers` and `message` fields come first, so a message field with one
of these names has to be read as `req.message.name`.

A method can also return `Result<HelloReply, E>` if `E: Into<HelloReply>`. The
error is turned into a reply, so the client still sees `grpc-status: 0`.

The server sends a gRPC status itself in these cases:

* `NOT_FOUND` for an unknown method;
* `UNIMPLEMENTED` for an rpc that has no `#[method]`;
* `INVALID_ARGUMENT` if the request message doesn't decode;
* `DEADLINE_EXCEEDED` if the client's `grpc-timeout` runs out before the method
  finishes.

`GrpcServer` is an ntex service that takes an I/O stream, so TLS works the
usual ntex way. Chain a TLS acceptor that negotiates `h2` with ALPN in front of
it.

## Custom field types

Any protobuf field can be mapped to your own Rust type. Implement
`ntex_grpc::NativeType` for the type and pass a mapping to the generator:

```sh
ntex-grpc helloworld.proto helloworld.rs --out-dir ./src --include-dir ./ \
    --map HelloRequest.msg_id=crate::unique_id::UniqueId
```

[`examples/custom`](https://github.com/ntex-rs/ntex-grpc/tree/main/examples/custom) maps a `bytes` field to a UUID type this way.

## Well-known types

`ntex_grpc::google_types` has `Duration`, `Timestamp` and the wrapper types
(`StringValue`, `Int64Value`, `BoolValue` and so on). When a `.proto` file
imports them from `google/protobuf`, the generated code uses these types.

`Duration` converts to and from `std::time::Duration`. `Timestamp` converts to
`SystemTime`, and `Timestamp::now()` gives the current time.

## Compression

gzip and zstd message compression comes with the `compression` feature:

```toml
ntex-grpc = { version = "3.2", features = ["compression"] }
```

Requests go out uncompressed unless you pick an encoding:

```rust
use ntex_grpc::Compression;

let msg = HelloRequest { name: "world".into() };
let mut req = client.say_hello(&msg);
req.compression(Compression::Zstd);
let res = req.send().await.unwrap();
```

The server needs no setup. It accepts requests in either encoding and
compresses the response the same way as the request. Both sides send
`grpc-accept-encoding: gzip,zstd`, and the client decompresses responses in
either encoding.

* Empty messages, messages under 64 bytes and messages that don't get smaller
  are sent uncompressed.
* Size limits apply after decompression. `max_message_size()` on the request
  and on `GrpcServer` covers the decompressed message, and a larger one fails
  with `RESOURCE_EXHAUSTED`. `max_send_message_size()` covers the message as
  it is sent, after compression.
* Large messages are compressed and decompressed on the runtime's blocking
  thread pool, so they don't hold up the worker thread.
* A server that doesn't support the encoding fails the call with
  `UNIMPLEMENTED`. That includes an ntex-grpc server built without the
  feature.

## Limitations

* Only unary calls are supported. Client, server and bidirectional streaming
  rpcs are not.

## Examples

* [`examples/helloworld`](https://github.com/ntex-rs/ntex-grpc/tree/main/examples/helloworld): a server and a multi-threaded load-generating client.
* [`examples/custom`](https://github.com/ntex-rs/ntex-grpc/tree/main/examples/custom): a custom field type.

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
