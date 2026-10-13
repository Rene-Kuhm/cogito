//! Benchmark: measure token savings when using Cogito vs. baseline exploration.
//!
//! Two strategies for answering "find this term and read the code around it":
//!
//! **Baseline** — an agent without Cogito:
//!   1. List every project file path
//!   2. Run a recursive text search for the term
//!   3. Read each matched file in full
//!
//! **Cogito**:
//!   1. Print the project card (`cogito map --card`)
//!   2. Call `search_code` with the term
//!   3. Call `file_outline` on each matched file
//!   4. Read only the line ranges that the match actually occupies

use anyhow::{Context, Result};
use cogito_harness_core::{Cogito, Fingerprint, Project, index::SearchHit, map};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tiktoken_rs::{CoreBPE, cl100k_base};

/// Token counter using tiktoken-rs with cl100k_base vocabulary.
/// Falls back to byte-count / 4 estimate if the tokenizer fails to initialise.
pub struct TokenCounter {
    inner: Option<CoreBPE>,
}

impl TokenCounter {
    /// Create a new token counter. Logs a warning and falls back to byte/4
    /// estimation if tiktoken-rs fails to load.
    pub fn new() -> Self {
        let inner = cl100k_base().ok();
        if inner.is_none() {
            eprintln!(
                "warning: tiktoken-rs failed to initialise cl100k_base; \
                 falling back to byte_count/4 estimate"
            );
        }
        Self { inner }
    }

    /// Count tokens in `text`. Returns (tokens, raw_bytes) for transparency.
    pub fn count(&self, text: &str) -> (usize, usize) {
        let bytes = text.len();
        match &self.inner {
            Some(encoder) => {
                let tokens = encoder.encode_ordinary(text).len();
                (tokens, bytes)
            }
            None => (bytes / 4, bytes),
        }
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new()
    }
}

/// One search term's benchmark result.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TermResult {
    pub term: String,
    /// True when no files matched the term.
    pub no_matches: bool,
    /// Baseline finding cost: list-all-files + grep output.
    pub baseline_finding_tokens: usize,
    pub baseline_finding_bytes: usize,
    /// Baseline reading cost: reading every matched file in full.
    pub baseline_reading_tokens: usize,
    pub baseline_reading_bytes: usize,
    /// Baseline total tokens and bytes.
    pub baseline_total_tokens: usize,
    pub baseline_total_bytes: usize,
    /// Cogito finding cost: card + search_code output.
    pub cogito_finding_tokens: usize,
    pub cogito_finding_bytes: usize,
    /// Cogito reading cost: file_outline outputs + only the matched line ranges.
    pub cogito_reading_tokens: usize,
    pub cogito_reading_bytes: usize,
    /// Cogito total tokens and bytes.
    pub cogito_total_tokens: usize,
    pub cogito_total_bytes: usize,
    /// How many tokens the Cogito path saved vs. baseline.
    pub savings_tokens: usize,
    /// Savings as a percentage of baseline.
    pub savings_pct: f64,
    /// How many matched files were considered (capped by --limit).
    pub matched_files_count: usize,
}

/// One repository's benchmark result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoResult {
    pub repo: String,
    pub commit: String,
    pub terms: Vec<TermResult>,
    /// Seconds spent on this repo (excluded from deterministic output).
    pub elapsed_seconds: f64,
}

/// Full benchmark run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkResult {
    /// Only present when running against the pinned manifest.
    pub manifest_version: Option<String>,
    pub repos: Vec<RepoResult>,
    /// Elapsed wall-clock time (excluded from deterministic output).
    pub total_elapsed_seconds: f64,
    /// Tokeniser that was actually used.
    pub tokenizer: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Output rendering
// ─────────────────────────────────────────────────────────────────────────────

impl TermResult {
    /// Render a human-readable table row for this term.
    fn to_row(&self) -> String {
        if self.no_matches {
            return format!("{:<20}  (no matches)", self.term);
        }
        format!(
            "{:<20} │ {:>9} │ {:>9} │ {:>8} │ {:>8} │ {:>8} │ {:>6.1}",
            format!("{} ({})", self.term, self.matched_files_count),
            self.baseline_finding_tokens,
            self.baseline_reading_tokens,
            self.cogito_finding_tokens,
            self.cogito_reading_tokens,
            self.savings_tokens,
            self.savings_pct,
        )
    }
}

impl RepoResult {
    /// Render a human-readable table for this repo.
    fn render_table(&self) -> String {
        let mut lines = Vec::with_capacity(self.terms.len() + 6);
        lines.push("─".repeat(75));
        lines.push(format!(
            "Repository: {}  (commit {})",
            self.repo,
            &self.commit[..8.min(self.commit.len())]
        ));
        lines.push(format!(
            "{:<20} │ {:>9} │ {:>9} │ {:>8} │ {:>8} │ {:>8} │ {:>6}",
            "term", "bl_find", "bl_read", "co_find", "co_read", "savings", "pct%"
        ));
        lines.push(format!(
            "{:-<20}-+-{:-<9}-+-{:-<9}-+-{:-<8}-+-{:-<8}-+-{:-<8}-+-{:-<6}",
            "", "", "", "", "", "", ""
        ));
        for term in &self.terms {
            lines.push(term.to_row());
        }
        // Totals
        let bl_find: usize = self.terms.iter().map(|t| t.baseline_finding_tokens).sum();
        let bl_read: usize = self.terms.iter().map(|t| t.baseline_reading_tokens).sum();
        let co_find: usize = self.terms.iter().map(|t| t.cogito_finding_tokens).sum();
        let co_read: usize = self.terms.iter().map(|t| t.cogito_reading_tokens).sum();
        let total_savings = (bl_find + bl_read).saturating_sub(co_find + co_read);
        let total_bl = bl_find + bl_read;
        let pct = if total_bl > 0 {
            (total_savings as f64 / total_bl as f64) * 100.0
        } else {
            0.0
        };
        lines.push(format!(
            "{:-<20}-+-{:-<9}-+-{:-<9}-+-{:-<8}-+-{:-<8}-+-{:-<8}-+-{:-<6}",
            "", "", "", "", "", "", ""
        ));
        lines.push(format!(
            "{:>20} │ {:>9} │ {:>9} │ {:>8} │ {:>8} │ {:>8} │ {:>6.1}",
            "TOTAL", bl_find, bl_read, co_find, co_read, total_savings, pct,
        ));
        lines.push(format!("[Wall-clock: {:.2}s]", self.elapsed_seconds));
        lines.join("\n")
    }
}

impl BenchmarkResult {
    /// Render a human-readable summary table for all repos.
    pub fn render_human(&self) -> String {
        self.repos
            .iter()
            .map(|r| r.render_table())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Render the deterministic part of the result as canonical JSON
    /// (no timing fields, sorted keys, pretty-printed).
    pub fn render_json(&self) -> Result<String> {
        #[derive(Serialize)]
        struct DeterministicRepo<'a> {
            repo: &'a str,
            commit: &'a str,
            terms: &'a [TermResult],
        }
        #[derive(Serialize)]
        struct Deterministic<'a> {
            manifest_version: Option<&'a str>,
            repos: Vec<DeterministicRepo<'a>>,
            tokenizer: &'a str,
        }
        let det = Deterministic {
            manifest_version: self.manifest_version.as_deref(),
            repos: self
                .repos
                .iter()
                .map(|r| DeterministicRepo {
                    repo: &r.repo,
                    commit: &r.commit,
                    terms: &r.terms,
                })
                .collect(),
            tokenizer: &self.tokenizer,
        };
        let json = serde_json::to_string_pretty(&det).context("JSON serialisation failed")?;
        Ok(json)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Pinned manifest
// ─────────────────────────────────────────────────────────────────────────────

/// Entry in the pinned benchmark manifest.
#[derive(Debug, Clone, Deserialize)]
pub struct ManifestEntry {
    pub url: String,
    pub commit: String,
    pub name: String,
    pub terms: Vec<String>,
}

/// The benchmark manifest.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct Manifest {
    pub manifest_version: String,
    pub repos: Vec<ManifestEntry>,
}

impl Manifest {
    /// Load from `benchmarks/repos.json` relative to the crate root.
    pub fn load() -> Result<Self> {
        let path = Self::manifest_path()?;
        let content = fs::read_to_string(&path)
            .context(format!("Failed to read manifest at {}", path.display()))?;
        serde_json::from_str(&content).context("Manifest is not valid JSON")
    }

    fn manifest_path() -> Result<PathBuf> {
        // Cargo.toml is two levels up from src/commands/
        let manifest_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/repos.json");
        let canonical = manifest_path.canonicalize().unwrap_or(manifest_path);
        Ok(canonical)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Repository cloning
// ─────────────────────────────────────────────────────────────────────────────

/// Clone (or update) `url` at `commit` into `cache_dir`.
/// Returns the local path on success.
fn ensure_clone(url: &str, commit: &str, cache_dir: &Path) -> Result<PathBuf> {
    let repo_name = url
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git");
    let target = cache_dir.join(repo_name);

    if target.exists() {
        // Verify it's at the right commit
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&target)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned().into());

        if head.as_deref() == Some(commit) {
            return Ok(target);
        }
        // Wrong commit — remove and re-clone
        fs::remove_dir_all(&target).context("Failed to remove stale clone")?;
    }

    // Clone shallowly at the specific commit
    let output = Command::new("git")
        .args([
            "clone",
            "--no-checkout",
            "--filter=blob:none",
            url,
            target.to_str().unwrap(),
        ])
        .output()
        .context("git clone failed")?;

    if !output.status.success() {
        anyhow::bail!(
            "git clone of {} failed: {}",
            url,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // Checkout the pinned commit
    let checkout = Command::new("git")
        .args(["checkout", commit])
        .current_dir(&target)
        .output()
        .context("git checkout failed")?;

    if !checkout.status.success() {
        let stderr = String::from_utf8_lossy(&checkout.stderr);
        anyhow::bail!("git checkout {} in {} failed: {}", commit, url, stderr);
    }

    Ok(target)
}

// ─────────────────────────────────────────────────────────────────────────────
// Baseline: listing + grep + full file reads
// ─────────────────────────────────────────────────────────────────────────────

/// Return all file paths under `root`, relative to it, sorted.
fn list_all_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    walkdir(root, &mut files);
    files.sort();
    files
}

fn walkdir(root: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|n| n.to_str())
            && (name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == ".git")
        {
            continue;
        }
        if path.is_dir() {
            walkdir(&path, out);
        } else {
            out.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
        }
    }
}

/// Run `grep -rn <term>` in `root`. Returns stdout lines (file:line:content).
fn grep_sync(root: &Path, term: &str) -> Vec<String> {
    let root_str = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let output = std::process::Command::new("grep")
        .args([
            "-rn",
            "--line-number",
            "--exclude-dir=target",
            "--exclude-dir=.git",
            term,
            ".",
        ])
        .current_dir(&root_str)
        .output();
    match output {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.to_owned())
            .collect(),
        _ => Vec::new(),
    }
}

/// Run `grep -rn <term>` in `root`. Returns stdout lines (file:line:content).
async fn grep(root: &Path, term: &str) -> Vec<String> {
    // Use std::thread directly instead of spawn_blocking
    let root_str = root.to_path_buf();
    let term_owned = term.to_owned();
    let handle = std::thread::spawn(move || grep_sync(&root_str, &term_owned));
    handle.join().unwrap_or_default()
}

/// Parse grep output to extract (file, line) pairs.
fn parse_grep_lines(grep_output: &[String]) -> BTreeMap<PathBuf, BTreeSet<usize>> {
    let mut by_file: BTreeMap<PathBuf, BTreeSet<usize>> = BTreeMap::new();
    for line in grep_output {
        if let Some((path_str, rest)) = line.split_once(':')
            && let Some((line_str, _)) = rest.split_once(':')
            && let Ok(line_num) = line_str.parse::<usize>()
        {
            let path = PathBuf::from(path_str);
            by_file.entry(path).or_default().insert(line_num);
        }
    }
    by_file
}

// ─────────────────────────────────────────────────────────────────────────────
// Cogito path: card + search_code + file_outline + selective reads
// ─────────────────────────────────────────────────────────────────────────────

/// Open a project and ensure it has a fingerprint.
/// Returns (cogito, project, fingerprint).
async fn open_project(path: &Path) -> Result<(Cogito, Project, Fingerprint)> {
    let cwd = std::env::current_dir().context("Cannot read cwd")?;
    std::env::set_current_dir(path).context("Cannot chdir to repo")?;

    let cogito = Cogito::new();
    let project = cogito.init_project(path.to_path_buf())?;
    let (fingerprint, _) = cogito.fingerprint(&project, false).await?;

    let _ = std::env::set_current_dir(&cwd); // Restore, best effort

    Ok((cogito, project, fingerprint))
}

/// Get the project card text (what `cogito map --card` returns).
fn project_card_text(map: &map::ProjectMap) -> String {
    map::render_card(map)
}

/// Get the search_code text for `query`.
#[allow(dead_code)]
fn search_code_text(_root: &Path, fp: &Fingerprint, query: &str) -> (String, Vec<SearchHit>) {
    // Replicate the search logic from mcp.rs
    let query_lower = query.to_lowercase();
    let mut hits = Vec::new();

    // Walk files and look for matches in the fingerprint
    let all_files: Vec<_> = fp.files.iter().chain(fp.docs.iter()).collect();

    for file in all_files {
        let path_str = file.to_string_lossy().to_lowercase();
        if path_str.contains(&query_lower) {
            hits.push(SearchHit {
                path: file.clone(),
                kind: "file".to_string(),
                name: file
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string(),
                line: 1,
                is_symbol: false,
                score: 1.0,
                is_test: false,
            });
        }
    }

    let modules: Vec<_> = fp.modules.clone();
    let text = map::render_search(&hits, &modules, query, false);

    (text, hits)
}

/// Get the file_outline text for `file`.
fn file_outline_text(root: &Path, fp: Option<&Fingerprint>, file: &Path) -> String {
    let content = match std::fs::read_to_string(root.join(file)) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let decls = map::outline::declarations(&content, ext);
    map::render_outline(root, fp, file, &decls, 0)
}

/// Read only the matched line ranges from `file`, returning the text.
fn read_matched_ranges(root: &Path, file: &Path, matched_lines: &BTreeSet<usize>) -> String {
    let content = match std::fs::read_to_string(root.join(file)) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    if matched_lines.is_empty() {
        return String::new();
    }

    // Expand each matched line to a range: the line itself plus a small context window.
    const CONTEXT_LINES: usize = 5;
    let all_lines: Vec<_> = content.lines().collect();
    let mut needed_ranges: Vec<(usize, usize)> = Vec::new();

    for &line in matched_lines {
        let start = line.saturating_sub(CONTEXT_LINES + 1).max(1);
        let end = (line + CONTEXT_LINES).min(all_lines.len());
        needed_ranges.push((start, end));
    }

    // Merge overlapping ranges
    needed_ranges.sort_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for range in needed_ranges {
        if let Some(last) = merged.last_mut()
            && range.0 <= last.1 + 1
        {
            last.1 = last.1.max(range.1);
            continue;
        }
        merged.push(range);
    }

    let mut out = String::new();
    for (start, end) in merged {
        for line in all_lines.iter().take(end.min(all_lines.len())).skip(start) {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// One term, both paths
// ─────────────────────────────────────────────────────────────────────────────

/// Benchmark one search term against a repository. Both paths are evaluated
/// over the same set of matched files, derived independently from Cogito (from grep).
/// This is an async function so it can be called from an async context.
async fn benchmark_term_async(
    repo_path: &Path,
    term: &str,
    limit: usize,
    counter: &TokenCounter,
) -> TermResult {
    // ── Baseline finding ─────────────────────────────────────────────────────
    let all_files_text = list_all_files(repo_path)
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    let all_files_count = counter.count(&all_files_text);

    let grep_output = grep(repo_path, term).await;
    let grep_text = grep_output.join("\n");
    let grep_count = counter.count(&grep_text);

    let baseline_finding_tokens = all_files_count.0 + grep_count.0;
    let baseline_finding_bytes = all_files_count.1 + grep_count.1;

    // ── Cogito finding ───────────────────────────────────────────────────────
    // Scan once per repo, but measure only the card + search_code output.
    let scan_result = open_project(repo_path).await;

    let (_card_text, _search_text, cogito_finding_tokens, cogito_finding_bytes) = match &scan_result
    {
        Ok((_, project, fingerprint)) => {
            let map = map::build(&project.path, &project.name, fingerprint);
            let card_text = project_card_text(&map);
            let card_count = counter.count(&card_text);

            let (search_text, _hits) = search_code_text(&project.path, fingerprint, term);
            let search_count = counter.count(&search_text);

            (
                card_text,
                search_text,
                card_count.0 + search_count.0,
                card_count.1 + search_count.1,
            )
        }
        Err(_) => (
            String::new(),
            String::new(),
            baseline_finding_tokens,
            baseline_finding_bytes,
        ),
    };

    // ── Determine the matched file set ───────────────────────────────────────
    // Use the baseline grep to determine which files match, so the comparison
    // isolates the reading strategy.
    let matched_by_file = parse_grep_lines(&grep_output);
    let matched_files: Vec<_> = matched_by_file.into_iter().take(limit).collect();

    if matched_files.is_empty() {
        return TermResult {
            term: term.to_string(),
            no_matches: true,
            baseline_finding_tokens,
            baseline_finding_bytes,
            baseline_reading_tokens: 0,
            baseline_reading_bytes: 0,
            baseline_total_tokens: baseline_finding_tokens,
            baseline_total_bytes: baseline_finding_bytes,
            cogito_finding_tokens,
            cogito_finding_bytes,
            cogito_reading_tokens: 0,
            cogito_reading_bytes: 0,
            cogito_total_tokens: cogito_finding_tokens,
            cogito_total_bytes: cogito_finding_bytes,
            savings_tokens: baseline_finding_tokens.saturating_sub(cogito_finding_tokens),
            savings_pct: if baseline_finding_tokens > 0 {
                let diff = baseline_finding_tokens as f64 - cogito_finding_tokens as f64;
                (diff.max(0.0) / baseline_finding_tokens as f64) * 100.0
            } else {
                0.0
            },
            matched_files_count: 0,
        };
    }

    let matched_files_count = matched_files.len();

    // Get fingerprint for outline rendering
    let fingerprint = scan_result.ok().map(|(_, _, fp)| fp);
    let root_for_reading = repo_path.to_path_buf();

    // ── Baseline reading ─────────────────────────────────────────────────────
    let mut baseline_reading_tokens = 0usize;
    let mut baseline_reading_bytes = 0usize;
    for (file, _lines) in &matched_files {
        let content = std::fs::read_to_string(root_for_reading.join(file)).unwrap_or_default();
        let (t, b) = counter.count(&content);
        baseline_reading_tokens += t;
        baseline_reading_bytes += b;
    }

    // ── Cogito reading ──────────────────────────────────────────────────────
    let mut cogito_reading_tokens = 0usize;
    let mut cogito_reading_bytes = 0usize;
    for (file, matched_lines) in &matched_files {
        let outline_text = file_outline_text(&root_for_reading, fingerprint.as_ref(), file);
        let (t, b) = counter.count(&outline_text);
        cogito_reading_tokens += t;
        cogito_reading_bytes += b;

        let range_text = read_matched_ranges(&root_for_reading, file, matched_lines);
        let (t, b) = counter.count(&range_text);
        cogito_reading_tokens += t;
        cogito_reading_bytes += b;
    }

    let baseline_total_tokens = baseline_finding_tokens + baseline_reading_tokens;
    let baseline_total_bytes = baseline_finding_bytes + baseline_reading_bytes;
    let cogito_total_tokens = cogito_finding_tokens + cogito_reading_tokens;
    let cogito_total_bytes = cogito_finding_bytes + cogito_reading_bytes;

    let savings_tokens = baseline_total_tokens.saturating_sub(cogito_total_tokens);
    let savings_pct = if baseline_total_tokens > 0 {
        (savings_tokens as f64 / baseline_total_tokens as f64) * 100.0
    } else {
        0.0
    };

    TermResult {
        term: term.to_string(),
        no_matches: false,
        baseline_finding_tokens,
        baseline_finding_bytes,
        baseline_reading_tokens,
        baseline_reading_bytes,
        baseline_total_tokens,
        baseline_total_bytes,
        cogito_finding_tokens,
        cogito_finding_bytes,
        cogito_reading_tokens,
        cogito_reading_bytes,
        cogito_total_tokens,
        cogito_total_bytes,
        savings_tokens,
        savings_pct,
        matched_files_count,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Top-level benchmark logic
// ─────────────────────────────────────────────────────────────────────────────

/// Benchmark options shared by the CLI entry point and the test helpers.
#[derive(Debug, Clone)]
pub struct BenchOptions {
    pub repo: Option<PathBuf>,
    pub terms: Vec<String>,
    pub limit: usize,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            repo: None,
            terms: Vec::new(),
            limit: 5,
        }
    }
}

/// Run the benchmark with the given options (async to support tokio runtime).
pub async fn run_benchmark_async(opts: BenchOptions) -> Result<BenchmarkResult> {
    let counter = TokenCounter::new();
    let tokenizer = if counter.inner.is_some() {
        "cl100k_base".to_string()
    } else {
        "byte_count/4_estimate".to_string()
    };

    let start = std::time::Instant::now();
    let mut repos = Vec::new();

    if let Some(repo_path) = &opts.repo {
        // Single local repository
        let terms = if opts.terms.is_empty() {
            // Default terms for a local repo when none specified
            vec!["search_code".to_string(), "project_map".to_string()]
        } else {
            opts.terms.clone()
        };
        let result = benchmark_repo_async(
            repo_path,
            &repo_path.display().to_string(),
            &terms,
            opts.limit,
            &counter,
        )
        .await?;
        repos.push(result);
    } else {
        // Run against the pinned manifest
        let manifest = Manifest::load().ok();

        let Some(manifest) = manifest else {
            anyhow::bail!("No --repo specified and benchmarks/repos.json not found");
        };

        let cache_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.cogito/bench-cache");
        fs::create_dir_all(&cache_dir).context("Failed to create bench cache directory")?;

        for entry in &manifest.repos {
            print!("Cloning {}... ", entry.name);
            std::io::stdout().flush().ok();
            match ensure_clone(&entry.url, &entry.commit, &cache_dir) {
                Ok(path) => {
                    println!("✓");
                    let result = benchmark_repo_async(
                        &path,
                        &entry.name,
                        &entry.terms,
                        opts.limit,
                        &counter,
                    )
                    .await?;
                    repos.push(result);
                }
                Err(e) => {
                    println!("✗ ({})", e);
                    // Continue with other repos per spec
                }
            }
        }
    }

    let total_elapsed = start.elapsed().as_secs_f64();

    Ok(BenchmarkResult {
        manifest_version: None,
        repos,
        total_elapsed_seconds: total_elapsed,
        tokenizer,
    })
}

/// Benchmark one repository (async to support tokio runtime).
async fn benchmark_repo_async(
    repo_path: &Path,
    repo_name: &str,
    terms: &[String],
    limit: usize,
    counter: &TokenCounter,
) -> Result<RepoResult> {
    let start = std::time::Instant::now();

    let commit = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_path)
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                Some(String::from_utf8_lossy(&o.stdout).trim().to_owned())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "unknown".to_string());

    let mut term_results = Vec::with_capacity(terms.len());
    for term in terms {
        let result = benchmark_term_async(repo_path, term, limit, counter).await;
        term_results.push(result);
    }

    let elapsed = start.elapsed().as_secs_f64();

    Ok(RepoResult {
        repo: repo_name.to_string(),
        commit,
        terms: term_results,
        elapsed_seconds: elapsed,
    })
}

/// Print the benchmark results to stdout.
pub fn print_results(result: &BenchmarkResult, json: bool) -> Result<()> {
    if json {
        println!("{}", result.render_json()?);
    } else {
        println!("{}", result.render_human());
    }
    Ok(())
}
