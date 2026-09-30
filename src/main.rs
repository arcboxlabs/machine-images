//! CLI entry point.

use std::num::NonZeroUsize;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use machine_images::config::Config;
use machine_images::index::Index;
use machine_images::{sync, verify};

#[derive(Parser)]
#[command(
    name = "machine-images",
    about = "Mirror ArcBox Linux machine images to the CDN staging layout",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Mirror new upstream builds into a staging directory.
    Sync {
        /// Mirror configuration file.
        #[arg(long, default_value = "mirror.toml")]
        config: PathBuf,
        /// Staging directory (the future namespace root on the CDN).
        #[arg(long, default_value = "dist")]
        output: PathBuf,
        /// Currently published index.json; omitted bootstraps an empty index.
        #[arg(long)]
        state_file: Option<PathBuf>,
        /// Write a machine-readable summary JSON here.
        #[arg(long)]
        summary: Option<PathBuf>,
        /// Override the `updated_at` timestamp (testing).
        #[arg(long)]
        now: Option<String>,
        /// Override `mirror.keep_versions` for this run. Bootstrapping an empty
        /// CDN with `1` keeps the first publish within a runner's disk.
        #[arg(long)]
        keep_versions: Option<NonZeroUsize>,
    },
    /// Verify a staging directory against its manifests and index.
    Verify {
        /// Staging directory to verify.
        #[arg(long, default_value = "dist")]
        dir: PathBuf,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Sync {
            config,
            output,
            state_file,
            summary,
            now,
            keep_versions,
        } => {
            let mut config = Config::load(&config)?;
            if let Some(keep) = keep_versions {
                config.mirror.keep_versions = keep.get();
            }
            let state = match &state_file {
                Some(path) => serde_json::from_str(&fs_err::read_to_string(path)?)
                    .with_context(|| format!("parse state {}", path.display()))?,
                None => {
                    eprintln!("no --state-file: bootstrapping an empty index");
                    Index::default()
                }
            };
            let now =
                now.unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

            let result = sync::sync(&config, state, &output, &now)?;
            for build in &result.staged {
                println!(
                    "+ {}@{} ({} bytes)",
                    build.stream, build.version, build.bytes
                );
            }
            println!("{} new build(s)", result.new_versions);

            if let Some(path) = summary {
                let mut json = serde_json::to_string_pretty(&result)?;
                json.push('\n');
                fs_err::write(path, json)?;
            }
        }
        Command::Verify { dir } => {
            let report = verify::verify(&dir)?;
            println!(
                "ok: {} build(s) verified, {} retained index entr(ies) without local files",
                report.verified, report.retained
            );
        }
    }
    Ok(())
}
