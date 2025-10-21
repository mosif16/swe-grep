use anyhow::Result;
use clap::Parser;

use swe_grep::bench;
use swe_grep::cli::{Cli, Commands};
use swe_grep::search;
use swe_grep::service;
use swe_grep::telemetry;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if !cli.disable_telemetry {
        telemetry::init()?;
    }
    match cli.command {
        Commands::Search(args) => {
            let summary = search::execute(args).await?;
            let json = serde_json::to_string_pretty(&summary)?;
            println!("{json}");
        }
        Commands::Bench(args) => {
            bench::run(args).await?;
        }
        Commands::Serve(args) => {
            service::serve(args).await?;
        }
    }
    Ok(())
}
