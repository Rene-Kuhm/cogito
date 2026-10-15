//! What a diff reaches: the declarations its hunks touch, the files that
//! import the changed files, and the files that declare the same symbol names.
//!
//! The answer is an overapproximation on purpose. It comes from the index —
//! the import graph and the declaration names — so it sees a file that imports
//! a changed one and a file that declares a symbol with the same name, but it
//! cannot see a call or a type reference. Every rendering says so.
//!
//! Nothing here is derived from the code itself: a hunk is mapped onto the
//! line ranges the outline already stores, so the tool costs one diff and a
//! handful of index queries.

use crate::map::outline::Declaration;
use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Importers of the changed files at or above which the risk is `high`.
const HIGH_IMPORTERS: usize = 3;
/// Importers at or above which the risk is `medium` (and below `high`).
const MEDIUM_IMPORTERS: usize = 1;

/// One hunk of a diff, in the new file's line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hunk {
    /// First line of the new file the hunk covers (1-based). A hunk that only
    /// deletes from the start of the file reports 0.
    pub start: usize,
    /// How many lines the hunk covers in the new file. A pure deletion covers
    /// none, and is probed at `start` instead.
    pub lines: usize,
}

/// How a file changed in the diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl ChangeStatus {
    /// The word the rendering uses for this status.
    pub fn label(self) -> &'static str {
        match self {
            ChangeStatus::Added => "added",
            ChangeStatus::Modified => "modified",
            ChangeStatus::Deleted => "deleted",
            ChangeStatus::Renamed => "renamed",
        }
    }
}

/// A file the diff changed, with its hunks in the new file's line numbers and
/// the declarations those hunks touch.
#[derive(Debug, Clone)]
pub struct FileChange {
    pub path: PathBuf,
    pub status: ChangeStatus,
    pub hunks: Vec<Hunk>,
    pub touched: Vec<TouchedSymbol>,
}

/// A declaration a changed hunk falls inside.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TouchedSymbol {
    pub file: PathBuf,
    pub name: String,
    pub kind: String,
    pub start: usize,
    pub end: usize,
}

/// How much of the project imports the changed files.
///
/// Derived from the importer count alone: nothing else moves it. The
/// thresholds are arbitrary but fixed, so the same diff always reports the
/// same level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    /// The level a given importer count means.
    pub fn of(importers: usize) -> Self {
        if importers >= HIGH_IMPORTERS {
            Risk::High
        } else if importers >= MEDIUM_IMPORTERS {
            Risk::Medium
        } else {
            Risk::Low
        }
    }

    /// The word the rendering uses for this level.
    pub fn label(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

/// The impact of one diff.
#[derive(Debug, Clone)]
pub struct Impact {
    /// The revision the caller asked to compare against, when any.
    pub base: Option<String>,
    /// Short HEAD the diff was taken at.
    pub head: Option<String>,
    /// Every file the diff changed.
    pub changed: Vec<FileChange>,
    /// A changed file -> the index's files that import it.
    pub importers: BTreeMap<PathBuf, Vec<PathBuf>>,
    /// A touched symbol -> the files that declare the same name, elsewhere.
    pub namers: BTreeMap<String, Vec<PathBuf>>,
    pub risk: Risk,
    /// Why the risk is what it is. Always the importer count, spelled out.
    pub risk_reason: String,
}

/// One file the diff changed, with the content of its new side.
struct Changed {
    file: FileChange,
    /// The new side's content, or `None` when the file does not exist there
    /// (a deleted file, or a binary one).
    content: Option<String>,
}

/// Build the impact of a diff against the index.
///
/// Without `base` the diff is the uncommitted work: the tree at HEAD against
/// the index and the working tree, untracked files included. With `base` it is
/// `base..HEAD`, whose new side is HEAD.
pub fn build(
    conn: &Connection,
    project_id: Uuid,
    root: &Path,
    base: Option<&str>,
) -> Result<Impact> {
    let repo = git2::Repository::discover(root)
        .with_context(|| format!("{} is not inside a git repository", root.display()))?;

    let mut changed: Vec<FileChange> = Vec::new();
    let mut importers: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    let mut namers: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    for change in changed_files(&repo, root, base)? {
        let ext = change
            .file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let content = change.content.unwrap_or_default();
        let declarations = crate::map::outline::declarations(&content, ext);
        let total_lines = content.lines().count();
        let touched: Vec<TouchedSymbol> =
            touched_declarations(&declarations, &change.file.hunks, total_lines)
                .into_iter()
                .map(|decl| TouchedSymbol {
                    file: change.file.path.clone(),
                    name: decl.name.clone(),
                    kind: decl.kind.clone(),
                    start: decl.start,
                    end: decl.end,
                })
                .collect();

        let importers_of = crate::index::importers(conn, project_id, &change.file.path)
            .with_context(|| {
                format!(
                    "cannot look up the importers of {}",
                    change.file.path.display()
                )
            })?;
        if !importers_of.is_empty() {
            importers.insert(change.file.path.clone(), importers_of);
        }

        for symbol in &touched {
            let others: Vec<PathBuf> =
                crate::index::files_declaring(conn, project_id, &symbol.name)
                    .with_context(|| {
                        format!("cannot look up the declarations named {}", symbol.name)
                    })?
                    .into_iter()
                    .filter(|path| path != &symbol.file)
                    .collect();
            if !others.is_empty() {
                namers
                    .entry(symbol.name.clone())
                    .or_default()
                    .extend(others);
            }
        }

        changed.push(FileChange {
            touched,
            ..change.file
        });
    }

    // A name is looked up once per touched symbol, so the same file can arrive
    // twice; and the same file can be both a namer of two symbols.
    for files in namers.values_mut() {
        files.sort();
        files.dedup();
    }

    let importer_count = importers.values().flatten().collect::<BTreeSet<_>>().len();
    Ok(Impact {
        base: base.map(str::to_string),
        head: crate::fingerprint::current_head(root).map(|head| head.chars().take(7).collect()),
        changed,
        importers,
        namers,
        risk: Risk::of(importer_count),
        risk_reason: risk_reason(importer_count),
    })
}

/// The changed files of a diff, with the content of its new side.
fn changed_files(repo: &git2::Repository, root: &Path, base: Option<&str>) -> Result<Vec<Changed>> {
    let mut opts = git2::DiffOptions::new();
    // Only the changed lines matter: context would widen every hunk and drag
    // in declarations the change never touched.
    opts.context_lines(0);

    // In base mode the new side is the HEAD tree; otherwise it is the working
    // tree. `head_tree` is the tree to read that side from in base mode.
    let (diff, head_tree) = if let Some(base) = base {
        let from = repo
            .revparse_single(base)
            .with_context(|| format!("not a revision: {base}"))?
            .peel_to_tree()
            .with_context(|| format!("{base} is not a tree"))?;
        let head = repo
            .head()
            .and_then(|head| head.peel_to_tree())
            .context("HEAD has no commit to compare against")?;
        let diff = repo.diff_tree_to_tree(Some(&from), Some(&head), Some(&mut opts))?;
        (diff, Some(head))
    } else {
        opts.include_untracked(true);
        opts.show_untracked_content(true);
        // An unborn HEAD has no tree, and then everything untracked is the
        // change; git2 wants `None` for that empty side.
        let head = repo.head().ok().and_then(|head| head.peel_to_tree().ok());
        let diff = repo.diff_tree_to_workdir_with_index(head.as_ref(), Some(&mut opts))?;
        (diff, head)
    };

    let mut changed = Vec::new();
    for (index, delta) in diff.deltas().enumerate() {
        let Some(path) = delta.new_file().path().or_else(|| delta.old_file().path()) else {
            continue;
        };
        let path = path.to_path_buf();
        let status = match delta.status() {
            git2::Delta::Added => ChangeStatus::Added,
            git2::Delta::Deleted => ChangeStatus::Deleted,
            git2::Delta::Renamed => ChangeStatus::Renamed,
            _ => ChangeStatus::Modified,
        };
        let content = if base.is_some() {
            head_tree
                .as_ref()
                .and_then(|tree| tree_content(tree, repo, &path))
        } else {
            std::fs::read_to_string(root.join(&path)).ok()
        };
        changed.push(Changed {
            file: FileChange {
                path,
                status,
                hunks: patch_hunks(&diff, index)?,
                touched: Vec::new(),
            },
            content,
        });
    }
    changed.sort_by(|a, b| a.file.path.cmp(&b.file.path));
    Ok(changed)
}

/// The hunks of the delta at `index`, in the new file's line numbers.
fn patch_hunks(diff: &git2::Diff<'_>, index: usize) -> Result<Vec<Hunk>> {
    // A binary file has no lines to map.
    let Some(patch) = git2::Patch::from_diff(diff, index)? else {
        return Ok(Vec::new());
    };
    let mut hunks = Vec::new();
    for hunk_index in 0..patch.num_hunks() {
        let (hunk, _) = patch.hunk(hunk_index)?;
        hunks.push(Hunk {
            start: hunk.new_start() as usize,
            lines: hunk.new_lines() as usize,
        });
    }
    Ok(hunks)
}

/// The content of `path` in a tree, when the tree holds it as a blob.
fn tree_content(tree: &git2::Tree<'_>, repo: &git2::Repository, path: &Path) -> Option<String> {
    let entry = tree.get_path(path).ok()?;
    let object = entry.to_object(repo).ok()?;
    let blob = object.peel_to_blob().ok()?;
    Some(String::from_utf8_lossy(blob.content()).into_owned())
}

/// The declarations the hunks fall inside, in source order.
///
/// A hunk with lines covers `start .. start + lines - 1` in the new file. A
/// pure deletion covers no new line, so it is probed at `start` clamped to the
/// file's length: the declaration that held the removed line is the one that
/// still spans where it was.
fn touched_declarations<'a>(
    declarations: &'a [Declaration],
    hunks: &[Hunk],
    total_lines: usize,
) -> Vec<&'a Declaration> {
    let mut hits: BTreeSet<usize> = BTreeSet::new();
    for hunk in hunks {
        let (start, end) = if hunk.lines > 0 {
            (hunk.start.max(1), hunk.start + hunk.lines - 1)
        } else {
            let probe = hunk.start.max(1).min(total_lines);
            if probe == 0 {
                continue;
            }
            (probe, probe)
        };
        for (index, declaration) in declarations.iter().enumerate() {
            if declaration.start <= end && start <= declaration.end {
                hits.insert(index);
            }
        }
    }
    hits.into_iter().map(|index| &declarations[index]).collect()
}

/// Why the risk is what it is: the importer count, spelled out.
fn risk_reason(importers: usize) -> String {
    let (noun, verb) = if importers == 1 {
        ("file", "imports")
    } else {
        ("files", "import")
    };
    format!("{importers} {noun} {verb} a changed file")
}

/// The files outside the diff the index links to it, by path, each with the
/// reason it is linked.
fn affected(impact: &Impact) -> BTreeMap<PathBuf, Vec<String>> {
    let mut links: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for (changed, importers) in &impact.importers {
        for importer in importers {
            links
                .entry(importer.clone())
                .or_default()
                .insert(format!("imports {}", changed.display()));
        }
    }
    for (symbol, files) in &impact.namers {
        for file in files {
            links
                .entry(file.clone())
                .or_default()
                .insert(format!("names {symbol}"));
        }
    }
    links
        .into_iter()
        .map(|(path, reasons)| (path, reasons.into_iter().collect()))
        .collect()
}

/// Render the impact for an agent, saying what it is and what it is not.
pub fn render(impact: &Impact) -> String {
    let mut out = match &impact.base {
        Some(base) => format!("Impact of changes in {base}..HEAD"),
        None => "Impact of uncommitted changes".to_string(),
    };
    if let Some(head) = &impact.head {
        out.push_str(&format!(" · HEAD {head}"));
    }
    if impact.changed.is_empty() {
        out.push_str("\nNothing changed; there is no impact to report.\n");
        return out;
    }
    out.push_str(&format!(
        " · {} {} changed\n",
        impact.changed.len(),
        if impact.changed.len() == 1 {
            "file"
        } else {
            "files"
        }
    ));

    let touched: Vec<&TouchedSymbol> = impact.changed.iter().flat_map(|c| &c.touched).collect();
    if touched.is_empty() {
        out.push_str("\nChanged symbols:\n  no declarations touched\n");
    } else {
        out.push_str(&format!("\nChanged symbols ({}):\n", touched.len()));
        for symbol in &touched {
            out.push_str(&format!(
                "  {}:{}-{} {} {}\n",
                symbol.file.display(),
                symbol.start,
                symbol.end,
                symbol.kind,
                symbol.name
            ));
        }
    }

    out.push_str(&format!("\nChanged files ({}):\n", impact.changed.len()));
    for change in &impact.changed {
        out.push_str(&format!(
            "  {} — {}\n",
            change.path.display(),
            change.status.label()
        ));
    }

    let links = affected(impact);
    if links.is_empty() {
        out.push_str("\nAffected files: none.\n");
    } else {
        out.push_str(&format!("\nAffected files ({}):\n", links.len()));
        for (path, reasons) in &links {
            out.push_str(&format!("  {} — {}\n", path.display(), reasons.join("; ")));
        }
    }

    out.push_str(&format!(
        "\nRisk: {} · {}\n",
        impact.risk.label(),
        impact.risk_reason
    ));
    out.push_str(
        "\nThis is an overapproximation: the import graph and the declaration names come \
from the index, which holds no calls or type references.\n",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(kind: &str, name: &str, start: usize, end: usize) -> Declaration {
        Declaration {
            name: name.to_string(),
            kind: kind.to_string(),
            start,
            end,
            signature: None,
        }
    }

    fn names(declarations: Vec<&Declaration>) -> Vec<&str> {
        declarations
            .into_iter()
            .map(|declaration| declaration.name.as_str())
            .collect()
    }

    #[test]
    fn test_a_hunk_touches_the_declarations_whose_range_it_covers() {
        let declarations = [
            declaration("function", "a", 1, 3),
            declaration("const", "B", 5, 5),
            declaration("function", "c", 7, 10),
        ];

        // A one-line replacement inside `a`.
        assert_eq!(
            names(touched_declarations(
                &declarations,
                &[Hunk { start: 2, lines: 1 }],
                10
            )),
            ["a"]
        );
        // A hunk ending on the blank line after `a` still only reaches `a`.
        assert_eq!(
            names(touched_declarations(
                &declarations,
                &[Hunk { start: 3, lines: 2 }],
                10
            )),
            ["a"]
        );
        // A hunk that spans the whole file reaches all three, once each.
        assert_eq!(
            names(touched_declarations(
                &declarations,
                &[Hunk {
                    start: 1,
                    lines: 10
                }],
                10
            )),
            ["a", "B", "c"]
        );
        // Two hunks inside the same declaration still name it once.
        assert_eq!(
            names(touched_declarations(
                &declarations,
                &[
                    Hunk { start: 2, lines: 1 },
                    Hunk { start: 7, lines: 2 },
                    Hunk { start: 8, lines: 1 },
                ],
                10
            )),
            ["a", "c"]
        );
    }

    #[test]
    fn test_a_pure_deletion_probes_the_line_it_removed() {
        // Deleting the last line of a three-line file leaves two lines and the
        // deletion is reported at line 3; the probe clamps to the last line.
        let declarations = [declaration("function", "a", 1, 2)];
        assert_eq!(
            names(touched_declarations(
                &declarations,
                &[Hunk { start: 3, lines: 0 }],
                2
            )),
            ["a"]
        );

        // A deletion that empties the file has no line left to probe.
        assert!(touched_declarations(&declarations, &[Hunk { start: 0, lines: 0 }], 0).is_empty());
    }

    #[test]
    fn test_a_comment_above_the_first_declaration_touches_nothing() {
        let declarations = [declaration("function", "a", 2, 4)];
        assert!(
            touched_declarations(&declarations, &[Hunk { start: 1, lines: 1 }], 4).is_empty(),
            "a comment added above a declaration is not part of its line range"
        );
    }

    #[test]
    fn test_risk_comes_only_from_the_importer_count() {
        assert_eq!(Risk::of(0), Risk::Low);
        assert_eq!(Risk::of(1), Risk::Medium);
        assert_eq!(Risk::of(2), Risk::Medium);
        assert_eq!(Risk::of(3), Risk::High);
        assert_eq!(Risk::of(30), Risk::High);

        assert_eq!(risk_reason(0), "0 files import a changed file");
        assert_eq!(risk_reason(1), "1 file imports a changed file");
        assert_eq!(risk_reason(3), "3 files import a changed file");
    }

    #[test]
    fn test_render_says_when_no_declaration_is_touched() {
        let impact = Impact {
            base: None,
            head: Some("abc1234".to_string()),
            changed: vec![FileChange {
                path: PathBuf::from("src/a.ts"),
                status: ChangeStatus::Modified,
                hunks: vec![Hunk { start: 1, lines: 1 }],
                touched: Vec::new(),
            }],
            importers: BTreeMap::new(),
            namers: BTreeMap::new(),
            risk: Risk::Low,
            risk_reason: risk_reason(0),
        };

        let text = render(&impact);
        assert!(text.contains("no declarations touched"), "{text}");
        assert!(text.contains("src/a.ts"), "{text}");
        assert!(text.contains("Risk: low"), "{text}");
        assert!(text.contains("overapproximation"), "{text}");
    }

    #[test]
    fn test_render_attributes_each_affected_file_to_its_reason() {
        let impact = Impact {
            base: Some("HEAD~1".to_string()),
            head: Some("abc1234".to_string()),
            changed: vec![FileChange {
                path: PathBuf::from("src/core.ts"),
                status: ChangeStatus::Modified,
                hunks: vec![Hunk { start: 2, lines: 1 }],
                touched: vec![TouchedSymbol {
                    file: PathBuf::from("src/core.ts"),
                    name: "computeValue".to_string(),
                    kind: "function".to_string(),
                    start: 1,
                    end: 3,
                }],
            }],
            importers: BTreeMap::from([(
                PathBuf::from("src/core.ts"),
                vec![PathBuf::from("src/consumer.ts")],
            )]),
            namers: BTreeMap::from([(
                "computeValue".to_string(),
                vec![
                    PathBuf::from("src/consumer.ts"),
                    PathBuf::from("src/dup.ts"),
                ],
            )]),
            risk: Risk::Medium,
            risk_reason: risk_reason(1),
        };

        let text = render(&impact);
        assert!(
            text.contains("src/consumer.ts — imports src/core.ts; names computeValue"),
            "{text}"
        );
        assert!(text.contains("src/dup.ts — names computeValue"), "{text}");
        assert!(
            text.starts_with("Impact of changes in HEAD~1..HEAD"),
            "{text}"
        );
        assert!(
            text.contains("Risk: medium · 1 file imports a changed file"),
            "{text}"
        );
    }

    #[test]
    fn test_render_says_when_nothing_changed() {
        let impact = Impact {
            base: None,
            head: None,
            changed: Vec::new(),
            importers: BTreeMap::new(),
            namers: BTreeMap::new(),
            risk: Risk::Low,
            risk_reason: risk_reason(0),
        };

        let text = render(&impact);
        assert!(text.contains("Nothing changed"), "{text}");
        assert!(!text.contains("Risk:"), "{text}");
    }
}
