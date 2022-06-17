use std::{io, path::Path, process::Command};

fn main() -> io::Result<()> {
    let _ = ntex_grpc_codegen::configure().compile_protos(&["helloworld.proto"], &["./"]);

    // run rustfmt
    let _ = Command::new("rustfmt")
        .args(&[
            "--edition",
            "2018",
            AsRef::<Path>::as_ref("./src/helloworld.rs")
                .to_string_lossy()
                .as_ref(),
        ])
        .output();

    Ok(())
}
