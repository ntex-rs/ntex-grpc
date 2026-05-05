#!/bin/bash

ntex-grpc timestamp.proto timestamp.rs --out-dir ./src/google_types --include-dir ../prost-build/third-party/include/google/protobuf/ --well-known-types
ntex-grpc duration.proto duration.rs --out-dir ./src/google_types --include-dir ../prost-build/third-party/include/google/protobuf/ --well-known-types
ntex-grpc wrappers.proto wrappers.rs --out-dir ./src/google_types --include-dir ../prost-build/third-party/include/google/protobuf/ --well-known-types
