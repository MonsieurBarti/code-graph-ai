mod cli;
mod config;
mod walker;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Commands};
use config::CodeGraphConfig;
use walker::walk_project;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Index { path, verbose, json } => {
            let config = CodeGraphConfig::load(&path);
            let files = walk_project(&path, &config, verbose)?;

            if json {
                println!("{{\"files\": {}}}", files.len());
            } else {
                println!("Indexing {}...", path.display());
                println!("Found {} file(s).", files.len());
            }
        }
    }

    Ok(())
}
