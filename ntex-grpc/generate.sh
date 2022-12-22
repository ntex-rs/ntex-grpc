#!/bin/bash

ntex-grpc timestamp.proto timestamp.rs --out-dir ./src/google_types --include-dir ../prost-build/third-party/include/google/protobuf/
ntex-grpc duration.proto duration.rs --out-dir ./src/google_types --include-dir ../prost-build/third-party/include/google/protobuf/
