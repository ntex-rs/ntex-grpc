//! Thrift service code generator
#![deny(rust_2018_idioms, warnings)]
use std::{io, path, process::Command};

use clap::Parser;

mod config;
mod generator;

use self::config::NGrpcGenerator;

/// ntex grpc arguments
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Paths to .proto files to compile
    #[clap(value_parser, name = "PROTO")]
    proto: path::PathBuf,

    /// Paths to generated .rs file
    #[clap(value_parser, name = "OUT")]
    out: String,

    /// Configures the output directory where generated Rust files will be written.
    #[clap(short, long, value_parser, name = "OUT-DIR")]
    out_dir: Option<path::PathBuf>,

    /// Paths to directories in which to search for imports.
    #[clap(short, long, value_parser, name = "INCLUDE-DIR")]
    include_dir: Vec<path::PathBuf>,

    /// Map protobuf type to custom rust type that implements NativeType trait. {name}={rust-type-name}
    #[clap(short, long, value_parser, name = "MAP")]
    map: Vec<String>,

    /// Path to rustfmt configuration file
    #[clap(short, long, value_parser, name = "RUSTFMT-PATH")]
    rustfmt_path: Option<path::PathBuf>,
}

fn main() -> io::Result<()> {
    env_logger::init();

    let args = Args::parse();
    let mut cfg = NGrpcGenerator::new();

    if let Some(out_dir) = args.out_dir.clone() {
        cfg.out_dir(out_dir);
    }

    for map in args.map {
        if let Some((s1, s2)) = map.split_once('=') {
            cfg.map_field_type(s1, s2);
        } else {
            println!("Cannot parse type mapping: {map:?}");
        }
    }

    cfg.default_package_filename(args.out.clone());

    if let Err(e) = cfg.compile_protos(&args.proto, &args.include_dir) {
        println!("{e}");
    } else {
        // run rustfmt
        let fname = if let Some(mut out_dir) = args.out_dir.clone() {
            out_dir.push(args.out);
            out_dir.canonicalize().unwrap_or(out_dir)
        } else {
            args.out.into()
        };

        println!(
            "GRPC {:?} package successfully processed. Generated {fname:?} file",
            args.proto
        );

        let mut fmt_args = vec!["--edition", "2021"];

        let rustfmt_path = args.rustfmt_path.map(|s| s.to_string_lossy().to_string());
        if let Some(ref cfg_path) = rustfmt_path {
            fmt_args.push("--config-path");
            fmt_args.push(cfg_path.as_ref());
        }
        let fname = fname.to_string_lossy().to_string();
        fmt_args.push(fname.as_ref());

        let fmt_result = Command::new("rustfmt").args(&fmt_args).output();
        if let Err(e) = fmt_result {
            println!("{e}");
        }
    }

    Ok(())
}
