mod cli;
mod search;
mod tools;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Search(args) => {
            let summary = search::execute(args).await?;
            let json = serde_json::to_string_pretty(&summary)?;
            println!("{json}");
        }
    }
    Ok(())
}
