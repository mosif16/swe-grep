fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/swegrep.proto");
    println!("cargo:rerun-if-changed=proto");

    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    // Safety: build scripts run single-threaded; this mutation is scoped to the current process
    // so it is safe to install the vendored `protoc` path.
    unsafe {
        std::env::set_var("PROTOC", &protoc);
    }

    tonic_build::configure()
        .build_server(true)
        .build_client(true)
        .compile(&["proto/swegrep.proto"], &["proto"])?;

    Ok(())
}
