fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/v1/common.proto",
                "proto/v1/channel.proto",
                "proto/v1/node.proto",
                "proto/v1/measurement.proto",
                "proto/v1/health.proto",
                "proto/v1/ingest.proto",
            ],
            &["proto"],
        )?;
    Ok(())
}
