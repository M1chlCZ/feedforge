//! Command line interface for the feedforge product feed generator.

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use feedforge::{Config, generate};

#[derive(Debug, Parser)]
#[command(
    name = "feedforge",
    version,
    about = "Streaming product feed generator for Heureka, Zbozi.cz and Google Merchant."
)]
struct Cli {
    /// Path to the TOML configuration file.
    #[arg(long, value_name = "FILE")]
    config: PathBuf,
    /// Path to the JSON catalog snapshot.
    #[arg(long, value_name = "FILE")]
    input: PathBuf,
    /// Output XML path. Overrides the config `output` value.
    #[arg(long, value_name = "FILE")]
    output: Option<PathBuf>,
    /// Path for the JSON report of written and skipped products.
    #[arg(long, value_name = "FILE")]
    report: Option<PathBuf>,
    /// Exit with status code 1 when any product is skipped.
    #[arg(long)]
    strict: bool,
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("feedforge: {error:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let config = Config::from_file(&cli.config)
        .with_context(|| format!("cannot load config {}", cli.config.display()))?;
    let output_path = cli
        .output
        .or_else(|| config.output.clone())
        .ok_or_else(|| anyhow!("no output path: set `output` in the config or pass --output"))?;
    let input = File::open(&cli.input)
        .with_context(|| format!("cannot open input {}", cli.input.display()))?;
    let output = File::create(&output_path)
        .with_context(|| format!("cannot create output {}", output_path.display()))?;
    let outcome = generate(&config, BufReader::new(input), BufWriter::new(output))
        .with_context(|| format!("cannot generate feed {}", output_path.display()))?;
    if let Some(report_path) = &cli.report {
        let report = File::create(report_path)
            .with_context(|| format!("cannot create report {}", report_path.display()))?;
        serde_json::to_writer_pretty(BufWriter::new(report), &outcome)
            .with_context(|| format!("cannot write report {}", report_path.display()))?;
    }
    println!(
        "written {} product(s), skipped {}",
        outcome.written,
        outcome.skipped_count()
    );
    for entry in &outcome.skipped {
        eprintln!(
            "skipped product #{} {}: {}",
            entry.index,
            entry.id.as_deref().unwrap_or("<no id>"),
            entry.reasons.join(", ")
        );
    }
    if cli.strict && outcome.skipped_count() > 0 {
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}
