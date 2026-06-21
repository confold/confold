//! Human-readable rendering of a [`DiffReport`]. JSON is provided directly via `serde` on the model.

use std::fmt::Write as _;

use crate::model::{DiffEntry, DiffReport, DiffStatus};

/// Single-character status marker used in text output.
fn marker(status: DiffStatus) -> char {
    match status {
        DiffStatus::Identical => '=',
        DiffStatus::Different => '~',
        DiffStatus::LeftOnly => '<',
        DiffStatus::RightOnly => '>',
        DiffStatus::Skipped => '.',
        DiffStatus::Error => '!',
    }
}

/// Render a report as an indented text tree followed by a summary line.
pub fn render_text(report: &DiffReport) -> String {
    let mut out = String::new();
    for child in &report.root.children {
        render_entry(&mut out, child, 0);
    }
    let s = &report.summary;
    let _ = writeln!(
        out,
        "\n{} identical, {} different, {} left-only, {} right-only, {} skipped, {} errored",
        s.identical, s.different, s.left_only, s.right_only, s.skipped, s.errored
    );
    out
}

fn render_entry(out: &mut String, entry: &DiffEntry, depth: usize) {
    let indent = "  ".repeat(depth);
    let slash = if entry.is_dir { "/" } else { "" };
    let detail = match &entry.detail {
        Some(d) => format!("  ({d})"),
        None => String::new(),
    };
    let _ = writeln!(
        out,
        "{indent}{} {}{slash}{detail}",
        marker(entry.status),
        entry.name
    );
    for child in &entry.children {
        render_entry(out, child, depth + 1);
    }
}
