//! Tests for `context_pack`.
//!
//! They run against real temporary projects and the real index, with a word-count
//! tokenizer instead of tiktoken. That substitution is the point of injecting the
//! counter: the budget logic is exercised exactly, and the test says nothing
//! about which vocabulary the caller uses.

use chrono::Utc;
use cogito_harness_core::Cogito;
use cogito_harness_core::index;
use cogito_harness_core::pack::{self, Reason};
use cogito_harness_core::types::{Fingerprint, Language, Zone, ZoneFrequency};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

/// A tokenizer a test can predict: one token per whitespace-separated word.
fn words(text: &str) -> usize {
    text.split_whitespace().count()
}

fn project(files: &[(&str, &str)]) -> (TempDir, PathBuf, Cogito, Uuid) {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_path_buf();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"packtest\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (rel, body) in files {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    let cogito = Cogito::with_project_sqlite(&root.join(".cogito")).unwrap();
    let project = cogito.init_project(root.clone()).unwrap();
    cogito.register_project(&project).unwrap();
    (tmp, root, cogito, project.id)
}

/// Sync the index and pack, in one place so every test measures the same path.
///
/// The connection type is never named: the guard coerces to what `pack::build`
/// wants, which keeps `rusqlite` out of this crate's dev-dependencies.
fn build_pack(
    cogito: &Cogito,
    root: &Path,
    id: Uuid,
    task: &str,
    budget: usize,
    fingerprint: Option<&Fingerprint>,
) -> pack::Pack {
    let guard = cogito.sqlite().unwrap().lock();
    index::sync(&guard, id, root, false).unwrap();
    pack::build(
        pack::Request {
            conn: &guard,
            project_id: id,
            root,
            task,
            budget,
            fingerprint,
        },
        &words,
    )
    .unwrap()
}

fn pack_for(cogito: &Cogito, root: &Path, id: Uuid, task: &str, budget: usize) -> pack::Pack {
    build_pack(cogito, root, id, task, budget, None)
}

fn fragment_paths(pack: &pack::Pack) -> Vec<String> {
    pack.fragments
        .iter()
        .map(|f| f.path.display().to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Determinism
// ---------------------------------------------------------------------------

#[test]
fn test_pack_is_deterministic() {
    let (_tmp, root, cogito, id) = project(&[
        (
            "src/session_store.rs",
            "pub fn session_store() -> u32 {\n    1\n}\n",
        ),
        (
            "src/session_util.rs",
            "pub fn helper() -> u32 {\n    2\n}\n",
        ),
        ("docs/session.md", "# Session\n\nHow the session works.\n"),
    ]);

    let first = pack_for(&cogito, &root, id, "how does the session store work", 2000);
    let second = pack_for(&cogito, &root, id, "how does the session store work", 2000);
    let rendered = pack::render(&first);

    assert_eq!(
        rendered,
        pack::render(&second),
        "the same task against the same index must render identically"
    );
    assert!(
        !rendered.is_empty(),
        "the fixture must produce something to compare"
    );
}

// ---------------------------------------------------------------------------
// Budget
// ---------------------------------------------------------------------------

#[test]
fn test_budget_is_never_exceeded_at_any_size() {
    let (_tmp, root, cogito, id) = project(&[
        (
            "src/session_store.rs",
            "pub fn session_store() -> u32 {\n    1\n}\n\npub fn session_load() -> u32 {\n    2\n}\n",
        ),
        (
            "src/session_util.rs",
            "pub fn helper() -> u32 {\n    3\n}\n",
        ),
        ("src/other.rs", "pub fn unrelated() -> u32 {\n    4\n}\n"),
    ]);

    with_synced_budgets(&cogito, &root, id);
}

fn with_synced_budgets(cogito: &Cogito, root: &Path, id: Uuid) {
    let guard = cogito.sqlite().unwrap().lock();
    index::sync(&guard, id, root, false).unwrap();
    for budget in [
        pack::MAX_BUDGET + 500,
        2000,
        500,
        200,
        120,
        60,
        30,
        10,
        5,
        1,
    ] {
        let built = pack::build(
            pack::Request {
                conn: &guard,
                project_id: id,
                root,
                task: "where is the session store",
                budget,
                fingerprint: None,
            },
            &words,
        )
        .unwrap();

        let rendered = pack::render(&built);
        let spent = words(&rendered);
        assert!(
            spent <= built.budget,
            "budget {} produced {} tokens",
            built.budget,
            spent
        );
        assert_eq!(
            spent, built.used,
            "the reported figure must be the text's real cost at budget {}",
            built.budget
        );
        if built.too_small {
            assert!(
                rendered.is_empty(),
                "a budget too small to answer honestly must send nothing, not {} tokens",
                spent
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Ranges
// ---------------------------------------------------------------------------

#[test]
fn test_fragment_ranges_match_the_file() {
    let (_tmp, root, cogito, id) = project(&[
        (
            "src/session_store.rs",
            "pub fn session_store() -> u32 {\n    1 + 1\n}\n\npub struct SessionConfig {\n    pub retries: u32,\n}\n",
        ),
        (
            "docs/session.md",
            "# Session\n\nIntro.\n\n## Loading\n\nBody.\n",
        ),
    ]);

    let built = pack_for(&cogito, &root, id, "session store config", 4000);
    assert!(
        !built.fragments.is_empty(),
        "the fixture must produce fragments"
    );

    for f in &built.fragments {
        assert!(f.start >= 1, "start is 1-based, got {}", f.start);
        assert!(
            f.start <= f.end,
            "{} has start {} > end {}",
            f.location(),
            f.start,
            f.end
        );
        let text = fs::read_to_string(root.join(&f.path)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            f.end <= lines.len(),
            "{} claims line {} but the file has {}",
            f.location(),
            f.end,
            lines.len()
        );
        assert_eq!(
            f.text,
            lines[f.start - 1..f.end].join("\n"),
            "{} text must be exactly its range",
            f.location()
        );
        assert!(
            f.text.trim_end().contains('\n') || f.start == f.end || !f.text.is_empty(),
            "{} has no text",
            f.location()
        );
    }
}

// ---------------------------------------------------------------------------
// Nothing to say
// ---------------------------------------------------------------------------

#[test]
fn test_no_match_returns_nothing_and_says_so() {
    let (_tmp, root, cogito, id) = project(&[(
        "src/session_store.rs",
        "pub fn session_store() -> u32 {\n    1\n}\n",
    )]);

    let built = pack_for(&cogito, &root, id, "kubernetes ingress annotations", 2000);
    assert_eq!(built.candidates, 0, "no candidate matched");
    assert!(
        built.fragments.is_empty(),
        "a pack with no candidate must return no fragment, got {:?}",
        fragment_paths(&built)
    );
    let rendered = pack::render(&built);
    assert!(
        rendered.contains("No matches for this task"),
        "the answer must say there is nothing: {rendered}"
    );
}

#[test]
fn test_a_task_of_only_stopwords_returns_nothing() {
    let (_tmp, root, cogito, id) = project(&[(
        "src/session_store.rs",
        "pub fn session_store() -> u32 {\n    1\n}\n",
    )]);

    let built = pack_for(&cogito, &root, id, "how does the and for with it", 2000);
    assert_eq!(built.candidates, 0);
    assert!(built.fragments.is_empty());
}

// ---------------------------------------------------------------------------
// No git
// ---------------------------------------------------------------------------

#[test]
fn test_pack_works_in_a_project_without_git() {
    let (_tmp, root, cogito, id) = project(&[(
        "src/session_store.rs",
        "pub fn session_store() -> u32 {\n    1\n}\n",
    )]);

    assert!(
        !root.join(".git").exists(),
        "the fixture must not be a git repository"
    );
    let built = pack_for(&cogito, &root, id, "session store", 2000);
    assert!(
        !built.fragments.is_empty(),
        "a project without git must still pack: no zones, but a usable index"
    );
}

// ---------------------------------------------------------------------------
// Ranking
// ---------------------------------------------------------------------------

#[test]
fn test_name_match_outranks_path_match_outranks_neighbour() {
    let (_tmp, root, cogito, id) = project(&[
        (
            "src/session_store.rs",
            "use crate::deep::thing;\n\npub fn session_store() -> u32 {\n    thing()\n}\n",
        ),
        // Matched by path only: the file name carries the term, not a declaration.
        (
            "src/session_util.rs",
            "pub fn helper() -> u32 {\n    2\n}\n",
        ),
        // Only reachable through the import above.
        ("src/deep.rs", "pub fn thing() -> u32 {\n    7\n}\n"),
    ]);

    let built = pack_for(&cogito, &root, id, "session", 2000);
    let paths = fragment_paths(&built);

    let position = |needle: &str| {
        paths
            .iter()
            .position(|p| p.contains(needle))
            .unwrap_or_else(|| panic!("{needle} missing from {paths:?}"))
    };

    assert!(
        position("session_store.rs") < position("session_util.rs"),
        "a name match must outrank a path match: {paths:?}"
    );
    assert!(
        position("session_util.rs") < position("deep.rs"),
        "a path match must outrank a neighbour: {paths:?}"
    );

    let neighbour = built
        .fragments
        .iter()
        .find(|f| f.path.to_string_lossy().contains("deep.rs"))
        .unwrap();
    assert_eq!(neighbour.reason, Reason::Neighbour);
    assert!(
        neighbour.detail.starts_with("imported by"),
        "the reason must name who pulled it in, got {:?}",
        neighbour.detail
    );
}

#[test]
fn test_source_outranks_test_for_the_same_match() {
    let (_tmp, root, cogito, id) = project(&[
        (
            "src/session_core.rs",
            "pub fn session_core() -> u32 {\n    1\n}\n",
        ),
        (
            "tests/session_core_test.rs",
            "pub fn session_core() -> u32 {\n    2\n}\n",
        ),
    ]);

    let built = pack_for(&cogito, &root, id, "session_core", 2000);
    let paths = fragment_paths(&built);
    let source = paths.iter().position(|p| p == "src/session_core.rs");
    let test = paths.iter().position(|p| p == "tests/session_core_test.rs");

    assert!(
        source.is_some() && test.is_some(),
        "both must appear: {paths:?}"
    );
    assert!(
        source.unwrap() < test.unwrap(),
        "for the same match the source must come first: {paths:?}"
    );
}

#[test]
fn test_more_changes_break_a_tie_between_files() {
    let (_tmp, root, cogito, id) = project(&[
        ("src/aa_store.rs", "pub fn helper() -> u32 {\n    1\n}\n"),
        ("src/zz_store.rs", "pub fn helper() -> u32 {\n    2\n}\n"),
    ]);

    // Alphabetically `aa_store.rs` wins. The fingerprint says the other file is
    // where the work happens, which must win instead.
    let zone = |path: &str, count: usize| Zone {
        path: PathBuf::from(path),
        frequency: ZoneFrequency::Hot,
        last_modified: Utc::now(),
        change_count: count,
    };
    let fingerprint = Fingerprint {
        project_id: id,
        entry_points: Vec::new(),
        data_flows: Vec::new(),
        hot_zones: vec![zone("src/zz_store.rs", 9)],
        cold_zones: vec![zone("src/aa_store.rs", 1)],
        frozen_zones: Vec::new(),
        language: Language::Rust,
        generated_at: Utc::now(),
        files: Vec::new(),
        modules: Vec::new(),
        head: None,
        docs: Vec::new(),
        symbols: Vec::new(),
        format: 0,
    };

    let built = build_pack(&cogito, &root, id, "store", 2000, Some(&fingerprint));

    let paths = fragment_paths(&built);
    let hot = paths.iter().position(|p| p.contains("zz_store.rs"));
    let cold = paths.iter().position(|p| p.contains("aa_store.rs"));
    assert!(
        hot.is_some() && cold.is_some(),
        "both must appear: {paths:?}"
    );
    assert!(
        hot.unwrap() < cold.unwrap(),
        "the file with more changes must come first: {paths:?}"
    );
}

// ---------------------------------------------------------------------------
// The cut
// ---------------------------------------------------------------------------

#[test]
fn test_a_fragment_that_does_not_fit_is_skipped_not_fatal() {
    let mut giant = String::from("pub fn giant_session() -> u32 {\n");
    for _ in 0..400 {
        giant.push_str("    let x = 1;\n");
    }
    giant.push_str("    1\n}\n");

    let (_tmp, root, cogito, id) = project(&[
        ("src/giant_session.rs", giant.as_str()),
        ("src/tiny_session.rs", "pub fn tiny() -> u32 {\n    2\n}\n"),
    ]);

    let built = pack_for(&cogito, &root, id, "session", 120);
    let paths = fragment_paths(&built);

    assert!(
        !paths.iter().any(|p| p.contains("giant_session")),
        "the oversized fragment must not be included: {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("tiny_session")),
        "a smaller fragment after an oversized one must still fit: {paths:?}"
    );
    assert!(
        built.skipped >= 1,
        "the skipped candidate must be counted, got {}",
        built.skipped
    );
    assert!(built.used <= built.budget);
}

#[test]
fn test_short_file_without_declarations_is_included_whole() {
    let (_tmp, root, cogito, id) = project(&[(
        "notes/session_notes.md",
        "Plain prose about the session.\nNo headings at all.\nThird line.\n",
    )]);

    let built = pack_for(&cogito, &root, id, "session_notes", 2000);
    let whole = built
        .fragments
        .iter()
        .find(|f| f.path.to_string_lossy().contains("session_notes.md"))
        .expect("a short file with no declarations must still be usable");
    assert_eq!(whole.kind, "file");
    assert_eq!(whole.start, 1);
    assert_eq!(whole.end, 3);
}

// ---------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------

#[test]
fn test_task_terms_drop_stopwords_and_short_words_in_both_languages() {
    let english = pack::task_terms("how does the session store work in a file");
    assert!(english.contains(&"session".to_string()), "{english:?}");
    assert!(english.contains(&"store".to_string()), "{english:?}");
    // Content words stay: "work" and "file" are weak, but they are not
    // function words, and over-filtering costs recall. A term that matches
    // nothing only costs one search.
    assert!(english.contains(&"work".to_string()), "{english:?}");
    assert!(english.contains(&"file".to_string()), "{english:?}");
    for stop in ["how", "does", "the", "in", "a"] {
        assert!(
            !english.contains(&stop.to_string()),
            "{stop:?} must be dropped: {english:?}"
        );
    }

    let spanish = pack::task_terms("¿Cómo se mapea el nombre de un archivo a una sintaxis?");
    assert!(spanish.contains(&"mapea".to_string()), "{spanish:?}");
    assert!(spanish.contains(&"sintaxis".to_string()), "{spanish:?}");
    assert!(
        !spanish.contains(&"como".to_string()) && !spanish.contains(&"cómo".to_string()),
        "the question word must be dropped: {spanish:?}"
    );
    let mut sorted = spanish.clone();
    sorted.sort();
    assert_eq!(spanish, sorted, "terms must be sorted for determinism");
}
