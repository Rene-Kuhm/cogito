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

use crate::tokenizer::TokenCounter;
use anyhow::{Context, Result};
use chrono::Utc;
use cogito_harness_core::{Cogito, Fingerprint, Project, index::SearchHit, map, pack};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

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
    /// Quality, latency and cost, kept apart. Always measured, printed only by
    /// `--report`; `render_json` never serialises it, so the old JSON output is
    /// byte-identical with the flag off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<Report>,
}

// ─────────────────────────────────────────────────────────────────────────────
// Report: quality, latency and cost, reported separately
// ─────────────────────────────────────────────────────────────────────────────

/// Provenance of one measured worktree.
///
/// A measurement without the revision it ran on and whether that revision had
/// uncommitted work cannot be reproduced, so both are recorded per repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeasuredRepo {
    pub name: String,
    pub path: String,
    /// Exact commit SHA that was measured, or `unknown` outside a repository.
    pub commit: String,
    /// False when the path is not a git working tree.
    pub is_git: bool,
    /// True only when `git status --porcelain --untracked-files=all` was empty.
    /// A path where git could not be read is never claimed clean.
    pub clean: bool,
    /// Modified, staged and untracked paths that git reported.
    pub dirty_paths: usize,
    /// Time to bring the index up to date before querying, in milliseconds.
    pub index_sync_ms: f64,
}

impl MeasuredRepo {
    /// How the worktree state reads in the report.
    fn state(&self) -> &'static str {
        if !self.is_git {
            "no-git"
        } else if self.clean {
            "clean"
        } else {
            "dirty"
        }
    }
}

/// One row of the report, for one task or one search term.
///
/// Quality, latency and cost live in separate fields. Nothing here multiplies
/// or adds them into a single score, because a fast wrong answer and a slow
/// right one are not the same thing and one number would hide that.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Measurement {
    pub label: String,
    /// `task` for a labelled gold-set task, `term` for a search term.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<usize>,
    /// quality: expected entries the pack returned, over the expected total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_pct: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_hits: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall_expected: Option<usize>,
    /// latency: the query this row measured, in milliseconds.
    pub query_ms: f64,
    /// cost: tokens the pack actually used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pack_tokens: Option<usize>,
    /// cost: tool calls the Cogito strategy needs for this row.
    pub tool_calls: usize,
    /// cost: tool calls plain exploration needs for the same row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_tool_calls: Option<usize>,
}

/// The measurement report: three classes, three sections, never one score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    /// How many times the measurement was repeated.
    pub repetitions: usize,
    /// Every worktree that was measured, with its SHA and clean state.
    pub repos: Vec<MeasuredRepo>,
    /// Every task and term that was measured.
    pub measurements: Vec<Measurement>,
    /// The labelled gold set the quality column was read from, when there was
    /// one. `None` means no recall could be measured, never that it was zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gold_path: Option<String>,
    /// Why the quality column is empty, when it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality_note: Option<String>,
}

impl Report {
    fn new(repetitions: usize) -> Self {
        Self {
            repetitions: repetitions.max(1),
            repos: Vec::new(),
            measurements: Vec::new(),
            gold_path: None,
            quality_note: None,
        }
    }

    /// Render the three classes as three separate sections.
    ///
    /// `artifacts` is the run directory, printed so the numbers can be traced
    /// back to the raw file that produced them.
    pub fn render(&self, tokenizer: &str, artifacts: Option<&Path>) -> String {
        let mut out = String::new();
        out.push_str("cogito bench report\n");
        out.push_str(&format!(
            "repetitions: {}   tokenizer: {}\n",
            self.repetitions, tokenizer
        ));
        if let Some(dir) = artifacts {
            out.push_str(&format!("artifacts:   {}\n", dir.display()));
        }

        // Provenance
        out.push_str("\nPROVENANCE — the measured worktree\n");
        out.push_str(&format!(
            "{:<20} {:<14} {:<8} {:>7}  {:>13}\n",
            "repo", "commit", "worktree", "dirty", "index_sync_ms"
        ));
        for repo in &self.repos {
            out.push_str(&format!(
                "{:<20} {:<14} {:<8} {:>7}  {:>13.1}\n",
                repo.name,
                short_commit(&repo.commit),
                repo.state(),
                repo.dirty_paths,
                repo.index_sync_ms
            ));
        }

        // Quality
        out.push_str("\nQUALITY — recall@budget (higher is better)\n");
        let tasks: Vec<&Measurement> = self
            .measurements
            .iter()
            .filter(|m| m.recall_pct.is_some())
            .collect();
        if tasks.is_empty() {
            out.push_str(&format!(
                "n/a — {}\n",
                self.quality_note
                    .as_deref()
                    .unwrap_or("no labelled gold set")
            ));
        } else {
            out.push_str(&format!(
                "{:<10} {:<5} {:>7} {:>5} {:>9} {:>8}\n",
                "task", "lang", "budget", "hits", "expected", "recall"
            ));
            for task in &tasks {
                out.push_str(&format!(
                    "{:<10} {:<5} {:>7} {:>5} {:>9} {:>7.1}%\n",
                    task.label,
                    task.language.as_deref().unwrap_or("-"),
                    task.budget
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "-".into()),
                    task.recall_hits.unwrap_or(0),
                    task.recall_expected.unwrap_or(0),
                    task.recall_pct.unwrap_or(0.0),
                ));
            }
            out.push_str(&format!(
                "{:<10} {:<5} {:>7} {:>5} {:>9} {:>7.1}%\n",
                "mean",
                "all",
                "",
                "",
                "",
                mean(&tasks).unwrap_or(0.0)
            ));
            for lang in ["en", "es"] {
                let rows: Vec<&Measurement> = tasks
                    .iter()
                    .copied()
                    .filter(|m| m.language.as_deref() == Some(lang))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "{:<10} {:<5} {:>7} {:>5} {:>9} {:>7.1}%\n",
                    "mean",
                    lang,
                    "",
                    "",
                    "",
                    mean(&rows).unwrap_or(0.0)
                ));
            }
        }

        // Latency
        out.push_str("\nLATENCY — milliseconds (lower is better)\n");
        out.push_str("index sync\n");
        out.push_str(&format!("{:<18} {:>13}\n", "repo", "index_sync_ms"));
        for repo in &self.repos {
            out.push_str(&format!("{:<18} {:>13.1}\n", repo.name, repo.index_sync_ms));
        }
        out.push_str("query\n");
        out.push_str(&format!("{:<18} {:>13}\n", "label", "query_ms"));
        for m in &self.measurements {
            out.push_str(&format!("{:<18} {:>13.1}\n", m.label, m.query_ms));
        }

        // Cost
        out.push_str("\nCOST — pack tokens and tool calls (lower is better)\n");
        out.push_str(&format!(
            "{:<18} {:>11} {:>10} {:>15}\n",
            "label", "pack_tokens", "tool_calls", "baseline_calls"
        ));
        for m in &self.measurements {
            out.push_str(&format!(
                "{:<18} {:>11} {:>10} {:>15}\n",
                m.label,
                m.pack_tokens
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "-".into()),
                m.tool_calls,
                m.baseline_tool_calls
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".into()),
            ));
        }

        out
    }
}

/// First 12 characters of a SHA, or the whole string when it is not one.
fn short_commit(commit: &str) -> String {
    commit.chars().take(12).collect()
}

fn mean(rows: &[&Measurement]) -> Option<f64> {
    let values: Vec<f64> = rows.iter().filter_map(|m| m.recall_pct).collect();
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
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
///
/// In a git repository this is what an agent would see: tracked files plus
/// untracked files that `.gitignore` does not exclude. Walking the directory
/// instead would count `node_modules` and generated output, which no agent
/// lists, and inflate the baseline by orders of magnitude.
fn list_all_files(root: &Path) -> Vec<PathBuf> {
    if let Some(files) = git_lines(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    ) {
        let mut files: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
        files.sort();
        return files;
    }
    let mut files = Vec::new();
    walkdir(root, &mut files);
    files.sort();
    files
}

/// Whether `root` is the top of a git working tree (not merely inside one).
fn is_git_top(root: &Path) -> bool {
    let top = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let (Some(top), Ok(root)) = (top, root.canonicalize()) else {
        return false;
    };
    Path::new(&top) == root
}

/// Lines of a git command run in `root`, NUL-separated when `-z` is in `args`.
/// `None` when `root` is not a git working tree or git fails.
fn git_lines(root: &Path, args: &[&str]) -> Option<Vec<String>> {
    if !is_git_top(root) {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8_lossy(&output.stdout);
    let sep = if args.contains(&"-z") { '\0' } else { '\n' };
    Some(
        text.split(sep)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect(),
    )
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

/// Search `root` for `term` as a text grep does. Returns stdout lines
/// (file:line:content). In a git repository the search covers the same files
/// as `list_all_files`, so the baseline never reads ignored output.
fn grep_sync(root: &Path, term: &str) -> Vec<String> {
    if is_git_top(root) {
        // Exit 1 from git grep means "no match", which is an answer, not a
        // failure: falling back here would search node_modules after all.
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["grep", "-n", "--untracked", "-e", term])
            .output();
        if let Ok(o) = output
            && matches!(o.status.code(), Some(0) | Some(1))
        {
            return String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(str::to_owned)
                .collect();
        }
    }
    let root_str = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let output = std::process::Command::new("grep")
        .args([
            "-rn",
            "--line-number",
            "--exclude-dir=target",
            "--exclude-dir=.git",
            "--exclude-dir=node_modules",
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
///
/// The instance is bound to the repository, not to the process working
/// directory. Besides being the supported constructor, that is what writes
/// `.cogito/.gitignore`, so measuring a checkout does not leave it dirty and
/// the next run still sees the tree it expected.
async fn open_project(path: &Path) -> Result<(Cogito, Project, Fingerprint)> {
    let cogito = Cogito::for_project(path)?;
    let project = cogito.init_project(path.to_path_buf())?;
    let (fingerprint, _) = cogito.fingerprint(&project, false).await?;
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

/// Everything measured once per repository, before any term runs.
///
/// Opening the project and bringing its index up to date is the expensive part.
/// Doing it inside the per-term function would count the same sync once per
/// term and report it as query latency.
struct RepoScan {
    /// False when the project could not be opened or scanned.
    ok: bool,
    card_tokens: (usize, usize),
    fingerprint: Option<Fingerprint>,
    /// Index sync plus fingerprint generation, in milliseconds.
    index_sync_ms: f64,
}

impl RepoScan {
    fn failed(index_sync_ms: f64) -> Self {
        Self {
            ok: false,
            card_tokens: (0, 0),
            fingerprint: None,
            index_sync_ms,
        }
    }
}

/// Open `repo_path` once, bring its index up to date, and time that sync.
async fn scan_repo(repo_path: &Path, counter: &TokenCounter) -> RepoScan {
    let started = Instant::now();
    let result = open_project(repo_path).await;
    let index_sync_ms = started.elapsed().as_secs_f64() * 1000.0;
    match result {
        Ok((_, project, fingerprint)) => {
            let built = map::build(&project.path, &project.name, &fingerprint);
            let card_text = project_card_text(&built);
            let card_tokens = counter.count(&card_text);
            RepoScan {
                ok: true,
                card_tokens,
                fingerprint: Some(fingerprint),
                index_sync_ms,
            }
        }
        Err(_) => RepoScan::failed(index_sync_ms),
    }
}

/// Benchmark one search term. Both paths are evaluated over the same set of
/// matched files, so the comparison isolates the reading strategy.
///
/// Returns the legacy term result plus this term's report row: query latency
/// and the tool calls each strategy needs.
async fn benchmark_term_async(
    repo_path: &Path,
    term: &str,
    limit: usize,
    counter: &TokenCounter,
    scan: &RepoScan,
) -> (TermResult, Measurement) {
    // Query latency covers the Cogito path only. The index sync already
    // happened in `scan_repo`, so it is reported once per repository.
    let mut query_ms = 0.0;

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
    // The card was measured once per repository; only search_code is per term.
    let (cogito_finding_tokens, cogito_finding_bytes) = match scan.fingerprint.as_ref() {
        Some(fingerprint) => {
            let started = Instant::now();
            let (search_text, _hits) = search_code_text(repo_path, fingerprint, term);
            let search_count = counter.count(&search_text);
            query_ms += started.elapsed().as_secs_f64() * 1000.0;
            (
                scan.card_tokens.0 + search_count.0,
                scan.card_tokens.1 + search_count.1,
            )
        }
        // Without a usable index the Cogito path is not a measurement, so the
        // old fallback (charge it the baseline) is kept for the token columns.
        None => (baseline_finding_tokens, baseline_finding_bytes),
    };

    // ── Determine the matched file set ───────────────────────────────────────
    // Use the baseline grep to determine which files match, so the comparison
    // isolates the reading strategy.
    let matched_by_file = parse_grep_lines(&grep_output);
    let matched_files: Vec<_> = matched_by_file.into_iter().take(limit).collect();

    if matched_files.is_empty() {
        let result = TermResult {
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
        // A term with no matches is still one tool call per strategy: the
        // search happened, and that it found nothing is the answer.
        let measurement = term_measurement(
            term,
            query_ms,
            if scan.ok { 2 } else { 0 },
            baseline_tool_calls(0),
        );
        return (result, measurement);
    }

    let matched_files_count = matched_files.len();

    // Get fingerprint for outline rendering
    let fingerprint = scan.fingerprint.as_ref();
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
    let started = Instant::now();
    for (file, matched_lines) in &matched_files {
        let outline_text = file_outline_text(&root_for_reading, fingerprint, file);
        let (t, b) = counter.count(&outline_text);
        cogito_reading_tokens += t;
        cogito_reading_bytes += b;

        let range_text = read_matched_ranges(&root_for_reading, file, matched_lines);
        let (t, b) = counter.count(&range_text);
        cogito_reading_tokens += t;
        cogito_reading_bytes += b;
    }
    query_ms += started.elapsed().as_secs_f64() * 1000.0;

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

    let result = TermResult {
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
    };
    let measurement = term_measurement(
        term,
        query_ms,
        // card + search_code, then an outline and a range read per file.
        if scan.ok {
            2 + 2 * matched_files_count
        } else {
            0
        },
        baseline_tool_calls(matched_files_count),
    );
    (result, measurement)
}

/// Tool calls plain exploration needs for a term: list the files, grep, then
/// read each matched file in full.
fn baseline_tool_calls(matched_files: usize) -> usize {
    2 + matched_files
}

/// The cost and latency row of one search term. Quality is left empty: a search
/// term has no ground truth, so there is no recall to report for it.
fn term_measurement(
    term: &str,
    query_ms: f64,
    tool_calls: usize,
    baseline_tool_calls: usize,
) -> Measurement {
    Measurement {
        label: term.to_string(),
        kind: "term".to_string(),
        language: None,
        budget: None,
        recall_pct: None,
        recall_hits: None,
        recall_expected: None,
        query_ms,
        pack_tokens: None,
        tool_calls,
        baseline_tool_calls: Some(baseline_tool_calls),
    }
}
// ─────────────────────────────────────────────────────────────────────────────
// Provenance and raw artifacts
// ─────────────────────────────────────────────────────────────────────────────

/// Run one git command in `path` and return its trimmed stdout.
fn git_capture(path: &Path, args: &[&str]) -> Option<String> {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// Read the revision and the clean state of the worktree that was measured.
///
/// A path where `git status` cannot be read is reported as `no-git`, never as
/// clean: claiming a clean tree that was never checked is worse than saying
/// nothing.
fn worktree_state(path: &Path) -> (String, bool, bool, usize) {
    let commit = git_capture(path, &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    if !is_git_top(path) {
        return (commit, false, false, 0);
    }
    match git_capture(path, &["status", "--porcelain=v1", "--untracked-files=all"]) {
        Some(status) => {
            let dirty = status.lines().filter(|l| !l.trim().is_empty()).count();
            (commit, true, dirty == 0, dirty)
        }
        None => (commit, true, false, 0),
    }
}

/// The root that holds one `<timestamp>/` directory per run.
///
/// `COGITO_BENCH_OUT_DIR` wins, then `XDG_CACHE_HOME`, then the documented
/// default of `~/.cache/cogito/bench-runs`. The artifacts live outside every
/// measured worktree on purpose: writing them into the tree being measured
/// would dirty it between runs and change what the next run sees.
pub fn default_out_dir() -> PathBuf {
    for key in ["COGITO_BENCH_OUT_DIR", "XDG_CACHE_HOME"] {
        if let Ok(dir) = std::env::var(key)
            && !dir.trim().is_empty()
        {
            return if key == "XDG_CACHE_HOME" {
                PathBuf::from(dir).join("cogito/bench-runs")
            } else {
                PathBuf::from(dir)
            };
        }
    }
    if let Ok(home) = std::env::var("HOME")
        && !home.trim().is_empty()
    {
        return PathBuf::from(home).join(".cache/cogito/bench-runs");
    }
    PathBuf::from(".cogito/bench-runs")
}

/// One run's raw artifact. Every run gets one, including a run that failed and
/// terms that matched nothing: discarding those is how a benchmark starts
/// reporting only its good days.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunArtifact {
    pub schema: String,
    pub started_at: String,
    pub repetitions: usize,
    pub runs: Vec<BenchmarkResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Create this run's `<timestamp>/` directory below `root`.
pub fn create_run_dir(root: &Path) -> Result<PathBuf> {
    fs::create_dir_all(root)
        .with_context(|| format!("Failed to create the artifact root {}", root.display()))?;
    let stamp = Utc::now().format("%Y-%m-%dT%H-%M-%SZ").to_string();
    let mut dir = root.join(&stamp);
    let mut suffix = 1;
    while dir.exists() {
        suffix += 1;
        dir = root.join(format!("{stamp}-{suffix}"));
    }
    fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create the run directory {}", dir.display()))?;
    Ok(dir)
}

/// Write the raw JSON and the rendered text of one run into `dir`.
pub fn write_artifact(dir: &Path, artifact: &RunArtifact, rendered: &str) -> Result<()> {
    let json = serde_json::to_string_pretty(artifact).context("artifact serialisation failed")?;
    fs::write(dir.join("run.json"), json)
        .with_context(|| format!("Failed to write {}/run.json", dir.display()))?;
    fs::write(dir.join("report.txt"), rendered)
        .with_context(|| format!("Failed to write {}/report.txt", dir.display()))?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────────
// Quality: recall@budget over the labelled gold set
// ─────────────────────────────────────────────────────────────────────────────

/// A repository the gold tasks are measured against.
#[derive(Debug, Clone, Deserialize)]
struct GoldRepo {
    name: String,
    /// A local checkout, used for a repository that is not one of the pinned
    /// public clones.
    #[serde(default)]
    root: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    commit: Option<String>,
}

/// What a person would consider necessary to answer a task.
#[derive(Debug, Clone, Default, Deserialize)]
struct GoldExpect {
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    symbols: Vec<String>,
}

/// One labelled task.
#[derive(Debug, Clone, Deserialize)]
struct GoldTask {
    id: String,
    repo: String,
    #[serde(default)]
    language: String,
    budget: usize,
    task: String,
    #[serde(default)]
    expect: GoldExpect,
}

/// The labelled gold set.
#[derive(Debug, Clone, Deserialize)]
struct GoldSet {
    #[serde(default)]
    repos: Vec<GoldRepo>,
    tasks: Vec<GoldTask>,
}

/// The fragments a rendered pack contains, as (path, symbol name).
///
/// The header is `path:start-end · kind · name · reason`. This mirrors the
/// parser in `tests/context_pack_recall.rs`; both grade the same rendered
/// format, and the format is `pack::render`'s contract.
fn pack_fragments(rendered: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in rendered.lines() {
        if line.starts_with("context_pack ") {
            continue;
        }
        let Some((location, rest)) = line.split_once(" · ") else {
            continue;
        };
        let Some((path, span)) = location.rsplit_once(':') else {
            continue;
        };
        let Some((start, end)) = span.split_once('-') else {
            continue;
        };
        if start.parse::<usize>().is_err() || end.parse::<usize>().is_err() {
            continue;
        }
        let name = rest.split(" · ").nth(1).unwrap_or_default().to_string();
        out.push((path.to_string(), name));
    }
    out
}

/// Score one pack against what a person would need: (hits, expected, missing).
fn score_pack(rendered: &str, expect: &GoldExpect) -> (usize, usize, Vec<String>) {
    let fragments = pack_fragments(rendered);
    let mut hits = 0usize;
    let mut missing = Vec::new();
    for want in &expect.files {
        if fragments.iter().any(|(path, _)| path == want) {
            hits += 1;
        } else {
            missing.push(format!("file {want}"));
        }
    }
    for want in &expect.symbols {
        let want_lower = want.to_lowercase();
        if fragments
            .iter()
            .any(|(_, name)| name.to_lowercase() == want_lower)
        {
            hits += 1;
        } else {
            missing.push(format!("symbol {want}"));
        }
    }
    (hits, expect.files.len() + expect.symbols.len(), missing)
}

/// Where a task's repository lives: an explicit `root`, else the bench cache
/// the pinned manifest clones into.
fn gold_repo_root(workspace: &Path, gold: &GoldSet, name: &str) -> PathBuf {
    if let Some(root) = gold
        .repos
        .iter()
        .find(|r| r.name == name)
        .and_then(|r| r.root.as_ref())
    {
        return PathBuf::from(root);
    }
    workspace.join(".cogito/bench-cache").join(name)
}

/// A repository opened and made fresh, ready to answer its tasks.
struct GoldRepoState {
    cogito: Cogito,
    project: Project,
}

/// Measure recall@budget and the pack cost of every labelled task.
///
/// The pack is built with the same `pack::build`, the same stored fingerprint
/// and the same shared `cl100k_base` counter the MCP `context_pack` tool uses,
/// so the number describes the tool an agent gets rather than a reimplementation
/// of it. A missing checkout is reported and skipped; it is never a miss and
/// never a zero.
///
/// Returns `(measurements, repos, gold_path_used, note)`. `gold_path_used` is
/// `None` only when no gold set was read at all, so an empty quality column is
/// never confused with a measured zero.
async fn measure_gold(
    workspace: &Path,
    gold_path: Option<&Path>,
    counter: &TokenCounter,
) -> (
    Vec<Measurement>,
    Vec<MeasuredRepo>,
    Option<String>,
    Option<String>,
) {
    let path = gold_path
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("benchmarks/context_pack_gold.json"));
    let Ok(content) = fs::read_to_string(&path) else {
        return (
            Vec::new(),
            Vec::new(),
            None,
            Some(format!("no labelled gold set at {}", path.display())),
        );
    };
    let gold: GoldSet = match serde_json::from_str(&content) {
        Ok(gold) => gold,
        Err(e) => {
            return (
                Vec::new(),
                Vec::new(),
                None,
                Some(format!(
                    "gold set at {} is not valid JSON: {e}",
                    path.display()
                )),
            );
        }
    };
    let gold_path_used = Some(path.display().to_string());

    // Phase 1: open and sync each repository once, not once per task.
    let mut states: BTreeMap<String, GoldRepoState> = BTreeMap::new();
    let mut repos: Vec<MeasuredRepo> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for task in &gold.tasks {
        if !seen.insert(task.repo.clone()) {
            continue;
        }
        let root = gold_repo_root(workspace, &gold, &task.repo);
        if !root.is_dir() {
            notes.push(format!(
                "{}: checkout missing at {}",
                task.repo,
                root.display()
            ));
            continue;
        }
        let (commit, is_git, clean, dirty_paths) = worktree_state(&root);
        let started = Instant::now();
        let opened = match Cogito::for_project(&root) {
            Ok(cogito) => match cogito.load_project(root.clone()) {
                Ok(project) => match cogito.sync_index(&project).await {
                    Ok(_) => Ok((cogito, project)),
                    Err(e) => Err(format!("index sync failed: {e:#}")),
                },
                Err(e) => Err(format!("project could not be registered: {e:#}")),
            },
            Err(e) => Err(format!("project could not be opened: {e:#}")),
        };
        let index_sync_ms = started.elapsed().as_secs_f64() * 1000.0;
        match opened {
            Ok((cogito, project)) => {
                states.insert(task.repo.clone(), GoldRepoState { cogito, project });
                repos.push(MeasuredRepo {
                    name: task.repo.clone(),
                    path: root.display().to_string(),
                    commit,
                    is_git,
                    clean,
                    dirty_paths,
                    index_sync_ms,
                });
            }
            Err(e) => notes.push(format!("{}: {e}", task.repo)),
        }
    }

    // Phase 2: build one pack per task on the already-fresh repositories.
    let mut measurements = Vec::new();
    for task in &gold.tasks {
        let Some(state) = states.get(&task.repo) else {
            continue;
        };
        let stored = state
            .cogito
            .stored_fingerprint(state.project.id)
            .ok()
            .flatten();
        let count = |text: &str| counter.count(text).0;
        let started = Instant::now();
        let built = match state.cogito.connection() {
            Some(conn) => pack::build(
                pack::Request {
                    conn: &conn,
                    project_id: state.project.id,
                    root: &state.project.path,
                    task: &task.task,
                    budget: task.budget,
                    fingerprint: stored.as_ref(),
                    tier_weights: crate::mcp::tier_weights(),
                },
                &count,
            )
            .map(|pack| {
                let rendered = pack::render(&pack);
                let used = pack::used_tokens(&pack, &count);
                (rendered, used)
            })
            .map_err(|e| format!("{e:#}")),
            None => Err("no index connection".to_string()),
        };
        let query_ms = started.elapsed().as_secs_f64() * 1000.0;

        // A build that failed is recorded with its reason, never dropped. It is
        // also not counted as a miss: an answer that was never produced is not
        // recall, and folding it in as zero would make the metric move with the
        // harness instead of with the product.
        let (rendered, pack_tokens, error) = match built {
            Ok((rendered, used)) => (rendered, Some(used), None),
            Err(e) => (String::new(), None, Some(e)),
        };
        let (hits, expected, missing) = score_pack(&rendered, &task.expect);
        let measured = error.is_none() && expected > 0;
        let recall_pct = if measured {
            Some(hits as f64 / expected as f64 * 100.0)
        } else {
            None
        };
        let mut note = error;
        if note.is_none() && !missing.is_empty() {
            note = Some(format!("missed: {}", missing.join(", ")));
        }
        measurements.push(Measurement {
            label: task.id.clone(),
            kind: "task".to_string(),
            language: Some(task.language.clone()),
            budget: Some(task.budget),
            recall_pct,
            recall_hits: measured.then_some(hits),
            recall_expected: measured.then_some(expected),
            query_ms,
            pack_tokens,
            // One context_pack call is the whole Cogito path for a task.
            tool_calls: 1,
            baseline_tool_calls: None,
        });
        if let Some(note) = note {
            notes.push(format!("{}: {note}", task.id));
        }
    }

    let note = if measurements.is_empty() {
        Some(if notes.is_empty() {
            format!("no task was measurable from {}", path.display())
        } else {
            format!("no task was measurable: {}", notes.join("; "))
        })
    } else if notes.is_empty() {
        None
    } else {
        Some(notes.join("; "))
    };

    (measurements, repos, gold_path_used, note)
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
    /// Print the three-section report. Off by default so the old output is
    /// byte-identical with the same flags as before.
    pub report: bool,
    /// Root that holds one `<timestamp>/` artifact per run. `None` means the
    /// documented default, `~/.cache/cogito/bench-runs`.
    pub out_dir: Option<PathBuf>,
    /// How many times to repeat the measurement. Every repetition is kept.
    pub repeat: usize,
    /// Labelled gold set for recall@budget. `None` means the one beside the
    /// manifest, `benchmarks/context_pack_gold.json`.
    pub gold: Option<PathBuf>,
}

impl Default for BenchOptions {
    fn default() -> Self {
        Self {
            repo: None,
            terms: Vec::new(),
            limit: 5,
            report: false,
            out_dir: None,
            repeat: 1,
            gold: None,
        }
    }
}

/// The workspace root, two levels above this crate's manifest.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

/// Run the benchmark with the given options (async to support tokio runtime).
///
/// One call is one repetition. Quality and cost are deterministic, so every
/// repetition reports the same numbers; latency is not, which is why the
/// artifact keeps them all instead of only an average.
pub async fn run_benchmark_async(opts: BenchOptions) -> Result<BenchmarkResult> {
    let counter = TokenCounter::new();
    let tokenizer = counter.name().to_string();
    let workspace = workspace_root();

    let start = Instant::now();
    let mut repos = Vec::new();
    let mut report = Report::new(opts.repeat);

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
            &mut report,
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
                        &mut report,
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

    // Quality: recall@budget over the labelled gold set, built through the same
    // pack the MCP tool builds. Measured on every run so the artifact always
    // carries all three classes, and never invented when a checkout is missing.
    let (quality, gold_repos, gold_path, note) =
        measure_gold(&workspace, opts.gold.as_deref(), &counter).await;
    report.measurements.extend(quality);
    for repo in gold_repos {
        // A repository already reported by the token path keeps its first row.
        if !report.repos.iter().any(|r| r.path == repo.path) {
            report.repos.push(repo);
        }
    }
    report.gold_path = gold_path;
    report.quality_note = note;

    let total_elapsed = start.elapsed().as_secs_f64();

    Ok(BenchmarkResult {
        manifest_version: None,
        repos,
        total_elapsed_seconds: total_elapsed,
        tokenizer,
        report: Some(report),
    })
}

/// Benchmark one repository (async to support tokio runtime).
///
/// The append-only `Report` receives this repository's provenance, its index
/// sync latency and one row per term.
async fn benchmark_repo_async(
    repo_path: &Path,
    repo_name: &str,
    terms: &[String],
    limit: usize,
    counter: &TokenCounter,
    report: &mut Report,
) -> Result<RepoResult> {
    let start = Instant::now();

    let (commit, is_git, clean, dirty_paths) = worktree_state(repo_path);

    // Index once per repository, not once per term: a sync counted per term
    // would multiply the same cost and report it as query latency.
    let scan = scan_repo(repo_path, counter).await;
    report.repos.push(MeasuredRepo {
        // The short name keeps the provenance table readable; the full path is
        // kept beside it, because a name alone does not identify a checkout.
        name: repo_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(repo_name)
            .to_string(),
        path: repo_path.display().to_string(),
        commit: commit.clone(),
        is_git,
        clean,
        dirty_paths,
        index_sync_ms: scan.index_sync_ms,
    });

    let mut term_results = Vec::with_capacity(terms.len());
    for term in terms {
        let (result, measurement) =
            benchmark_term_async(repo_path, term, limit, counter, &scan).await;
        term_results.push(result);
        report.measurements.push(measurement);
    }

    let elapsed = start.elapsed().as_secs_f64();

    Ok(RepoResult {
        repo: repo_name.to_string(),
        commit,
        terms: term_results,
        elapsed_seconds: elapsed,
    })
}

#[cfg(test)]
mod baseline_tests {
    use super::*;
    use tempfile::TempDir;

    fn git(root: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repo with a tracked file, an untracked file, and an ignored
    /// `node_modules` that also contains the term.
    fn repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        git(root, &["init", "-q"]);
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::create_dir_all(root.join("node_modules/lib")).unwrap();
        fs::write(root.join("src/tracked.ts"), "onu tracked\n").unwrap();
        fs::write(root.join("src/untracked.ts"), "onu untracked\n").unwrap();
        fs::write(root.join("node_modules/lib/dep.js"), "onu ignored\n").unwrap();
        git(root, &["add", ".gitignore", "src/tracked.ts"]);
        dir
    }

    #[test]
    fn test_baseline_lists_only_what_an_agent_sees_in_a_repo() {
        let dir = repo();
        let files: Vec<String> = list_all_files(dir.path())
            .into_iter()
            .map(|p| p.display().to_string())
            .collect();
        assert!(files.contains(&"src/tracked.ts".to_string()), "{files:?}");
        assert!(files.contains(&"src/untracked.ts".to_string()), "{files:?}");
        assert!(
            files.iter().all(|f| !f.starts_with("node_modules")),
            "ignored files are not listed: {files:?}"
        );
    }

    #[test]
    fn test_baseline_grep_skips_ignored_matches() {
        let dir = repo();
        let mut hits = grep_sync(dir.path(), "onu");
        hits.sort();
        assert_eq!(hits.len(), 2, "{hits:?}");
        assert!(hits.iter().all(|h| !h.contains("node_modules")), "{hits:?}");
    }

    #[test]
    fn test_a_term_only_in_ignored_files_is_no_match_not_a_fallback() {
        // Exit 1 from git grep is "no match". If it fell back to the directory
        // walk, the ignored node_modules copy would be counted.
        let dir = repo();
        assert!(grep_sync(dir.path(), "dep_only_here").is_empty());
        assert!(grep_sync(dir.path(), "ignored").is_empty());
    }

    /// The recall rule: a file counts when a fragment's path equals it, a symbol
    /// when the name equals it, and a rendered pack line that is not a fragment
    /// header is not a hit.
    #[test]
    fn test_score_pack_counts_files_and_symbols_by_equality() {
        let rendered = "context_pack \"x\" used 10/2000 tokens\n\
             src/a.rs:1-4 · fn · alpha · name match\n\
             src/b.rs:9-12 · fn · Beta · path match\n";
        let expect = GoldExpect {
            files: vec!["src/a.rs".to_string(), "src/missing.rs".to_string()],
            symbols: vec!["beta".to_string(), "gamma".to_string()],
        };
        let (hits, expected, missing) = score_pack(rendered, &expect);
        assert_eq!((hits, expected), (2, 4));
        assert_eq!(missing, vec!["file src/missing.rs", "symbol gamma"]);
    }

    /// A dirty worktree is never reported as clean, and a path git cannot read
    /// is `no-git` rather than dirty.
    #[test]
    fn test_worktree_state_marks_dirty_and_non_git() {
        let dir = TempDir::new().unwrap();
        let (_, is_git, clean, dirty) = worktree_state(dir.path());
        assert!(!is_git && !clean && dirty == 0);

        git(dir.path(), &["init", "-q"]);
        let (_, is_git, clean, dirty) = worktree_state(dir.path());
        assert!(is_git && clean && dirty == 0);

        fs::write(dir.path().join("new.rs"), "fn a() {}\n").unwrap();
        let (_, is_git, clean, dirty) = worktree_state(dir.path());
        assert!(is_git && !clean && dirty == 1, "dirty={dirty}");
    }

    #[test]
    fn test_outside_git_the_fallback_still_skips_node_modules() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("node_modules/lib")).unwrap();
        fs::write(dir.path().join("node_modules/lib/dep.js"), "onu ignored\n").unwrap();
        fs::write(dir.path().join("a.ts"), "onu here\n").unwrap();
        let hits = grep_sync(dir.path(), "onu");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert!(
            hits[0].starts_with("./a.ts") || hits[0].starts_with("a.ts"),
            "{hits:?}"
        );
    }
}
