// build.rs - Protocol Buffer Compilation

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR")?);
    let descriptor_path = out_dir.join("fipa_descriptor.bin");

    // Compile protobuf files
    tonic_prost_build::compile_protos("src/proto/fipa.proto")?;

    // Generate file descriptor set for gRPC reflection
    let protoc = std::env::var("PROTOC").unwrap_or_else(|_| "protoc".to_string());
    std::process::Command::new(&protoc)
        .arg("--descriptor_set_out")
        .arg(&descriptor_path)
        .arg("--include_imports")
        .arg("--proto_path=src/proto")
        .arg("src/proto/fipa.proto")
        .status()?;

    // Tell Cargo to rerun if proto file changes
    println!("cargo:rerun-if-changed=src/proto/fipa.proto");

    Ok(())
}
