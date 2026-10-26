#!/bin/bash

# Generate grpc client for route_guide.proto
#
# To generate rust code use ntex-grpc-codegen crate
#
#   >> cargo install ntex-grpc-codegen
#
# This crate provides `ntex-grpc` command line utility

ntex-grpc route_guide.proto routeguide.rs --out-dir ./src --include-dir ./
