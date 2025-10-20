use anyhow::Result;

use crate::cli::ServeArgs;
use crate::telemetry;

pub mod grpc;
pub mod http;
pub mod server;

pub mod proto {
    tonic::include_proto!("swegrep.v1");
}

/// Launch the combined HTTP and gRPC services using the provided CLI arguments.
pub async fn serve(args: ServeArgs) -> Result<()> {
    telemetry::init()?;
    let config = server::ServeConfig::try_from_args(args)?;
    let server = server::SweGrepServer::new(config);
    server.run().await
}
