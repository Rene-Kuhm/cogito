//! Opt-in precision/recall measurement for reference edges.
//!
//! Needs the two pinned holdout repositories checked out under one root, plus a
//! scratch directory for the index. Nothing is written into the measured
//! repositories except one line inside the sampled declaration, and that line
//! is restored immediately after the impact is built:
//!
//! ```text
//! COGITO_REFS_MEASURE=1 \
//! COGITO_REFS_ROOT=<root with bat/ and ripgrep/> \
//! COGITO_REFS_STATE=<scratch dir for state.db> \
//!   cargo test -p cogito-harness-core --test refs_measure_test -- --ignored --nocapture
//! ```
//!
//! Truth is what a person reading the code would count as a reference: a file
//! that uses the name and imports the module that declares it, by a `use`,
//! `import`, or an inline qualified path naming that module. The labels were
//! made by an agent reading the source, not by a person.

use cogito_harness_core::Cogito;
use cogito_harness_core::impact;
use cogito_harness_core::index;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// One hand-labeled sample: the edited line must fall inside `symbol`, and
/// `truth` lists the files (repository-relative, never the declaring file) that
/// really reference it.
struct Sample {
    repo: &'static str,
    file: &'static str,
    symbol: &'static str,
    /// 1-based line inside the declaration named `symbol`.
    line: usize,
    truth: &'static [&'static str],
}

const SAMPLES: &[Sample] = &[
    // bat @ d9559c6 (Rust, module root `src/`).
    Sample {
        repo: "bat",
        file: "src/lessopen.rs",
        symbol: "LessOpenPreprocessor",
        line: 66,
        truth: &["src/controller.rs"],
    },
    Sample {
        repo: "bat",
        file: "src/diff.rs",
        symbol: "LineChange",
        line: 12,
        truth: &["src/decorations.rs"],
    },
    Sample {
        repo: "bat",
        file: "src/input.rs",
        symbol: "OpenedInput",
        line: 109,
        truth: &[
            "src/assets.rs",
            "src/controller.rs",
            "src/lessopen.rs",
            "src/printer.rs",
        ],
    },
    // A binary target: `crate::` here means `src/bin/bat/`, not `src/`.
    Sample {
        repo: "bat",
        file: "src/bin/bat/config.rs",
        symbol: "system_config_file",
        line: 16,
        truth: &["src/bin/bat/main.rs"],
    },
    // ripgrep @ 3fce3b5 (Rust).
    Sample {
        repo: "ripgrep",
        file: "crates/printer/src/util.rs",
        symbol: "NiceDuration",
        line: 372,
        truth: &["crates/printer/src/stats.rs"],
    },
    Sample {
        repo: "ripgrep",
        file: "crates/printer/src/util.rs",
        symbol: "DecimalFormatter",
        line: 416,
        truth: &[
            "crates/printer/src/hyperlink/mod.rs",
            "crates/printer/src/standard.rs",
        ],
    },
    Sample {
        repo: "ripgrep",
        file: "crates/printer/src/hyperlink/mod.rs",
        symbol: "HyperlinkAlias",
        line: 192,
        truth: &["crates/printer/src/hyperlink/aliases.rs"],
    },
    // `src/lib.rs` re-exports it and `examples/walk.rs` declares its own enum
    // of the same name, so neither is a reference.
    Sample {
        repo: "ripgrep",
        file: "crates/ignore/src/walk.rs",
        symbol: "DirEntry",
        line: 32,
        truth: &[
            "crates/ignore/src/dir.rs",
            "crates/ignore/src/pathutil.rs",
            "crates/core/haystack.rs",
        ],
    },
];

#[test]
#[ignore = "opt-in measurement: needs COGITO_REFS_MEASURE=1 and both holdout repos"]
fn test_reference_edge_precision_and_recall() {
    if std::env::var("COGITO_REFS_MEASURE").is_err() {
        eprintln!("skipped: set COGITO_REFS_MEASURE=1 to run this measurement");
        return;
    }
    let root = PathBuf::from(std::env::var("COGITO_REFS_ROOT").expect("COGITO_REFS_ROOT"));
    let state = PathBuf::from(std::env::var("COGITO_REFS_STATE").expect("COGITO_REFS_STATE"));
    let cogito = Cogito::with_project_sqlite(&state).unwrap();

    // repo -> (true positives, false positives, false negatives)
    let mut totals: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    let mut invalid: Vec<String> = Vec::new();

    println!("| repo | file | symbol | predicted | truth | tp | fp | fn |");
    println!("|---|---|---|---|---|---|---|---|");

    for sample in SAMPLES {
        let repo_root = root.join(sample.repo);
        let file = repo_root.join(sample.file);
        let original = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));

        // One line inside the declaration, so the diff's only hunk is inside
        // the symbol this sample measures.
        let mut lines: Vec<String> = original.lines().map(str::to_string).collect();
        lines[sample.line - 1].push_str(" // refs-measure");
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();

        let project = cogito.init_project(repo_root.clone()).unwrap();
        cogito.register_project(&project).unwrap();
        let result = {
            let conn = cogito.connection().unwrap();
            index::sync(&conn, project.id, &repo_root, false)
                .and_then(|_| impact::build(&conn, project.id, &repo_root, None))
        };

        // Restore before looking at the result: the repository is only read.
        std::fs::write(&file, &original).unwrap();
        let impact =
            result.unwrap_or_else(|e| panic!("sync/build failed for {}: {e:#}", sample.file));

        let touched: BTreeSet<&str> = impact
            .changed
            .iter()
            .flat_map(|change| &change.touched)
            .map(|symbol| symbol.name.as_str())
            .collect();
        if !touched.contains(sample.symbol) {
            invalid.push(format!(
                "{} {}: edited line {} touches {touched:?}, not {}",
                sample.repo, sample.file, sample.line, sample.symbol
            ));
        }

        let predicted: BTreeSet<String> = impact
            .references
            .iter()
            .filter(|reference| reference.file != reference.declared_in)
            .map(|reference| reference.file.to_string_lossy().into_owned())
            .collect();
        let truth: BTreeSet<String> = sample.truth.iter().map(|path| path.to_string()).collect();
        let hits = predicted.intersection(&truth).count();

        let entry = totals.entry(sample.repo).or_default();
        entry.0 += hits;
        entry.1 += predicted.len() - hits;
        entry.2 += truth.len() - hits;

        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} |",
            sample.repo,
            sample.file,
            sample.symbol,
            join(&predicted),
            join(&truth),
            hits,
            predicted.len() - hits,
            truth.len() - hits,
        );
    }

    println!();
    let (mut tp, mut fp, mut fneg) = (0, 0, 0);
    for (repo, (repo_tp, repo_fp, repo_fn)) in &totals {
        tp += repo_tp;
        fp += repo_fp;
        fneg += repo_fn;
        println!(
            "{repo}: tp={repo_tp} fp={repo_fp} fn={repo_fn} precision={} recall={}",
            ratio(*repo_tp, repo_tp + repo_fp),
            ratio(*repo_tp, repo_tp + repo_fn),
        );
    }
    println!(
        "TOTAL: samples={} tp={tp} fp={fp} fn={fneg} precision={} recall={}",
        SAMPLES.len(),
        ratio(tp, tp + fp),
        ratio(tp, tp + fneg),
    );

    assert!(invalid.is_empty(), "invalid samples: {invalid:#?}");
}

fn join(set: &BTreeSet<String>) -> String {
    if set.is_empty() {
        "—".to_string()
    } else {
        set.iter().cloned().collect::<Vec<_>>().join(", ")
    }
}

fn ratio(part: usize, whole: usize) -> String {
    if whole == 0 {
        "n/a".to_string()
    } else {
        format!("{:.3}", part as f64 / whole as f64)
    }
}
