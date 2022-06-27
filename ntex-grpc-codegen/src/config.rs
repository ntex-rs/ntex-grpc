use std::{env, io, ops, path::Path};

use ntex_prost_build::Config;

use crate::generator::GrpcServiceGenerator;

#[derive(Debug)]
pub struct NGrpcGenerator {
    pconfig: Config,
}

impl NGrpcGenerator {
    pub(crate) fn new() -> Self {
        let mut cfg = Config::default();
        cfg.service_generator(Box::new(GrpcServiceGenerator));

        let mut current = env::current_dir().unwrap();
        current.push("src");
        cfg.out_dir(current);

        Self { pconfig: cfg }
    }

    /// Map protobuf bytes type to custom rust type
    pub fn map_bytes(&mut self, path: &str, rust_type: &str) {
        let _ = self.pconfig.bytes(&[path], rust_type);
    }

    /// Compile `.proto` files into Rust files during a Cargo build with additional code generator
    /// configuration options.
    pub fn compile_protos(
        &mut self,
        proto: &impl AsRef<Path>,
        includes: &[impl AsRef<Path>],
    ) -> io::Result<()> {
        self.pconfig.compile_protos(&[proto], includes)
    }
}

impl ops::Deref for NGrpcGenerator {
    type Target = Config;

    fn deref(&self) -> &Config {
        &self.pconfig
    }
}

impl ops::DerefMut for NGrpcGenerator {
    fn deref_mut(&mut self) -> &mut Config {
        &mut self.pconfig
    }
}
