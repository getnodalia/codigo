// SPDX-License-Identifier: MIT OR Apache-2.0
//! `cargo run -p codigo-quality --example scan -- <folder>`: print what the analysis finds.
#![allow(clippy::print_stdout)]

use std::sync::atomic::AtomicBool;

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| ".".to_owned());
    let started = std::time::Instant::now();
    let report =
        codigo_quality::analyze_project(std::path::Path::new(&root), &AtomicBool::new(false));
    for f in &report.findings {
        println!(
            "{:?} {} {}:{} {}",
            f.finding.severity,
            f.finding.rule.id,
            f.path.display(),
            f.finding.line + 1,
            f.finding.detail.as_deref().unwrap_or("")
        );
    }
    println!(
        "{} files, {} lines, {} findings, gate {:?}, {:?}",
        report.files,
        report.lines,
        report.findings.len(),
        report.gate(),
        started.elapsed()
    );
}
