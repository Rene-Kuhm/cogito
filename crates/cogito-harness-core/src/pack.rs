//! `context_pack`: the fragments a task needs, ranked and inside a budget.
//!
//! An agent that wants to work on something usually pays four or five round
//! trips to get there: `search_code` to find names, `file_outline` per file,
//! then reads, then `related_files` to see what touches what. This module
//! answers the same need in one call by turning a task description into a
//! ranked list of fragments and cutting it at a token budget.
//!
//! The token counter is a parameter rather than a dependency. That keeps the
//! tokenizer crate out of this crate and, more usefully, makes the selection
//! logic testable with a counter a test can predict exactly.
//!
//! Everything here is deterministic: the same task against the same index
//! produces byte-identical output. Nothing iterates a `HashMap` into the
//! result, and every comparison ends in path and line.
//!
//! Ranking multiplies two signals together:
//!
//! - **Where** the term matched: a declaration's name, the file's path, or one
//!   import hop away. This is the brief's `name > path > neighbour` order.
//! - **How much that term narrows the project.** A word matching a thousand
//!   symbols says little; one matching three says a lot. Measured as the number
//!   of candidates the term produced, in integer parts per million.
//!
//! The tier multiplies the rarity rather than gating it. Treating the tier as an
//! absolute wall measured wrong twice on the labelled set: a task phrased with
//! common words filled its whole budget with weak name matches and never reached
//! the one precise path, while a single rare word could not lift its own file
//! above the crowd. With the multipliers below, a name hit on a common word loses
//! to a path hit on a rare one, and two hits in the same tier still order by
//! rarity, which is what the brief's precedence is for.

use crate::index::{self, IndexedDeclaration};
use crate::types::Fingerprint;
use anyhow::Result;
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Budget used when the caller does not ask for one.
pub const DEFAULT_BUDGET: usize = 2000;
/// Largest budget Cogito will honour. A pack bigger than this is a file dump.
pub const MAX_BUDGET: usize = 8000;

/// Task words turned into search terms, at most this many.
const MAX_TERMS: usize = 12;
/// Hits requested from the index per term, to bound how much work one task
/// costs. It no longer caps the term weighting, which counts exactly.
const PER_TERM_LIMIT: usize = 100;
/// Declarations taken from a file that matched by path.
const MAX_DECLS_PER_FILE: usize = 3;
/// Declarations taken from a file pulled in as a neighbour.
const MAX_DECLS_PER_NEIGHBOUR: usize = 2;
/// Candidate files expanded into one import hop, best first.
const NEIGHBOUR_SEEDS: usize = 8;
/// Neighbour files taken per seed.
const MAX_NEIGHBOURS_PER_SEED: usize = 4;
/// A file with no declarations is included whole only up to this many lines.
const WHOLE_FILE_MAX_LINES: usize = 120;
/// Longest task echo in the header, in characters.
const TASK_ECHO_MAX: usize = 120;

/// How much weaker a document is than code with the same kind of match.
///
/// Ten is one tier: a heading is treated as roughly one step down from the
/// symbol it otherwise looks like, so a document still wins when it matches
/// clearly better and loses when the evidence is comparable. A task that asks
/// about documentation switches the penalty off entirely.
const DOC_PENALTY: u64 = 10;

/// Why a fragment was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A task term appears in the declaration's name.
    Name,
    /// A task term appears in the file's path or the file's own name.
    Path,
    /// One import hop away from a candidate, so it is context rather than an
    /// answer.
    Neighbour,
}

/// How strongly each tier counts against the others.
///
/// Deliberately close together: the brief's `name > path > neighbour` order holds
/// for comparable evidence, without letting a weak match in a higher tier bury a
/// strong one in a lower tier. A parameter rather than constants inside the
/// comparison, so the choice can be measured instead of asserted; the recall
/// harness runs alternatives through `COGITO_PACK_TIERS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierWeights {
    pub name: u64,
    pub path: u64,
    pub neighbour: u64,
}

impl Default for TierWeights {
    fn default() -> Self {
        Self {
            name: 10,
            path: 3,
            neighbour: 1,
        }
    }
}

impl TierWeights {
    fn of(self, reason: Reason) -> u64 {
        match reason {
            Reason::Name => self.name,
            Reason::Path => self.path,
            Reason::Neighbour => self.neighbour,
        }
    }

    /// Parse `name/path/neighbour`, e.g. `5/2/1`. `None` when the shape is wrong.
    pub fn parse(spec: &str) -> Option<Self> {
        let mut parts = spec.split('/').map(|p| p.trim().parse::<u64>());
        let weights = Self {
            name: parts.next()?.ok()?,
            path: parts.next()?.ok()?,
            neighbour: parts.next()?.ok()?,
        };
        if parts.next().is_some()
            || weights.name == 0
            || weights.path == 0
            || weights.neighbour == 0
        {
            return None;
        }
        Some(weights)
    }
}

/// One piece of the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fragment {
    pub path: PathBuf,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    /// Declaration kind, or `file` for a short file without declarations.
    pub kind: String,
    pub name: String,
    pub reason: Reason,
    /// Short human reason: `name match`, `path match`, `weak match`,
    /// `imported by src/lib.rs` or `imports src/lib.rs`.
    pub detail: String,
    /// The fragment's own source text.
    pub text: String,
}

impl Fragment {
    /// `path:start-end`, the handle a caller can turn into a range read.
    pub fn location(&self) -> String {
        format!("{}:{}-{}", self.path.display(), self.start, self.end)
    }
}

/// A ranked, budgeted set of fragments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    pub task: String,
    pub budget: usize,
    /// Tokens the rendered pack occupies, header and trailer included.
    pub used: usize,
    pub fragments: Vec<Fragment>,
    /// Candidates the index offered, after deduplication and before the cut.
    pub candidates: usize,
    /// Candidates that did not fit the budget.
    pub skipped: usize,
    /// True when the budget was too small for even the shortest honest answer,
    /// in which case the render is empty rather than over budget.
    pub too_small: bool,
}

/// What to pack, and against which index.
pub struct Request<'a> {
    pub conn: &'a Connection,
    pub project_id: Uuid,
    /// Project root, for reading fragment text.
    pub root: &'a Path,
    pub task: &'a str,
    pub budget: usize,
    /// Stored fingerprint, when one exists, for change counts in the tie-break.
    pub fingerprint: Option<&'a Fingerprint>,
    /// Relative strength of the name, path and neighbour tiers.
    pub tier_weights: TierWeights,
}

/// A candidate before its text is loaded.
struct Candidate {
    path: PathBuf,
    start: usize,
    end: usize,
    kind: String,
    name: String,
    reason: Reason,
    detail: String,
    /// How many task terms this candidate matched.
    terms: usize,
    /// Summed rarity of the matched terms, in integer parts per million.
    ///
    /// A term that matches a thousand symbols says much less about relevance
    /// than one that matches three, and without this a task phrased with common
    /// words fills its budget with whatever happens to sort first. Integers, not
    /// floats, so the comparison stays exact and reproducible.
    weight: u64,
    /// Whether this fragment is documentation rather than code.
    ///
    /// Headings arrive from the index as symbols and were scored like a
    /// function, which is how an archived proposal's h1 outranked the detector
    /// it describes. Documents rank one tier down unless the task asks for
    /// documentation.
    is_doc: bool,
    is_test: bool,
}

impl Candidate {
    /// How strong this candidate's evidence is, every factor included.
    fn evidence(&self, tiers: TierWeights, asks_for_docs: bool) -> u64 {
        let base = tiers.of(self.reason).saturating_mul(self.weight);
        if asks_for_docs || !self.is_doc {
            base.saturating_mul(DOC_PENALTY)
        } else {
            base
        }
    }
}

/// Rarity of one matched term, in parts per million: rarer is larger.
fn term_weight(document_frequency: usize) -> u64 {
    1_000_000 / (1 + document_frequency as u64)
}

/// What the index told us about one candidate, accumulated across terms.
#[derive(Default, Clone)]
struct Accum {
    name: String,
    terms: BTreeSet<String>,
    weight: u64,
    name_matched: bool,
    path_matched: bool,
    is_test: bool,
}

impl Accum {
    /// The honest reason for including this candidate.
    fn reason(&self) -> Reason {
        if self.name_matched {
            Reason::Name
        } else {
            Reason::Path
        }
    }

    fn detail(&self) -> String {
        if self.name_matched {
            "name match".to_string()
        } else if self.path_matched {
            "path match".to_string()
        } else {
            // The index matched something we cannot attribute to a name or a
            // path. Saying so beats claiming a match that is not there.
            "weak match".to_string()
        }
    }
}

/// Build the pack. `count` returns the token cost of a string.
pub fn build(req: Request<'_>, count: &dyn Fn(&str) -> usize) -> Result<Pack> {
    let budget = req.budget.clamp(1, MAX_BUDGET);
    let terms = task_terms(req.task);
    let mut pack = Pack {
        task: req.task.to_string(),
        budget,
        used: 0,
        fragments: Vec::new(),
        candidates: 0,
        skipped: 0,
        too_small: false,
    };
    if terms.is_empty() {
        return Ok(pack);
    }

    let indexed: BTreeSet<PathBuf> = index::files(req.conn, req.project_id)?
        .into_iter()
        .collect();
    let changes = change_counts(req.fingerprint);
    let asks_for_docs = asks_for_documentation(req.task);
    let found = candidates(
        &req,
        &terms,
        &indexed,
        &changes,
        req.tier_weights,
        asks_for_docs,
    )?;
    pack.candidates = found.len();

    // Load text, clamp ranges to what the file really has, and spend budget in
    // rank order. A fragment that does not fit is skipped rather than ending
    // the walk, so a small later fragment can still use the space.
    let mut files: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    let mut kept: Vec<Fragment> = Vec::new();
    for c in &found {
        let lines = files
            .entry(c.path.clone())
            .or_insert_with(|| read_lines(req.root, &c.path));
        let total = lines.len();
        let start = c.start.max(1);
        if total == 0 || start > total {
            continue;
        }
        let end = c.end.clamp(start, total);
        let fragment = Fragment {
            path: c.path.clone(),
            start,
            end,
            kind: c.kind.clone(),
            name: c.name.clone(),
            reason: c.reason,
            detail: c.detail.clone(),
            text: lines[start - 1..end].join("\n"),
        };

        let mut probe = Pack {
            task: pack.task.clone(),
            budget,
            used: 0,
            fragments: kept.clone(),
            candidates: pack.candidates,
            skipped: 0,
            too_small: false,
        };
        probe.fragments.push(fragment.clone());
        settle(&mut probe, count);
        if probe.used <= budget {
            kept.push(fragment);
            pack.used = probe.used;
        } else {
            pack.skipped += 1;
        }
    }

    pack.fragments = kept;
    settle(&mut pack, count);

    // The header and trailer are counted too, so a budget can be too small for
    // the answer to be honest in. Drop fragments until it fits, and if nothing
    // fits, send nothing rather than an answer over budget.
    while pack.used > budget && !pack.fragments.is_empty() {
        pack.fragments.pop();
        pack.skipped += 1;
        settle(&mut pack, count);
    }
    if pack.used > budget {
        pack.too_small = true;
        pack.used = 0;
    }
    Ok(pack)
}

/// Render the pack as the text an agent receives.
///
/// One header line per fragment — `path:start-end`, kind, name and the short
/// reason — followed by the fragment's own source, because pointers alone would
/// not save the round trips this tool exists to save.
pub fn render(pack: &Pack) -> String {
    if pack.too_small {
        return String::new();
    }
    let task = echo(&pack.task);
    let mut out = format!(
        "context_pack \"{task}\" · budget {} · used {} tokens · {} candidates, {} fragments, {} over budget\n",
        pack.budget,
        pack.used,
        pack.candidates,
        pack.fragments.len(),
        pack.skipped
    );

    if pack.candidates == 0 {
        out.push_str(
            "\nNo matches for this task: no indexed symbol, section or path covers those words. \
             Nothing is returned, because unrelated fragments spend budget without answering \
             anything. Use a name you already know, or read the project map first.\n",
        );
        return out;
    }

    if pack.fragments.is_empty() {
        out.push_str("\nNo fragment fitted the budget. Raise it, or ask a narrower question.\n");
        return out;
    }

    for f in &pack.fragments {
        out.push('\n');
        out.push_str(&format!(
            "{} · {} · {} · {}\n",
            f.location(),
            f.kind,
            f.name,
            f.detail
        ));
        out.push_str(&f.text);
        out.push('\n');
    }
    out
}

/// Tokens the caller actually spent on the rendered pack.
pub fn used_tokens(pack: &Pack, count: &dyn Fn(&str) -> usize) -> usize {
    count(&render(pack))
}

/// Set `used` to the token cost of the pack as it will be sent.
///
/// The header prints `used`, so the number and the text depend on each other.
/// Iterating to a fixed point makes the printed figure match the bytes rather
/// than the previous guess.
fn settle(pack: &mut Pack, count: &dyn Fn(&str) -> usize) {
    for _ in 0..8 {
        let cost = count(&render(pack));
        if cost == pack.used {
            return;
        }
        pack.used = cost;
    }
}

// ---------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------

/// Words that carry no signal in a task description, in both languages the tool
/// accepts. Only affects efficiency: a stopword that slips through costs one
/// search and adds nothing.
const STOPWORDS: &[&str] = &[
    // English
    "the",
    "and",
    "for",
    "with",
    "how",
    "does",
    "what",
    "where",
    "when",
    "which",
    "this",
    "that",
    "from",
    "into",
    "over",
    "are",
    "was",
    "were",
    "been",
    "being",
    "its",
    "use",
    "used",
    "using",
    "get",
    "gets",
    "set",
    "sets",
    "can",
    "should",
    "would",
    "will",
    "not",
    "but",
    "all",
    "any",
    "has",
    "have",
    "had",
    "then",
    "than",
    "there",
    "their",
    "they",
    "you",
    "your",
    "our",
    "who",
    "whom",
    "whose",
    "why",
    "did",
    "done",
    "or",
    "if",
    "else",
    "about",
    "after",
    "before",
    "between",
    "during",
    "more",
    "most",
    "other",
    "some",
    "such",
    "only",
    "own",
    "same",
    "too",
    "very",
    "just",
    "now",
    "also",
    "may",
    "might",
    "must",
    "need",
    "needs",
    "out",
    "one",
    "two",
    // Spanish, accented and plain, since terms are not de-accented
    "que",
    "qué",
    "los",
    "las",
    "del",
    "una",
    "uno",
    "unas",
    "unos",
    "con",
    "por",
    "para",
    "como",
    "cómo",
    "cuando",
    "cuándo",
    "donde",
    "dónde",
    "desde",
    "hasta",
    "este",
    "esta",
    "estos",
    "estas",
    "ese",
    "esa",
    "esos",
    "esas",
    "son",
    "ser",
    "está",
    "estan",
    "están",
    "hay",
    "muy",
    "más",
    "pero",
    "sino",
    "sin",
    "sobre",
    "entre",
    "cada",
    "todo",
    "toda",
    "todos",
    "todas",
    "otro",
    "otra",
    "otros",
    "otras",
    "mismo",
    "misma",
    "además",
    "cuál",
    "cuáles",
    "quién",
    "quiénes",
    "cual",
    "cuales",
    "al",
    "el",
    "la",
    "lo",
    "un",
    "se",
    "su",
    "sus",
    "le",
    "les",
    "mi",
    "mis",
    "ni",
    "ya",
    "si",
    "sí",
    "es",
    "no",
    // Verbs a task uses to ask about code, not names the code carries.
    "defined",
    "implemented",
    "parsed",
    "declared",
    "handled",
    "computed",
];

/// Search terms from a free-text task, sorted and deduplicated so the result
/// does not depend on word order.
pub fn task_terms(task: &str) -> Vec<String> {
    let mut terms: BTreeSet<String> = BTreeSet::new();
    for raw in task.split(|c: char| !c.is_alphanumeric()) {
        let term = raw.to_lowercase();
        if term.chars().count() < 3 || STOPWORDS.contains(&term.as_str()) {
            continue;
        }
        terms.insert(term);
    }
    terms.into_iter().take(MAX_TERMS).collect()
}

/// A term shorter than this must equal the whole name to match it.
///
/// Three letters and fewer are too common to be evidence on their own: `ont`
/// matched `ONT_SN_REGEX` and `OntFormFactor` in a question about signal drift,
/// and those fragments displaced the detector that answers it. An exact name
/// still matches, so a short symbol can be found by its own name.
const MIN_WORD_TERM: usize = 4;

/// Whether `term` names a word in a declaration name.
///
/// A name is split into its words (`requireSession`, `require_session`,
/// `HTTPServer`), and the term matches a word, or a run of consecutive words
/// read together (`requiresession`). A term that is only a fragment of a word
/// does not match: `defined` must not match `undefined`.
fn name_matches(name: &str, term: &str) -> bool {
    let needle = squash(term);
    if needle.is_empty() {
        return false;
    }
    if needle.chars().count() < MIN_WORD_TERM {
        return squash(name) == needle;
    }
    let words = name_words(name);
    (0..words.len()).any(|start| {
        let mut joined = String::new();
        words[start..].iter().any(|word| {
            joined.push_str(word);
            joined == needle
        })
    }) || words.iter().any(|word| word == &needle)
}

/// The words of an identifier, lowercased: split on separators and on
/// camelCase and acronym boundaries.
fn name_words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            continue;
        }
        let starts_word = c.is_uppercase()
            && !current.is_empty()
            && (chars[i - 1].is_lowercase() || chars.get(i + 1).is_some_and(|n| n.is_lowercase()));
        if starts_word {
            words.push(std::mem::take(&mut current));
        }
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Whether `term` appears in a path.
///
/// Separators are kept here: dropping them would let `ab` match `a/b`.
fn path_matches(path: &Path, term: &str) -> bool {
    path.to_string_lossy()
        .to_lowercase()
        .contains(&term.to_lowercase())
}

/// Whether a fragment's file is documentation rather than code.
fn is_doc_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(crate::map::outline::is_doc)
}

/// Where a heading's own section ends.
///
/// The index stores a heading with its whole subtree, which is what a range
/// read wants. As a pack fragment that is too coarse: an h1 above every
/// subsection spends the entire budget on one fragment. A section runs to the
/// line before the next heading of any level; the last heading keeps the end
/// the index gave it.
fn section_end(decls: &[IndexedDeclaration], index: usize) -> usize {
    let start = decls[index].start;
    match decls
        .iter()
        .skip(index + 1)
        .map(|d| d.start)
        .find(|next| *next > start)
    {
        Some(next) => next - 1,
        None => decls[index].end,
    }
}

/// Whether the task asks about documentation rather than about code.
///
/// When it does, documents are not penalised, because the document is the
/// answer. Short forms match as whole words so `doc` does not fire on `docker`;
/// the `documenta…` and `guía` families match by prefix, which covers
/// `documentado`, `documentada`, `documentación` and `documentar`.
fn asks_for_documentation(task: &str) -> bool {
    const WORDS: &[&str] = &["doc", "docs", "spec", "specs", "readme", "markdown"];
    const PREFIXES: &[&str] = &["documenta", "guia", "guía"];
    task.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .any(|word| {
            WORDS.contains(&word.as_str()) || PREFIXES.iter().any(|prefix| word.starts_with(prefix))
        })
}

/// Lowercase, alphanumerics only.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

// ---------------------------------------------------------------------------
// Candidates
// ---------------------------------------------------------------------------

fn candidates(
    req: &Request<'_>,
    terms: &[String],
    indexed: &BTreeSet<PathBuf>,
    changes: &BTreeMap<PathBuf, usize>,
    tiers: TierWeights,
    asks_for_docs: bool,
) -> Result<Vec<Candidate>> {
    let mut found: Vec<Candidate> = Vec::new();
    let mut symbol_hits: BTreeMap<(PathBuf, usize), Accum> = BTreeMap::new();
    let mut file_hits: BTreeMap<PathBuf, Accum> = BTreeMap::new();
    // One outline query per file, however many hits that file produced.
    let mut outlines: BTreeMap<PathBuf, Vec<IndexedDeclaration>> = BTreeMap::new();

    for term in terms {
        let hits = index::search(req.conn, req.project_id, term, PER_TERM_LIMIT)?;
        // How much this term narrows the project. A term matching many
        // candidates is weak evidence; one matching a few is strong evidence.
        // The count is exact and separate from `hits`: a capped count would make
        // every common term look equally common.
        let weight = term_weight(index::count_matches(req.conn, req.project_id, term)?);
        for hit in hits {
            if hit.is_symbol {
                let entry = symbol_hits
                    .entry((hit.path.clone(), hit.line))
                    .or_insert_with(|| Accum {
                        name: hit.name.clone(),
                        is_test: hit.is_test,
                        ..Accum::default()
                    });
                entry.terms.insert(term.clone());
                entry.weight = entry.weight.saturating_add(weight);
                entry.name_matched |= name_matches(&hit.name, term);
                entry.path_matched |= path_matches(&hit.path, term);
            } else {
                let entry = file_hits.entry(hit.path.clone()).or_insert_with(|| Accum {
                    name: hit.name.clone(),
                    is_test: hit.is_test,
                    ..Accum::default()
                });
                entry.terms.insert(term.clone());
                entry.weight = entry.weight.saturating_add(weight);
                // A file hit is a match on the file: its path or its own name.
                // The declarations inside it are how we show it, not what
                // matched, so they all rank as a path match.
                entry.path_matched = true;
            }
        }
    }

    // Symbol hits: the fragment is the declaration that owns the line.
    for ((path, line), acc) in &symbol_hits {
        let decls = outline(&mut outlines, req, path)?;
        let doc = is_doc_path(path);
        let chosen = decls.iter().position(|d| d.name == acc.name).or_else(|| {
            decls
                .iter()
                .position(|d| d.start <= *line && *line <= d.end)
        });
        let (start, end, kind, name) = match chosen {
            Some(i) => {
                let d = &decls[i];
                let end = if doc { section_end(decls, i) } else { d.end };
                (d.start, end, d.kind.clone(), d.name.clone())
            }
            // Not outlineable: keep the line the index reported.
            None => (*line, *line, "line".to_string(), acc.name.clone()),
        };
        found.push(Candidate {
            path: path.clone(),
            start,
            end,
            kind,
            name,
            reason: acc.reason(),
            detail: acc.detail(),
            terms: acc.terms.len(),
            weight: acc.weight,
            is_doc: doc,
            is_test: acc.is_test,
        });
    }

    // Path hits: the fragment is that file's most relevant declarations.
    for (path, acc) in &file_hits {
        let decls = outline(&mut outlines, req, path)?;
        if decls.is_empty() {
            // Nothing to point at, so include a short file whole rather than
            // dumping a long one.
            let total = read_lines(req.root, path).len();
            if total > 0 && total <= WHOLE_FILE_MAX_LINES {
                found.push(Candidate {
                    path: path.clone(),
                    start: 1,
                    end: total,
                    kind: "file".to_string(),
                    name: basename(path),
                    reason: Reason::Path,
                    detail: "path match · whole short file".to_string(),
                    terms: acc.terms.len(),
                    weight: acc.weight,
                    is_doc: is_doc_path(path),
                    is_test: acc.is_test,
                });
            }
            continue;
        }
        let doc = is_doc_path(path);
        let mut ranked: Vec<usize> = (0..decls.len()).collect();
        ranked.sort_by(|&a, &b| {
            let a_named = terms.iter().any(|t| name_matches(&decls[a].name, t));
            let b_named = terms.iter().any(|t| name_matches(&decls[b].name, t));
            b_named
                .cmp(&a_named)
                .then(decls[a].start.cmp(&decls[b].start))
        });
        for &i in ranked.iter().take(MAX_DECLS_PER_FILE) {
            let d = &decls[i];
            let named = terms.iter().any(|t| name_matches(&d.name, t));
            let end = if doc { section_end(decls, i) } else { d.end };
            found.push(Candidate {
                path: path.clone(),
                start: d.start,
                end,
                kind: d.kind.clone(),
                name: d.name.clone(),
                reason: if named { Reason::Name } else { Reason::Path },
                detail: if named {
                    "name match".to_string()
                } else {
                    "path match".to_string()
                },
                terms: acc.terms.len(),
                weight: acc.weight,
                is_doc: doc,
                is_test: acc.is_test,
            });
        }
    }

    // One import hop. Context, so it ranks below a direct match.
    let mut seeds: Vec<PathBuf> = found.iter().map(|c| c.path.clone()).collect();
    seeds.sort();
    seeds.dedup();
    seeds.truncate(NEIGHBOUR_SEEDS);
    for seed in &seeds {
        let mut neighbours: Vec<(PathBuf, String)> = Vec::new();
        for target in index::imports(req.conn, req.project_id, seed)? {
            let path = PathBuf::from(target);
            if indexed.contains(&path) {
                neighbours.push((path, format!("imported by {}", seed.display())));
            }
        }
        for importer in index::importers(req.conn, req.project_id, seed)? {
            if indexed.contains(&importer) {
                neighbours.push((importer, format!("imports {}", seed.display())));
            }
        }
        neighbours.sort();
        neighbours.dedup();
        for (path, detail) in neighbours.into_iter().take(MAX_NEIGHBOURS_PER_SEED) {
            let decls = outline(&mut outlines, req, &path)?;
            for d in decls.iter().take(MAX_DECLS_PER_NEIGHBOUR) {
                found.push(Candidate {
                    path: path.clone(),
                    start: d.start,
                    end: d.end,
                    kind: d.kind.clone(),
                    name: d.name.clone(),
                    reason: Reason::Neighbour,
                    detail: detail.clone(),
                    terms: 0,
                    weight: 0,
                    is_doc: is_doc_path(&path),
                    is_test: index::is_test_path(&path),
                });
            }
        }
    }

    // Rank, then drop duplicates keeping the best-ranked occurrence. The last
    // comparisons make the order total, so the result never depends on hash
    // iteration order.
    found.sort_by(|a, b| {
        b.evidence(tiers, asks_for_docs)
            .cmp(&a.evidence(tiers, asks_for_docs))
            .then(b.terms.cmp(&a.terms))
            .then(a.is_test.cmp(&b.is_test))
            .then(changes_of(changes, b).cmp(&changes_of(changes, a)))
            .then(a.path.cmp(&b.path))
            .then(a.start.cmp(&b.start))
            .then(a.name.cmp(&b.name))
    });
    let mut seen: BTreeSet<(PathBuf, usize, usize)> = BTreeSet::new();
    found.retain(|c| seen.insert((c.path.clone(), c.start, c.end)));
    Ok(found)
}

/// The file's declarations, queried once per file per request.
///
/// Raising the per-term hit limit multiplies the hits a request sees, so the
/// outline has to be fetched per file rather than per hit.
fn outline<'a>(
    cache: &'a mut BTreeMap<PathBuf, Vec<IndexedDeclaration>>,
    req: &Request<'_>,
    path: &Path,
) -> Result<&'a Vec<IndexedDeclaration>> {
    if !cache.contains_key(path) {
        let decls = index::outline(req.conn, req.project_id, path)?;
        cache.insert(path.to_path_buf(), decls);
    }
    cache
        .get(path)
        .ok_or_else(|| anyhow::anyhow!("outline missing for {} after insert", path.display()))
}

/// Change counts per file, from the fingerprint's zones.
fn change_counts(fingerprint: Option<&Fingerprint>) -> BTreeMap<PathBuf, usize> {
    let mut counts: BTreeMap<PathBuf, usize> = BTreeMap::new();
    let Some(fp) = fingerprint else {
        return counts;
    };
    for zone in fp
        .hot_zones
        .iter()
        .chain(&fp.cold_zones)
        .chain(&fp.frozen_zones)
    {
        let entry = counts.entry(zone.path.clone()).or_insert(0);
        *entry = (*entry).max(zone.change_count);
    }
    counts
}

fn changes_of(changes: &BTreeMap<PathBuf, usize>, c: &Candidate) -> usize {
    changes.get(&c.path).copied().unwrap_or(0)
}

fn basename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn echo(task: &str) -> String {
    let trimmed = task.trim();
    if trimmed.chars().count() <= TASK_ECHO_MAX {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(TASK_ECHO_MAX).collect();
    format!("{cut}…")
}

/// The file's lines, lossily decoded so one bad byte does not drop a file.
fn read_lines(root: &Path, path: &Path) -> Vec<String> {
    let Ok(bytes) = std::fs::read(root.join(path)) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&bytes);
    text.lines().map(str::to_string).collect()
}

#[cfg(test)]
mod name_match_tests {
    use super::*;

    #[test]
    fn test_a_term_matches_whole_words_not_inside_other_words() {
        // "defined" is a word in a task, not a fragment of "undefined".
        assert!(!name_matches("undefined", "defined"));
        assert!(!name_matches("entriesWithUndefined", "defined"));
        assert!(name_matches("defined", "defined"));
    }

    #[test]
    fn test_compound_identifiers_match_their_words_and_joined_form() {
        assert!(name_matches("requireSession", "session"));
        assert!(name_matches("require_session", "session"));
        assert!(name_matches("requireSession", "requiresession"));
        assert!(name_matches("requireSession", "require"));
        assert!(name_matches("HTTPServer", "server"));
        assert!(
            !name_matches("requireSession", "sess"),
            "a fragment of a word is not a word"
        );
    }

    #[test]
    fn test_generic_task_verbs_are_not_terms() {
        let terms = task_terms("where is the SNMP catalog defined and how is it parsed");
        assert!(terms.contains(&"snmp".to_string()));
        assert!(terms.contains(&"catalog".to_string()));
        assert!(!terms.contains(&"defined".to_string()));
        assert!(!terms.contains(&"parsed".to_string()));
    }
}
