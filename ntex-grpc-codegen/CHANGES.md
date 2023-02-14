# Changes

## [0.2.6] - 2023-02-14

* Fix codegen for "oneof" default value serialization

## [0.2.5] - 2023-01-13

* Add "clippy::derive_partial_eq_without_eq" to generated files

## [0.2.4] - 2022-12-22

* Support f32, f64 #2

* Support Timestamp and Duration google types #3

## [0.2.2] - 2022-12-12

* Separate trait impls for generated code

## [0.2.1] - 2022-11-29

* Fix split prefix feature

## [0.2.0-b.1] - 2022-11-21

* Do not use Option for message fields if not explicitly set

## [0.2.0-b.0] - 2022-11-15

* Generate code for ntex-grpc 0.2

## [0.1.6] - 2022-10-07

* Fix service name generation for empty package

## [0.1.5] - 2022-07-08

* Allow to replace any protobuf field with custom rust type

## [0.1.4] - 2022-07-06

* Generate service defs manually

* Replace --map-bytes and --map-strings with --map

## [0.1.3] - 2022-07-06

* Generate code instead of proc macro

## [0.1.2] - 2022-07-01

* Add support for server

* Refactor protobuf handling

## [0.1.1] - 2022-06-28

* Fix oneof, map types

* Fix ntex-grpc cmd util

## [0.1.0] - 2022-06-27

* Initial release
