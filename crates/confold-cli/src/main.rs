//! Confold CLI (`confold`) — a thin frontend over `confold-core`.

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use confold_core::{
    compare, CompareConfig, CompareMethod, FilterSet, LocalSource, DEFAULT_LARGE_FILE_THRESHOLD,
};
use confold_semantic::{
    apply as semantic_apply, prepare as semantic_prepare, read_bundle, read_proposal,
    review as semantic_review, write_json_new, MAX_INPUT_BYTES, MAX_PROTOCOL_JSON_BYTES,
    MAX_RESULT_BYTES, SCHEMA_VERSION, SUPPORTED_EXTENSIONS,
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
    /// Report machine-readable CLI and protocol capabilities.
    Capabilities(CapabilitiesArgs),
    /// Compare two directory trees.
    Compare(CompareArgs),
    /// Prepare, review, and apply AI-assisted semantic document proposals.
    Semantic(SemanticArgs),
}

#[derive(Parser)]
struct CapabilitiesArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,
}

#[derive(Parser)]
struct SemanticArgs {
    #[command(subcommand)]
    command: SemanticCommand,
}

#[derive(Subcommand)]
enum SemanticCommand {
    /// Capture bounded immutable text inputs in a semantic bundle.
    Prepare(SemanticPrepareArgs),
    /// Validate a semantic proposal and display its deterministic result diff.
    Review(SemanticReviewArgs),
    /// Revalidate and atomically write a proposal to a new output file.
    Apply(SemanticApplyArgs),
}

#[derive(Parser)]
struct SemanticPrepareArgs {
    /// Left document variant.
    #[arg(long)]
    left: PathBuf,
    /// Right document variant.
    #[arg(long)]
    right: PathBuf,
    /// Optional common base document.
    #[arg(long)]
    base: Option<PathBuf>,
    /// New JSON bundle path. Existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
}

#[derive(Parser)]
struct SemanticReviewArgs {
    /// Bundle produced by `semantic prepare`.
    #[arg(long)]
    bundle: PathBuf,
    /// Agent-authored proposal JSON.
    #[arg(long)]
    proposal: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,
}

#[derive(Parser)]
struct SemanticApplyArgs {
    /// Bundle produced by `semantic prepare`.
    #[arg(long)]
    bundle: PathBuf,
    /// Reviewed agent-authored proposal JSON.
    #[arg(long)]
    proposal: PathBuf,
    /// New merged document path. Existing files are never overwritten.
    #[arg(long)]
    output: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = FormatArg::Text)]
    format: FormatArg,
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
        Command::Capabilities(args) => run_capabilities(args),
        Command::Compare(args) => run_compare(args),
        Command::Semantic(args) => run_semantic(args),
    }
}

fn run_capabilities(args: CapabilitiesArgs) -> anyhow::Result<ExitCode> {
    let value = serde_json::json!({
        "cli_version": env!("CARGO_PKG_VERSION"),
        "semantic_protocol_versions": [SCHEMA_VERSION],
        "semantic_max_input_bytes": MAX_INPUT_BYTES,
        "semantic_max_result_bytes": MAX_RESULT_BYTES,
        "semantic_max_protocol_json_bytes": MAX_PROTOCOL_JSON_BYTES,
        "semantic_extensions": SUPPORTED_EXTENSIONS,
        "commands": [
            "capabilities",
            "compare",
            "semantic prepare",
            "semantic review",
            "semantic apply"
        ]
    });
    match args.format {
        FormatArg::Json => println!("{}", serde_json::to_string_pretty(&value)?),
        FormatArg::Text => {
            println!("Confold CLI {}", env!("CARGO_PKG_VERSION"));
            println!("Semantic protocol: v{SCHEMA_VERSION}");
            println!("Semantic input limit: {MAX_INPUT_BYTES} bytes per file");
            println!("Semantic result limit: {MAX_RESULT_BYTES} bytes");
            println!("Semantic protocol JSON limit: {MAX_PROTOCOL_JSON_BYTES} bytes");
            println!("Semantic extensions: {}", SUPPORTED_EXTENSIONS.join(", "));
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn run_semantic(args: SemanticArgs) -> anyhow::Result<ExitCode> {
    match args.command {
        SemanticCommand::Prepare(args) => {
            let bundle = semantic_prepare(&args.left, &args.right, args.base.as_deref())?;
            write_json_new(&args.output, &bundle)?;
            let value = serde_json::json!({
                "bundle": args.output,
                "operation_id": bundle.operation_id,
                "fast_path": bundle.fast_path,
            });
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        SemanticCommand::Review(args) => {
            let bundle = read_bundle(&args.bundle)?;
            let proposal = read_proposal(&args.proposal)?;
            let report = semantic_review(&bundle, &proposal)?;
            match args.format {
                FormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                FormatArg::Text => {
                    println!("Verdict: {:?}", report.verdict);
                    println!("Applicable: {}", report.applicable);
                    println!("Summary: {}", report.summary);
                    for diff in report.diffs {
                        println!(
                            "\n--- {:?} to result ---\n{}",
                            diff.source, diff.unified_diff
                        );
                    }
                }
            }
        }
        SemanticCommand::Apply(args) => {
            let bundle = read_bundle(&args.bundle)?;
            let proposal = read_proposal(&args.proposal)?;
            let report = semantic_apply(&bundle, &proposal, &args.output)?;
            match args.format {
                FormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                FormatArg::Text => {
                    println!("Wrote {}", report.output.display());
                    println!("SHA-256: {}", report.sha256);
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
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
