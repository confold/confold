//! Confold CLI (`confold`) — a thin frontend over `confold-core`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use confold_core::{
    compare, CompareConfig, CompareMethod, FilterSet, LocalSource, DEFAULT_LARGE_FILE_THRESHOLD,
};

/// Confold: cross-platform folder compare.
#[derive(Parser)]
#[command(name = "confold", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compare two directory trees.
    Compare(CompareArgs),
}

#[derive(Parser)]
struct CompareArgs {
    /// Left (first) directory.
    left: PathBuf,
    /// Right (second) directory.
    right: PathBuf,

    /// Comparison method.
    #[arg(long, value_enum, default_value_t = MethodArg::Quick)]
    method: MethodArg,

    /// Do not descend into subdirectories.
    #[arg(long)]
    no_recursive: bool,

    /// Glob(s) of paths/names to include (files only). May be repeated.
    #[arg(long)]
    include: Vec<String>,

    /// Glob(s) of paths/names to exclude. May be repeated.
    #[arg(long)]
    exclude: Vec<String>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,

    /// Size (bytes) above which `quick` switches to sampled comparison.
    #[arg(long, default_value_t = DEFAULT_LARGE_FILE_THRESHOLD)]
    quick_threshold: u64,

    /// Exit with code 1 if any difference is found (for scripting).
    #[arg(long)]
    fail_on_diff: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum MethodArg {
    Full,
    Quick,
    Size,
    Mtime,
    SizeMtime,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Text,
    Json,
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();
    match cli.command {
        Command::Compare(args) => run_compare(args),
    }
}

fn run_compare(args: CompareArgs) -> anyhow::Result<ExitCode> {
    let method = match args.method {
        MethodArg::Full => CompareMethod::Full,
        MethodArg::Quick => CompareMethod::Quick {
            large_file_threshold: args.quick_threshold,
        },
        MethodArg::Size => CompareMethod::Size,
        MethodArg::Mtime => CompareMethod::Mtime,
        MethodArg::SizeMtime => CompareMethod::SizeAndMtime,
    };
    let filters =
        FilterSet::new(&args.include, &args.exclude).context("invalid include/exclude glob")?;
    let cfg = CompareConfig {
        method,
        recursive: !args.no_recursive,
        filters,
    };

    let left = LocalSource::new(&args.left);
    let right = LocalSource::new(&args.right);
    let report = compare(&left, &right, &cfg)
        .with_context(|| format!("comparing {:?} and {:?}", args.left, args.right))?;

    match args.format {
        FormatArg::Text => print!("{}", confold_core::render_text(&report)),
        FormatArg::Json => {
            let json = serde_json::to_string_pretty(&report).context("serializing report")?;
            println!("{json}");
        }
    }

    if args.fail_on_diff && report.has_differences() {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}
