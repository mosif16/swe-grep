use anyhow::Result;
use clap::Parser;

use swe_grep_mcp::bench;
use swe_grep_mcp::cli::{Cli, Commands};
use swe_grep_mcp::search;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Search(args) => {
            let summary = search::execute(args).await?;
            let json = serde_json::to_string_pretty(&summary)?;
            println!("{json}");
        }
        Commands::Bench(args) => {
            bench::run(args).await?;
        }
    }
    Ok(())
}
