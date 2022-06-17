//! Thrift service code generator
// #![deny(rust_2018_idioms, warnings)]
#![allow(dead_code, unused_variables)]

mod config;
mod generator;

pub use self::config::NGrpcGenerator;

pub fn configure() -> NGrpcGenerator {
    NGrpcGenerator::new()
}

fn snake_case(s: &str) -> String {
    let mut ident = String::new();
    for ch in s.chars() {
        if ch.is_uppercase() {
            if !ident.is_empty() {
                ident.push('_');
            }
            for ch in ch.to_lowercase() {
                ident.push(ch);
            }
        } else {
            ident.push(ch);
        }
    }
    ident
}
