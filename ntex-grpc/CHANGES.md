# Changes

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
