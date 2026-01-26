//! NIM Usage Scanner
//!
//! A static code analyzer that scans repositories to discover and catalog
//! NVIDIA NIM usage (Local NIM containers and Hosted NIM endpoints).

mod config;
mod git_ops;
mod models;
mod ngc_api;
mod report;
mod scanner;

use std::path::PathBuf;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use log::{info, warn, error, LevelFilter};
use std::process::Command;
use tempfile::TempDir;

use crate::models::ScanReport;

/// NIM Usage Scanner - Detect NVIDIA NIM usage across repositories
#[derive(Parser, Debug)]
#[command(name = "nim-usage-scanner")]
#[command(author = "NVIDIA")]
#[command(version)]
#[command(about = "Static code analyzer that scans repositories to discover and catalog NVIDIA NIM usage")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Scan repositories for NIM usage
    Scan(ScanArgs),
    
    /// Query Hosted NIM information by model name
    Query(QueryArgs),
}

/// Arguments for the scan subcommand
#[derive(Parser, Debug)]
struct ScanArgs {
    /// Path to the repos.yaml configuration file
    #[arg(short, long)]
    config: PathBuf,

    /// Output directory for reports
    #[arg(short, long, default_value = "./output")]
    output: PathBuf,

    /// NGC API key for enrichment (optional, or use NVIDIA_API_KEY env var)
    #[arg(long, env = "NVIDIA_API_KEY")]
    ngc_api_key: Option<String>,

    /// GitHub token for cloning private repositories (optional, or use GITHUB_TOKEN env var)
    #[arg(long, env = "GITHUB_TOKEN")]
    github_token: Option<String>,

    /// Working directory for cloning repositories
    #[arg(short, long)]
    workdir: Option<PathBuf>,

    /// Keep cloned repositories after scanning
    #[arg(long, default_value = "false")]
    keep_repos: bool,

    /// Increase logging verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Maximum number of parallel jobs
    #[arg(short, long)]
    jobs: Option<usize>,

    /// Regenerate repos.yaml from Build Page before scanning
    #[arg(long, default_value = "false")]
    refresh_repos: bool,
}

/// Arguments for the query subcommand
#[derive(Parser, Debug)]
struct QueryArgs {
    /// Query type: hosted-nim or local-nim
    #[command(subcommand)]
    query_type: QueryType,
}

#[derive(Subcommand, Debug)]
enum QueryType {
    /// Query Hosted NIM information (Function ID, status, containerImage, etc.)
    HostedNim(HostedNimQueryArgs),
    
    /// Query Local NIM information (latest tag, description, etc.)
    LocalNim(LocalNimQueryArgs),
}

/// Arguments for querying Hosted NIM
#[derive(Parser, Debug)]
struct HostedNimQueryArgs {
    /// Model name to query (e.g., "nvidia/llama-3.1-nemotron-70b-instruct")
    #[arg(short, long)]
    model: String,

    /// NGC API key (required, or use NVIDIA_API_KEY env var)
    #[arg(long, env = "NVIDIA_API_KEY", required = true)]
    ngc_api_key: String,

    /// Increase logging verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

/// Arguments for querying Local NIM
#[derive(Parser, Debug)]
struct LocalNimQueryArgs {
    /// Image name to query (e.g., "nvidia/llama-3.2-nv-embedqa-1b-v2")
    /// Format: <team>/<model-name> (without nvcr.io/nim/ prefix)
    #[arg(short, long)]
    image: String,

    /// NGC API key (required, or use NVIDIA_API_KEY env var)
    #[arg(long, env = "NVIDIA_API_KEY", required = true)]
    ngc_api_key: String,

    /// Increase logging verbosity (-v, -vv, -vvv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn init_logging(verbosity: u8) {
    let level = match verbosity {
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new()
        .filter_level(level)
        .format_timestamp_secs()
        .init();
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Scan(args) => run_scan(args),
        Commands::Query(args) => run_query(args),
    }
}

/// Run the scan subcommand
fn run_scan(args: ScanArgs) -> Result<()> {
    // Initialize logging (info level by default for scan)
    init_logging(args.verbose + 1);
    
    info!("NIM Usage Scanner starting...");
    info!("Config file: {}", args.config.display());
    info!("Output directory: {}", args.output.display());
    
    // Set rayon thread pool size if specified
    if let Some(jobs) = args.jobs {
        rayon::ThreadPoolBuilder::new()
            .num_threads(jobs)
            .build_global()
            .context("Failed to set thread pool size")?;
        info!("Using {} parallel jobs", jobs);
    }
    
    if args.refresh_repos {
        info!("Refreshing repos from Build Page...");
        let status = Command::new("python3")
            .arg("scripts/generate_repos_from_ngc.py")
            .arg("--output")
            .arg(&args.config)
            .status()
            .context("Failed to run Build Page repo generation script")?;
        if !status.success() {
            bail!("Build Page repo generation script failed");
        }
    }

    // Load and validate configuration
    info!("Loading configuration...");
    let config = config::load_config(&args.config)
        .context("Failed to load configuration")?;
    
    config::validate_config(&config)
        .context("Configuration validation failed")?;
    
    // Apply defaults and filter enabled repos
    let repos = config::apply_defaults(&config);
    let repos = config::filter_enabled(repos);
    
    if repos.is_empty() {
        warn!("No enabled repositories found in configuration");
        return Ok(());
    }
    
    info!("Found {} enabled repositories to scan", repos.len());
    
    // Create working directory
    let temp_dir: Option<TempDir>;
    let workdir = if let Some(ref dir) = args.workdir {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create workdir: {}", dir.display()))?;
        temp_dir = None;
        dir.clone()
    } else {
        let td = TempDir::new().context("Failed to create temp directory")?;
        let path = td.path().to_path_buf();
        temp_dir = Some(td);
        path
    };
    
    info!("Working directory: {}", workdir.display());
    
    if args.github_token.is_none() {
        warn!("No GitHub token provided; private repositories may fail to clone");
    }

    // Clone repositories
    info!("Cloning repositories...");
    let clone_results = git_ops::clone_all_repos(&repos, &workdir, args.github_token.as_deref());
    
    let (success_count, failed_count) = git_ops::clone_stats(&clone_results);
    info!("Clone complete: {} succeeded, {} failed", success_count, failed_count);
    
    // Log failed clones
    for result in &clone_results {
        if let Some(ref err) = result.error {
            error!("Failed to clone {}: {}", result.repo.name, err);
        }
    }
    
    // Scan repositories
    info!("Scanning repositories for NIM references...");
    let mut all_local = Vec::new();
    let mut all_hosted = Vec::new();
    
    for result in &clone_results {
        if let Some(ref path) = result.path {
            info!("Scanning {}...", result.repo.name);
            let (local, hosted) = scanner::scan_directory(path, &result.repo.name);
            
            info!("  Found {} Local NIM, {} Hosted NIM references",
                  local.len(), hosted.len());
            
            all_local.extend(local);
            all_hosted.extend(hosted);
        }
    }
    
    // Categorize results
    info!("Categorizing results...");
    let (mut source_code, mut actions_workflow) = scanner::categorize_results(all_local, all_hosted);
    
    // Deduplicate
    scanner::deduplicate_results(&mut source_code);
    scanner::deduplicate_results(&mut actions_workflow);
    
    info!("Source code: {} Local NIM, {} Hosted NIM",
          source_code.local_nim.len(), source_code.hosted_nim.len());
    info!("Actions workflow: {} Local NIM, {} Hosted NIM",
          actions_workflow.local_nim.len(), actions_workflow.hosted_nim.len());
    
    // Enrich with NGC API
    info!("Enriching findings with NGC API...");
    ngc_api::enrich_all_findings(
        args.ngc_api_key.as_deref(),
        &mut source_code,
        &mut actions_workflow,
    );
    
    // Generate report
    let report = ScanReport::new(repos.len(), source_code, actions_workflow);
    
    // Create output directory
    std::fs::create_dir_all(&args.output)
        .with_context(|| format!("Failed to create output directory: {}", args.output.display()))?;
    
    // Generate JSON report
    let json_path = args.output.join("report.json");
    report::generate_json_report(&report, &json_path)
        .context("Failed to generate JSON report")?;
    
    // Generate CSV reports
    report::generate_csv_reports(&report, &args.output)
        .context("Failed to generate CSV reports")?;

    // Generate aggregate report
    let aggregate_path = args.output.join("report_aggregate.json");
    report::generate_aggregate_report(&report, &aggregate_path)
        .context("Failed to generate aggregate report")?;
    
    // Print summary
    report::print_summary(&report);
    
    // Cleanup
    if !args.keep_repos {
        info!("Cleaning up cloned repositories...");
        if let Some(td) = temp_dir {
            // TempDir will clean up on drop
            drop(td);
        } else if let Some(ref dir) = args.workdir {
            if let Err(e) = git_ops::cleanup_repos(dir) {
                warn!("Failed to cleanup workdir: {}", e);
            }
        }
    } else {
        info!("Keeping cloned repositories in {}", workdir.display());
    }
    
    info!("Scan complete!");
    info!("Reports written to: {}", args.output.display());
    
    Ok(())
}

/// Run the query subcommand
fn run_query(args: QueryArgs) -> Result<()> {
    match args.query_type {
        QueryType::HostedNim(hosted_args) => run_query_hosted_nim(hosted_args),
        QueryType::LocalNim(local_args) => run_query_local_nim(local_args),
    }
}

/// Query Hosted NIM information by model name
fn run_query_hosted_nim(args: HostedNimQueryArgs) -> Result<()> {
    // Initialize logging
    init_logging(args.verbose);
    
    info!("Querying Hosted NIM information for model: {}", args.model);
    
    // Create NGC client
    let mut client = ngc_api::NgcClient::new(args.ngc_api_key)
        .context("Failed to create NGC client")?;
    
    // Query the model
    let result = client.query_hosted_nim(&args.model)?;
    
    // Output as JSON
    let json = serde_json::to_string_pretty(&result)
        .context("Failed to serialize result to JSON")?;
    
    println!("{}", json);
    
    Ok(())
}

/// Query Local NIM information by image name
fn run_query_local_nim(args: LocalNimQueryArgs) -> Result<()> {
    // Initialize logging
    init_logging(args.verbose);
    
    info!("Querying Local NIM information for image: {}", args.image);
    
    // Create NGC client
    let mut client = ngc_api::NgcClient::new(args.ngc_api_key)
        .context("Failed to create NGC client")?;
    
    // Build full image URL for query
    let image_url = if args.image.starts_with("nvcr.io/nim/") {
        args.image.clone()
    } else {
        format!("nvcr.io/nim/{}", args.image)
    };
    
    // Query the image
    let result = client.query_local_nim(&image_url)?;
    
    // Output as JSON
    let json = serde_json::to_string_pretty(&result)
        .context("Failed to serialize result to JSON")?;
    
    println!("{}", json);
    
    Ok(())
}
