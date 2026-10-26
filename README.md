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
ntex-grpc = "4"
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
        ClientError::Status(status) => {
            println!("failed with {:?}: {}", status.code, status.message())
        }
        ClientError::NotProcessed(_) => println!("safe to retry: {err}"),
        _ => println!("request failed with {:?}: {err}", err.status()),
    },
}
```

A non-OK status from the server comes back as `ClientError::Status`, with the
trailers as `metadata`. The other variants are local failures: `InvalidRequest`,
`NotProcessed` (the server did not process the call, it is safe to retry),
`Connection`, `StreamFailed`, `NotGrpc` (the response is not gRPC, e.g. an
HTTP error from a proxy), `InvalidResponse` and `DeadlineExceeded` (the client
gave up when the timeout ran out). `err.status()` and `err.message()` give the
gRPC status and message for any of them.

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
`service`, `name`, `headers` and `message` fields come first, so a message field with one
of these names has to be read as `req.message.name`.

A method can also return `Result<HelloReply, E>` if `E: Into<HelloReply>`. The
error is turned into a reply, so the client still sees `grpc-status: 0`.

The server sends a gRPC status itself in these cases:

* `UNIMPLEMENTED` for an unknown service or method, a path that is not
  `/service/method` or an rpc that has no `#[method]`;
* `INVALID_ARGUMENT` if the request message doesn't decode;
* `INVALID_ARGUMENT` with HTTP status 415 if the request `content-type` is not
  `application/grpc`;
* `INTERNAL` with HTTP status 405 if the request method is not `POST`;
* `DEADLINE_EXCEEDED` if the client's `grpc-timeout` runs out before the method
  finishes.

### Multiple services

`GrpcServer::new()` takes the calls for one service, add more services on the
same port with `service()`. Calls are routed by the service name in the
request path, so `/helloworld.Greeter/SayHello` goes to the service added for
`helloworld.Greeter`:

```rust
server::GrpcServer::new(GreeterServer)
    .service(CalculatorServer)
```

`#[server]` implements `server::NamedService` with the full service name from
the `.proto` file. For a hand-written service use
`service_named("helloworld.Greeter", factory)`. A call to a service that is
not added fails with `UNIMPLEMENTED` before the request body is read, unless
`default_service(factory)` is set, then it takes these calls.

The server creates an instance of every service for each connection, the
connection fails if one of them cannot be created.

`GrpcServer` is an ntex service that takes an I/O stream, so TLS works the
usual ntex way. Chain a TLS acceptor that negotiates `h2` with ALPN in front of
it.

## Streaming

Client, server and bidirectional streaming rpcs use two types that are the
same on the client and on the server. `SendStream<T>` sends messages with
`send(&msg).await` and ends with `close()`, dropping every clone of it ends
the stream too. `RecvStream<T>` gives the messages with `recv().await`, it
returns `None` at the end of the stream.

The [routeguide](examples/routeguide) example has a server and a client with
all four kinds of rpcs.

The generated client opens a bidi call with `open()`:

```rust
let (tx, mut rx) = client.route_chat().open().await?;
tx.send(&note).await?;
tx.close();
while let Some(msg) = rx.recv().await {
    println!("{:?}", msg?);
}
// the status and the trailers, once the stream has ended
println!("{:?} {:?}", rx.status(), rx.trailers());
```

A server streaming method takes the request message and `open()` returns the
`RecvStream` of the response. A client streaming method takes a `SendStream`
and `send()` returns the response, so the messages are sent while `send()` is
polled:

```rust
let tx = SendStream::new();
let (res, sent) = ntex::util::join(client.record_route(&tx).send(), async {
    for point in points {
        tx.send(&point).await?;
    }
    tx.close();
    Ok::<_, StreamError>(())
})
.await;
```

`header()`, `timeout()`, `compression()` and the size limits are set on the
request before `open()` or `send()`, like for a unary call.

In a `#[server]` service the kind of a method comes from its arguments, it
has to match the rpc in the `.proto` file or the service doesn't build:

```rust
#[server(crate::route::RouteGuide)]
impl RouteGuideServer {
    // server streaming
    #[method(ListFeatures)]
    async fn list_features(&self, req: Rectangle, tx: SendStream<Feature>) {
        for feature in self.features(&req) {
            if tx.send(&feature).await.is_err() {
                return;
            }
        }
    }

    // client streaming
    #[method(RecordRoute)]
    async fn record_route(&self, mut rx: RecvStream<Point>) -> RouteSummary {
        let mut summary = RouteSummary::default();
        while let Some(Ok(point)) = rx.recv().await {
            summary.point_count += 1;
        }
        summary
    }

    // bidirectional streaming
    #[method(RouteChat)]
    async fn route_chat(
        &self,
        mut rx: RecvStream<RouteNote>,
        tx: SendStream<RouteNote>,
    ) -> Result<(), ServerError> {
        while let Some(note) = rx.recv().await {
            tx.send(&note?).await?;
        }
        Ok(())
    }
}
```

A server streaming method can take `server::Request<T>` instead of the
message. Server and bidi methods return `()` or `Result<(), E>` with
`E: Into<ServerError>`, an error ends the call with its status. `rx.headers()`
has the request headers, `tx.header()` adds a response header before the
first message and `tx.trailer()` adds a trailer.

* The method future is dropped when the client cancels the call or the
  `grpc-timeout` runs out, the timeout is checked while the method polls its
  streams.
* `SendStream::send()` waits for flow-control capacity, so a slow reader
  holds up the sender, `ready()` doesn't wait for it. Dropping the `send()`
  future while the message is written cancels the call.
* Streaming calls go only to `#[server]` services, a service added with
  `service_named()` or `default_service()` gets only unary calls.
* Streaming messages are encoded and decoded with `Message`, the
  `MethodDef::encode()` and `decode()` overrides are used only for unary calls.

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

## Message-size limits

The client accepts response messages up to 4 MiB and sends request messages
up to 2 GiB - 1 by default. Set client-wide defaults on the transport:

```rust
let transport = ntex_grpc::client::Client::new(h2client)
    .max_message_size(16 * 1024 * 1024)
    .max_send_message_size(8 * 1024 * 1024);
let client = GreeterClient::new(transport);
```

These limits apply to each message in unary and streaming calls. Override
either limit for one call using `Request::max_message_size()` /
`max_send_message_size()` or the same methods on `StreamRequest`:

```rust
let mut req = client.say_hello(&msg);
req.max_message_size(32 * 1024 * 1024);
let res = req.send().await.unwrap();
```

`GrpcServer` has the same setters: `max_message_size()` limits received
request messages, and `max_send_message_size()` limits response messages.
Zero allows only empty encoded messages; it does not disable the limit.

## Compression

gzip and zstd message compression comes with the `compression` feature:

```toml
ntex-grpc = { version = "4", features = ["compression"] }
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
compresses the response the same way as the request, uncompressed if the
request's `grpc-accept-encoding` leaves that encoding out. Both sides send
`grpc-accept-encoding: gzip,zstd`, and the client decompresses responses in
either encoding.

* Empty messages, messages under 64 bytes and messages that don't get smaller
  are sent uncompressed.
* Size limits apply after decompression. `max_message_size()` on the client,
  request and `GrpcServer` covers the decompressed message, and a larger one
  fails with `RESOURCE_EXHAUSTED`. `max_send_message_size()` covers the
  message as it is sent, after compression.
* Large messages are compressed and decompressed on the runtime's blocking
  thread pool, so they don't hold up the worker thread.
* A server that doesn't support the encoding fails the call with
  `UNIMPLEMENTED`. That includes an ntex-grpc server built without the
  feature.

## Limitations

* Unknown fields are skipped when a message is decoded. A message that is
  decoded and encoded again drops fields that are not in the generated code,
  for example fields added in a newer version of the proto file.
* Singular message fields are generated as plain values, not `Option<T>`, so
  presence is not tracked. A missing message field decodes as its default
  value, and a message field is always encoded, as an empty message when it
  holds the default. Run the generator with `--optional-messages` to get
  `Option<T>` fields. Recursive message fields are always `Option<Box<T>>`.
* Enum fields have the generated enum type, a value unknown to the enum
  decodes as the default variant and is lost when the message is encoded
  again. Run the generator with `--open-enums` to get `i32` fields that keep
  unknown values.

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
