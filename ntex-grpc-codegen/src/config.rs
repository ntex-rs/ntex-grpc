use std::{env, io, ops, path::Path};

use prost_build::Config;

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
