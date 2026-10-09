# Changes

## [3.2.0] - Unreleased

* Simplify message compression: compression errors reach the client and
  server as a gRPC status, the same way as decompression errors

* Use less memory for compressed messages. A zstd message is decoded straight
  into its buffer, in one pass when the frame stores its size, gzip no longer
  copies the input through a 32 KiB buffer, and the compressed message is
  written into pages instead of a growing buffer. zstd uses a 512 KiB window,
  and the buffer for a decompressed message is at most 64 times the size of
  the compressed one until the data arrives

* Each worker thread keeps its gzip and zstd encoders and decoders for the
  next message, which makes compressing a small message up to 40% faster. A
  zstd context larger than 256 KiB is dropped, and threads of the blocking
  pool keep none. Requires flate2 1.1.3

* A zstd message whose frame does not store its size is decoded in a single
  pass, into a buffer for as much as its blocks can hold. The decoder no
  longer buffers a 2 MiB window for it

* A zstd message that compresses more than 64 times is decoded in a single
  pass, the size its frame stores is trusted as far as its blocks can hold

* Messages under 64 bytes, and messages that do not get smaller, are sent
  uncompressed even if the call uses compression

* Server gives back the memory of its map of open streams after a burst of
  requests, instead of keeping it for the life of the connection

* Server keeps the request path of an unfinished request instead of separate
  service and method names, which makes the state of a request 24 bytes
  smaller

* Server sends an empty response message from a static buffer instead of
  allocating a page for its 5-byte prefix

* Server reports a request without a message and a truncated request message
  with different `grpc-message` texts

* Add the `compression` feature with gzip and zstd message compression. Client
  and server accept compressed messages and send
  `grpc-accept-encoding: gzip,zstd`. `Request::compression()` and
  `RequestContext::compression()` compress the request message, the server
  compresses a response with the request's `grpc-encoding`. A message over the
  size limit after decompression fails with `RESOURCE_EXHAUSTED`. Large
  messages are compressed and decompressed on the blocking thread pool

* Compress and decompress messages into a single buffer instead of copying the
  result once more. Decompression allocates the size stored in the message up
  front, the zstd encoder stores the message size in the frame. A message
  that spans several pages is compressed page by page instead of being joined
  into one buffer first. A message is decompressed once: the compressed size
  decides whether that happens in place or on the thread pool

* Server checks the request message length as data arrives and rejects a
  message over the limit before buffering the body, then resets the stream so
  the client stops sending. Data after the request message is rejected with
  `INTERNAL` instead of being ignored

* Client keeps at most 64 KiB of a response body that is not a grpc one, e.g.
  an error page from a proxy, and stops reading the response there

* A message split over several data frames is read into a buffer sized for
  the whole message once its length is known, instead of growing as data
  arrives. The first frame is reused when nothing else holds it

* Server rejects a request message larger than 4 MiB with
  `RESOURCE_EXHAUSTED`. Add `GrpcServer::max_message_size()` to change the
  limit

* `Request::header()` and `RequestContext::header()` ignore headers the client
  sets itself: `content-type`, `user-agent`, `te`, `grpc-encoding`,
  `grpc-message-type`, `grpc-message`, `grpc-status` and `grpc-timeout`. They
  replaced the client's values before. Other headers are sent after the client's
  ones, a `grpc-accept-encoding` set by the user is sent along with `identity`

* Client reports a connection that is closed, fails or goes away during a call
  as `ClientError::GrpcStatus` with `UNAVAILABLE` instead of
  `ClientError::Operation`

* Client does not send a request message larger than 2 GiB - 1 and fails with
  `RESOURCE_EXHAUSTED`, a message of 4 GiB or more got a truncated length
  prefix. Add `Request::max_send_message_size()` and
  `RequestContext::max_send_message_size()` to change the limit

* Add `Request::append_header()` and `RequestContext::append_header()` to send
  several values for one metadata key

* Add `encode_binary_header()` and `decode_binary_header()` for `-bin` metadata
  values, which are base64 encoded

* `RequestContext::headers()` returns `impl Iterator` instead of
  `impl ExactSizeIterator`, it yields every value of a key

* Client fails a response with data after the message with `INTERNAL`, it was
  ignored

* Client limits the size of a received message to 4 MiB, larger messages fail
  with `RESOURCE_EXHAUSTED`. Add `Request::max_message_size()` and
  `RequestContext::max_message_size()` to change it

* Client maps a stream reset by the server to a gRPC code as the spec says and
  returns `ClientError::GrpcStatus` instead of `ClientError::Stream`

* Client reads the reply if the server resets the stream before the request is
  sent, a server can reply early

* Client fails a call with a zero timeout with `DeadlineExceeded` without
  sending it

* Client accepts only HTTP status 200, other 2xx statuses are reported as
  `UNKNOWN`

* Client sends `user-agent: grpc-rust-ntex/<version>` with the crate version,
  was `ntex-grpc/1.0.0`

* Add `ClientError::grpc_message()`, returns the percent-decoded `grpc-message`

* Client reports an unknown or invalid `grpc-status` as
  `ClientError::GrpcStatus` with `UNKNOWN` instead of `ClientError::Decode`

* Client fails a compressed reply with `ClientError::GrpcStatus` and `INTERNAL`,
  only identity is supported

* Server answers a compressed request with `UNIMPLEMENTED`, or `INTERNAL` if
  `grpc-encoding` is missing, and sends `grpc-accept-encoding: identity`

* Server no longer panics on a request body shorter than 5 bytes

* Client maps an HTTP status other than 200 without `grpc-status` to a gRPC code
  as the spec says and returns `ClientError::GrpcStatus` instead of
  `ClientError::Response`

* `ClientError::GrpcStatus` holds the response body when the client picked the
  status

* Client fails a response without `grpc-status` with `ClientError::GrpcStatus`,
  `UNKNOWN` if the trailers lack it, `INTERNAL` if there are no trailers

* Client fails a response whose `content-type` is not `application/grpc` with
  `ClientError::GrpcStatus` and `UNKNOWN`

* Client enforces the request timeout, returns `ClientError::DeadlineExceeded`
  with empty headers and resets the stream

* Make `RequestContext::headers()` and `get_disconnect_on_drop()` public, custom
  transports need them

* Fix code examples in `google_types` docs being run as Rust doctests

* Document all public items, add crate level docs

* Fix client `Response::res_size`, it reported the leftover bytes instead of the
  response size

* Fix client panic on a response with a body shorter than the gRPC frame prefix

* Fix `ClientError::DeadlineExceeded` headers for headers-only responses

* Fix `Debug` for client `Response` printing headers as trailers

* Add `Display` for `ClientError::Client`

* `RequestContext::clear()` also resets the timeout, it already removed the
  `grpc-timeout` header

* Rename `GrpcStatus::AlredyExists` to `GrpcStatus::AlreadyExists`.
  `signature()` now returns `grpc-status-AlreadyExists`

* Export `NegativeDurationError` and `OutOfRangeDurationError`, implement
  `Error` for both

* Server resets a stream with `INTERNAL_ERROR` when its trailers exceed the
  peer's `SETTINGS_MAX_HEADER_LIST_SIZE`

* Client returns `ClientError::Http` for an invalid request header instead of
  dropping it, add `RequestContext::take_error()`

* Update to ntex-error 3.0, ntex-h2 4.2 and ntex-http 2.0

## [3.0.0] - 2026-09-14

* Update to ntex 4.0

## [2.3.0] - 2026-07-22

* RequestContext::header() overrides existing headers

## [2.2.0] - 2026-05-15

* Simplify send request process

## [2.1.0] - 2026-05-15

* Add NativeType impl for Arc<str>

## [2.0.0] - 2026-05-05

* Use new codec api with BytePages support

## [1.5.2] - 2026-04-02

* Update ntex-error to 2.0

## [1.5.1] - 2026-03-26

* Update ntex_error::Error

## [1.5.0] - 2026-03-08

* Use ntex_error::Error

## [1.4.1] - 2026-02-25

* Do not serialize empty vec #34

## [1.4.0] - 2026-02-17

* SharedCfg is not Copy

## [1.3.0] - 2026-02-02

* Upgrade to ntex v3.0

## [1.2.0] - 2025-12-22

* Add "CONTENT-TYPE" header to server response

## [1.1.0] - 2025-12-17

* Upgrade to ntex-service v4

## [1.0.0-pre.0] - 2025-11-28

* Update the `ntex` to 3.0

* Update MSRV to 1.85

* Update edition to 2024

## [0.7.6] - 2025-07-08

* Better error handling

* Make DecodeError clonable

## [0.7.5] - 2025-02-13

* Handle DeadlineExceeded status

## [0.7.4] - 2025-01-31

* Export google types

* Add default client transport for h2::Client and h2::SimpleClient

## [0.7.3] - 2025-01-30

* Add "disconnect on drop" support for client

## [0.7.2] - 2025-01-27

* Add grpc timeout support

## [0.7.1] - 2024-12-10

* Refactor server error handling

## [0.7.0] - 2024-05-28

* Upgrade to ntex v2.0

## [0.6.4] - 2024-05-16

* Fix f32/f64 encoding

## [0.6.3] - 2024-03-25

* Remove ntex-connect dependency

## [0.6.2] - 2024-02-01

* Handle broken protobuf frames

* Fix Vec<_> encoding

## [0.6.1] - 2024-01-17

* Add support for f32 and f64 types #4

## [0.6.0] - 2024-01-09

* Release

## [0.6.0-b.0] - 2024-01-07

* Use "async fn" in trait for Service definition

## [0.5.0] - 2023-10-09

* Migrate to ntex-h2 0.4

## [0.4.0] - 2023-06-22

* Release v0.4.0

## [0.4.0-beta.2] - 2023-06-19

* .get_ref() instead of Deref for service container

## [0.4.0-beta.1] - 2023-06-19

* Use ServiceCtx instead of Ctx

## [0.4.0-beta.0] - 2023-06-17

* Migrate to ntex 0.7

## [0.3.8] - 2023-05-05

* Fix handling Vec of varint

## [0.3.7] - 2023-05-03

* Fix handling Vec of varint types for server

## [0.3.6] - 2023-05-02

* Fix handling Vec of varint types

## [0.3.5] - 2023-04-06

* Fix panic on error after stream eof

## [0.3.4] - 2023-02-27

* Add google wrapper types

## [0.3.3] - 2023-01-13

* Handle request's future drop

## [0.3.2] - 2023-01-10

* Handle default values in HashMap

## [0.3.1] - 2023-01-09

* Handle default values in Vec<T>

* Handle not enough data to decode server message

## [0.3.0] - 2023-01-04

* 0.3 Release

## [0.3.0-beta.0] - 2022-12-28

* Use GAT for Transport trait

* Migrate to ntex-service 1.0

## [0.2.3] - 2022-12-22

* Fix NativeType impl for HashMap

## [0.2.2] - 2022-12-22

* Add Timestampt and Duration google types #3

## [0.2.1] - 2022-12-04

* Try to extract GrpcError instead of UnexpecetedEof

## [0.2.0] - 2022-11-23

* Refactor code layout

* Allow to access request and create custom responses for server

## [0.2.0-b.2] - 2022-11-15

* Fix Option<T> encodinging

## [0.2.0-b.1] - 2022-11-14

* Fix default value for Option<T>, None is always default

## [0.2.0-b.0] - 2022-11-01

* Add request context for client calls

## [0.1.4] - 2022-10-31

* Add Message impl for ()

## [0.1.3] - 2022-07-13

* Disconnect on client drop

## [0.1.2] - 2022-07-12

* Better client error handling

## [0.1.1] - 2022-07-08

* Export custom HashMap type for auto-gen code

## [0.1.0] - 2022-07-07

* Initial release
