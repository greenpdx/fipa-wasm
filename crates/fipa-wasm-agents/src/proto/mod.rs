// Proto module - Generated Protocol Buffer types

// Include generated protobuf code from tonic_build
// The code is generated into OUT_DIR during build
pub mod fipa {
    pub mod v1 {
        tonic::include_proto!("fipa.v1");
    }
}

// Re-export common types for convenience
pub use fipa::v1::*;

/// File descriptor set for gRPC reflection
pub const FILE_DESCRIPTOR_SET: &[u8] = tonic::include_file_descriptor_set!("fipa_descriptor");
