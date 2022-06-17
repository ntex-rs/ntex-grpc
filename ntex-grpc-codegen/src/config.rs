use std::{env, io, ops, path::Path, path::PathBuf};

use prost_build::Config;

use crate::generator::GrpcServiceGenerator;

#[derive(Debug)]
pub struct NGrpcGenerator {
    pconfig: Config,
    fmt_path: Option<PathBuf>,
}

impl NGrpcGenerator {
    pub(crate) fn new() -> Self {
        let mut cfg = Config::default();
        cfg.service_generator(Box::new(GrpcServiceGenerator));

        let current = env::current_dir().unwrap();
        let mut src = current.clone();
        src.push("src");
        cfg.out_dir(src);

        println!("{:?}", module_path!());

        Self {
            pconfig: cfg,
            fmt_path: None,
        }
    }

    /// Configures the path to custom rustfmt.toml file.
    pub fn rustfmt_path<P>(&mut self, path: P) -> &mut Self
    where
        P: Into<PathBuf>,
    {
        self.fmt_path = Some(path.into());
        self
    }

    /// Compile `.proto` files into Rust files during a Cargo build with additional code generator
    /// configuration options.
    pub fn compile_protos(
        &mut self,
        protos: &[impl AsRef<Path>],
        includes: &[impl AsRef<Path>],
    ) -> io::Result<()> {
        self.pconfig.compile_protos(protos, includes)?;

        Ok(())
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
