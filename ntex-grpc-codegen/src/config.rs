use std::{env, io, ops, path::Path};

use crate::builder::Config;
use crate::generator::GrpcServiceGenerator;

#[derive(Debug)]
pub(crate) struct NGrpcGenerator {
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

    /// Map protobuf type to custom rust type
    pub(crate) fn map_field_type(&mut self, path: &str, rust_type: &str) {
        let _ = self.pconfig.map_field_type([path], rust_type);
    }

    /// Compile a `.proto` file and its imports using the configured output directory.
    pub(crate) fn compile_protos(
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
