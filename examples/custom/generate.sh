#!/bin/bash

# Generate grpc client for helloworld.proto
#
# To generate rust code use ntex-grpc-codegen crate
#
#   >> cargo install ntex-grpc-codegen
#
# This crate provides `ntex-grpc` command line utility

ntex-grpc helloworld.proto helloworld.rs --out-dir ./src --include-dir ./ \
          --map HelloRequest.msg_id=crate::unique_id::UniqueId
