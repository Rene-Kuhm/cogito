//! The project map: what an agent needs to find its way around a codebase
//! without reading it.
//!
//! Everything here is derived from a stored [`Fingerprint`], so building a
//! map, listing a file's neighbours or searching paths reads no source files.
//! The renderers are deliberately terse: their output is spent as agent
//! context, so every line has to save more reading than it costs.

pub(crate) mod layout;
pub mod outline;
pub mod symbols;

use crate::types::{Fingerprint, ModuleInfo};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Lines of hot files shown per module, and in the project-wide list.
const HOT_PER_MODULE: usize = 3;
const HOT_OVERALL: usize = 10;
/// Cap on each list in `related` output.
const LIST_LIMIT: usize = 25;
/// A file changed once is not "hot"; listing it would only spend tokens.
const MIN_HOT_CHANGES: usize = 2;

/// One module (a sub-project with a manifest, or a top-level directory).
#[derive(Debug, Clone)]
pub struct ModuleSummary {
    pub path: PathBuf,
    pub name: String,
    pub files: usize,
    pub entry: Option<PathBuf>,
    /// Most changed files, relative to the module, with their change counts.
    pub hot: Vec<(PathBuf, usize)>,
    /// Other modules this one imports from.
    pub uses: Vec<PathBuf>,
    /// How many other modules import from this one.
    pub used_by: usize,
}

/// A compact overview of a project.
#[derive(Debug, Clone)]
pub struct ProjectMap {
    pub name: String,
    pub fingerprint_language: String,
    pub generated_at: String,
    pub head: Option<String>,
    pub total_files: usize,
    pub modules: Vec<ModuleSummary>,
    pub hot_files: Vec<(PathBuf, usize)>,
    /// Key documents for the card: root docs, then doc directories with counts.
    pub docs: Vec<String>,
}

/// Paths in a fingerprint relative to `root`. Older fingerprints stored some
/// absolute paths; this accepts both.
fn rel(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// Modules to group files by. Without any manifest below the root, the
/// top-level directories stand in for modules.
fn effective_modules(fp: &Fingerprint) -> Vec<ModuleInfo> {
    if !fp.modules.is_empty() {
        return fp.modules.clone();
    }
    let dirs: BTreeSet<PathBuf> = fp
        .files
        .iter()
        .filter(|f| f.components().count() > 1)
        .filter_map(|f| f.components().next())
        .map(|c| PathBuf::from(c.as_os_str()))
        .collect();
    dirs.into_iter()
        .map(|path| ModuleInfo {
            name: path.display().to_string(),
            path,
            manifest: String::new(),
        })
        .collect()
}

/// The module a file belongs to: the deepest module containing it, or the
/// root (`""`) when none does.
fn module_of<'a>(modules: &'a [ModuleInfo], file: &Path) -> Option<&'a ModuleInfo> {
    modules
        .iter()
        .filter(|m| file.starts_with(&m.path))
        .max_by_key(|m| m.path.components().count())
}

/// Change count of every file the fingerprint knows about.
fn change_counts(root: &Path, fp: &Fingerprint) -> HashMap<PathBuf, usize> {
    fp.hot_zones
        .iter()
        .chain(&fp.cold_zones)
        .chain(&fp.frozen_zones)
        .map(|z| (rel(root, &z.path), z.change_count))
        .collect()
}

/// Build the map of a scanned project.
pub fn build(root: &Path, name: &str, fp: &Fingerprint) -> ProjectMap {
    let modules = effective_modules(fp);
    let changes = change_counts(root, fp);
    // Activity rankings list code only: manifests, lockfiles, docs and CI
    // config change often but are not where an agent goes to read code.
    let code: std::collections::HashSet<&PathBuf> = fp.files.iter().collect();
    let key = |file: &Path| module_of(&modules, file).map(|m| m.path.clone());

    let mut files_per: HashMap<Option<PathBuf>, usize> = HashMap::new();
    for file in &fp.files {
        *files_per.entry(key(file)).or_default() += 1;
    }

    // Module → modules it imports from.
    let mut uses: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for flow in &fp.data_flows {
        let Some(target) = &flow.resolved else {
            continue;
        };
        let (Some(from), Some(to)) = (key(&rel(root, &flow.from)), key(target)) else {
            continue;
        };
        if from != to {
            uses.entry(from).or_default().insert(to);
        }
    }
    let mut used_by: HashMap<PathBuf, usize> = HashMap::new();
    for targets in uses.values() {
        for t in targets {
            *used_by.entry(t.clone()).or_default() += 1;
        }
    }

    let entries: Vec<PathBuf> = fp.entry_points.iter().map(|e| rel(root, e)).collect();
    let mut summaries: Vec<ModuleSummary> = modules
        .iter()
        .map(|m| {
            let mut hot: Vec<(PathBuf, usize)> = changes
                .iter()
                .filter(|(f, c)| **c >= MIN_HOT_CHANGES && code.contains(*f))
                .filter(|(f, _)| module_of(&modules, f).is_some_and(|o| o.path == m.path))
                .map(|(f, c)| (rel(&m.path, f), *c))
                .collect();
            hot.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            hot.truncate(HOT_PER_MODULE);

            // A module's front door sits at its root or in `src/`. A deeper
            // match (`lib/auth/server.ts`) is just a file with a common name,
            // so none is shown rather than a misleading one.
            let entry = entries
                .iter()
                .filter(|e| module_of(&modules, e).is_some_and(|o| o.path == m.path))
                .map(|e| rel(&m.path, e))
                .filter(|e| {
                    let depth = e.components().count();
                    depth == 1 || (depth == 2 && e.starts_with("src"))
                })
                .min_by_key(|e| (e.components().count(), e.as_os_str().len()));

            ModuleSummary {
                path: m.path.clone(),
                name: m.name.clone(),
                files: files_per.get(&Some(m.path.clone())).copied().unwrap_or(0),
                entry,
                hot,
                uses: uses
                    .get(&m.path)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default(),
                used_by: used_by.get(&m.path).copied().unwrap_or(0),
            }
        })
        .filter(|m| m.files > 0)
        .collect();
    summaries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut hot_files: Vec<(PathBuf, usize)> = fp
        .hot_zones
        .iter()
        .map(|z| (rel(root, &z.path), z.change_count))
        .filter(|(f, _)| code.contains(f))
        .collect();
    hot_files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    hot_files.truncate(HOT_OVERALL);

    ProjectMap {
        name: name.to_string(),
        fingerprint_language: fp.language.to_string(),
        generated_at: fp.generated_at.format("%Y-%m-%d %H:%M UTC").to_string(),
        head: fp.head.as_ref().map(|h| h.chars().take(7).collect()),
        total_files: fp.files.len(),
        modules: summaries,
        hot_files,
        docs: key_docs(&fp.docs),
    }
}

fn join_counts(items: &[(PathBuf, usize)]) -> String {
    items
        .iter()
        .map(|(p, c)| format!("{} ({c})", p.display()))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Render the map as compact text for an agent.
pub fn render(map: &ProjectMap) -> String {
    let mut out = format!(
        "{} · {} · {} code files · {} modules · scanned {}{}\n",
        map.name,
        map.fingerprint_language,
        map.total_files,
        map.modules.len(),
        map.generated_at,
        map.head
            .as_ref()
            .map(|h| format!(" at {h}"))
            .unwrap_or_default(),
    );

    for m in &map.modules {
        out.push('\n');
        let path = m.path.display().to_string();
        let mut line = path.clone();
        if !m.name.is_empty() && m.name != path && !path.ends_with(&m.name) {
            line.push_str(&format!(" ({})", m.name));
        }
        line.push_str(&format!(" · {} files", m.files));
        if let Some(entry) = &m.entry {
            line.push_str(&format!(" · entry {}", entry.display()));
        }
        if m.used_by > 0 {
            line.push_str(&format!(" · used by {}", m.used_by));
        }
        out.push_str(&line);
        out.push('\n');
        if !m.hot.is_empty() {
            out.push_str(&format!("  hot: {}\n", join_counts(&m.hot)));
        }
        if !m.uses.is_empty() {
            let uses: Vec<String> = m.uses.iter().map(|u| u.display().to_string()).collect();
            out.push_str(&format!("  uses: {}\n", uses.join(", ")));
        }
    }

    if !map.hot_files.is_empty() {
        out.push_str(&format!(
            "\nMost changed: {}\n",
            join_counts(&map.hot_files)
        ));
    }
    out.push_str(
        "\nNext: related_files(file) for a file's imports and importers; \
         search_code(query) to find files by path, module or tag.\n",
    );
    out
}

/// A few-line summary of the project for an agent's system prompt.
///
/// It is meant to sit in the prompt for a whole session, so it stays small
/// (about 250 tokens on a 600-file monorepo) and identical between turns,
/// which keeps it in the provider's prompt cache. Details stay one tool call
/// away.
pub fn render_card(map: &ProjectMap) -> String {
    let modules: Vec<String> = map
        .modules
        .iter()
        .map(|m| format!("{} ({})", m.path.display(), m.files))
        .collect();
    let busiest: Vec<String> = map
        .hot_files
        .iter()
        .take(5)
        .map(|(p, _)| p.display().to_string())
        .collect();
    let mut out = format!(
        "{} · {} · {} code files{}\n",
        map.name,
        map.fingerprint_language,
        map.total_files,
        map.head
            .as_ref()
            .map(|h| format!(" · indexed at {h}"))
            .unwrap_or_default(),
    );
    if !modules.is_empty() {
        out.push_str(&format!("Modules: {}\n", modules.join(", ")));
    }
    if !busiest.is_empty() {
        out.push_str(&format!("Busiest files: {}\n", busiest.join(", ")));
    }
    if !map.docs.is_empty() {
        out.push_str(&format!("Docs: {}\n", map.docs.join(", ")));
    }
    out.push_str(
        "This card is the project overview: do not call project_map unless you need one \
         module's details. To find code or documentation, call search_code (symbols and doc \
         sections with file:line, files, modules). Before reading any file, code or Markdown, \
         call file_outline and read only the line range you need. related_files shows imports \
         and importers. Do not list directories, grep for file names or read whole files to \
         find your way.\n",
    );
    out
}

/// The documentation worth naming in the card: root-level documents first,
/// then each top-level directory that holds documents, with its count.
fn key_docs(docs: &[PathBuf]) -> Vec<String> {
    const ROOT_LIMIT: usize = 8;
    const DIR_LIMIT: usize = 6;
    let mut root: Vec<String> = docs
        .iter()
        .filter(|d| d.components().count() == 1)
        .map(|d| d.display().to_string())
        .collect();
    // README first: it is where "what is this" is answered.
    root.sort_by_key(|d| (!d.to_lowercase().starts_with("readme"), d.clone()));
    root.truncate(ROOT_LIMIT);

    let mut dirs: BTreeMap<String, usize> = BTreeMap::new();
    for d in docs.iter().filter(|d| d.components().count() > 1) {
        if let Some(first) = d.components().next() {
            *dirs
                .entry(first.as_os_str().to_string_lossy().into_owned())
                .or_default() += 1;
        }
    }
    // A directory with one or two documents is not where documentation lives.
    let mut dirs: Vec<(String, usize)> = dirs.into_iter().filter(|(_, n)| *n >= 3).collect();
    dirs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    root.extend(
        dirs.into_iter()
            .take(DIR_LIMIT)
            .map(|(dir, n)| format!("{dir}/ ({n})")),
    );
    root
}

/// Render ranked search hits and modules as agent text.
///
/// Symbols are rendered as `path:line`. File paths and module names are rendered
/// by name. `modules` are used only for the module section (may be empty).
/// `files_first` should be `true` when the first file hit was named exactly like
/// the query (so files lead the output).
///
/// Modules are filtered to only those where any query token (split on whitespace)
/// appears in the module's name or path, case-insensitively. When the query is
/// empty or has no tokens, no modules are included.
pub fn render_search(
    hits: &[crate::index::SearchHit],
    modules: &[crate::types::ModuleInfo],
    query: &str,
    files_first: bool,
) -> String {
    let symbols: Vec<String> = hits
        .iter()
        .filter(|h| h.is_symbol)
        .map(|h| format!("{} {} {}:{}", h.kind, h.name, h.path.display(), h.line))
        .collect();
    let files: Vec<String> = hits
        .iter()
        .filter(|h| !h.is_symbol)
        .map(|h| format!("{}", h.path.display()))
        .collect();

    // Filter modules by query tokens: include only if any token appears in name or path.
    let query_tokens: Vec<&str> = query.split_whitespace().collect();
    let module_names: Vec<String> = modules
        .iter()
        .filter(|m| {
            if query_tokens.is_empty() {
                return false;
            }
            let path_str = m.path.to_string_lossy().to_lowercase();
            let name_str = m.name.to_lowercase();
            query_tokens.iter().any(|tok| {
                path_str.contains(&tok.to_lowercase()) || name_str.contains(&tok.to_lowercase())
            })
        })
        .map(|m| format!("{} ({})", m.path.display(), m.name))
        .collect();

    if symbols.is_empty() && files.is_empty() && module_names.is_empty() {
        return format!("Nothing matches \"{query}\".\n");
    }

    let mut out = String::new();
    let section = |title: &str, items: &[String], out: &mut String| {
        if !items.is_empty() {
            out.push_str(&format!("{title}:\n  {}\n", items.join("\n  ")));
        }
    };
    if files_first {
        section("Files", &files, &mut out);
    }
    section("Symbols", &symbols, &mut out);
    section("Modules", &module_names, &mut out);
    if !files_first {
        section("Files", &files, &mut out);
    }
    out
}

/// Find the file `query` names: an exact project path, or a unique suffix
/// such as `auth/server.ts`.
pub fn find_file(root: &Path, fp: &Fingerprint, query: &str) -> Result<PathBuf, String> {
    let wanted = rel(root, Path::new(query.trim()));
    let known = || fp.files.iter().chain(&fp.docs);
    if known().any(|f| *f == wanted) {
        return Ok(wanted);
    }
    let matches: Vec<&PathBuf> = known().filter(|f| f.ends_with(&wanted)).collect();
    match matches.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "{} is not a code or doc file of this project (try search_code)",
            wanted.display()
        )),
        many => Err(format!(
            "{} matches {} files, be more specific: {}",
            wanted.display(),
            many.len(),
            many.iter()
                .take(8)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Render a file's neighbourhood: what it imports and what imports it.
///
/// `imports`, `importers`, and `external` are provided by the caller (typically
/// from the index). `fp` is used only for the optional header line (module name,
/// change count). Pass `None` for `fp` to omit the header.
pub fn render_related(
    root: &Path,
    fp: Option<&Fingerprint>,
    file: &Path,
    imports: &[String],
    importers: &[String],
    external: &[String],
) -> String {
    let mut out = file.display().to_string();

    if let Some(fp) = fp {
        let modules = effective_modules(fp);
        let changes = change_counts(root, fp);
        if let Some(m) = module_of(&modules, file) {
            out.push_str(&format!(" · module {}", m.path.display()));
        }
        if let Some(c) = changes.get(file) {
            out.push_str(&format!(" · {c} changes"));
        }
    }
    out.push('\n');

    let list = |title: &str, items: &[String], out: &mut String| {
        if items.is_empty() {
            return;
        }
        out.push_str(&format!("\n{title} ({}):\n", items.len()));
        for item in items.iter().take(LIST_LIMIT) {
            out.push_str(&format!("  {item}\n"));
        }
        if items.len() > LIST_LIMIT {
            out.push_str(&format!("  … {} more\n", items.len() - LIST_LIMIT));
        }
    };
    list("Imports", imports, &mut out);
    list("Imported by", importers, &mut out);
    list("External", external, &mut out);
    out
}

/// Render a file's outline from its declarations.
///
/// `declarations` come from the index or from disk parsing. `fp` is used only
/// for the optional header line (module, change count, importer count). Pass
/// `None` for `fp` to omit those fields. `importer_count` is the number of
/// files that import this one (from the index's importers query).
pub fn render_outline(
    root: &Path,
    fp: Option<&Fingerprint>,
    file: &Path,
    declarations: &[outline::Declaration],
    importer_count: usize,
) -> String {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    let total_lines = if let Some(fp) = fp {
        let changes = change_counts(root, fp);
        let modules = effective_modules(fp);
        let mut out = file.display().to_string();
        if let Some(m) = module_of(&modules, file) {
            out.push_str(&format!(" · module {}", m.path.display()));
        }
        if let Some(c) = changes.get(file) {
            out.push_str(&format!(" · {c} changes"));
        }
        if importer_count > 0 {
            out.push_str(&format!(" · imported by {importer_count}"));
        }
        out.push('\n');
        out
    } else {
        format!("{}\n", file.display())
    };

    let mut out = total_lines;

    if declarations.is_empty() {
        out.push_str("No declarations recognised; read the file directly.\n");
        return out;
    }

    if outline::is_doc(ext) {
        for d in declarations {
            let level = d.kind.trim_start_matches('h').parse::<usize>().unwrap_or(1);
            out.push_str(&format!(
                "  {}-{} {} {}\n",
                d.start,
                d.end,
                "#".repeat(level),
                d.name
            ));
        }
        out.push_str("Read only the section you need.\n");
        return out;
    }

    // Consecutive one-line declarations of the same kind collapse into one
    // line ("16-37 const ×10: A, B, …"): a block of constants or type aliases
    // costs one line instead of ten.
    let mut i = 0;
    while i < declarations.len() {
        let d = &declarations[i];
        let mut j = i;
        while j + 1 < declarations.len()
            && d.signature.is_none()
            && declarations[j].start == declarations[j].end
            && declarations[j + 1].start == declarations[j + 1].end
            && declarations[j + 1].kind == d.kind
            && declarations[j + 1].signature.is_none()
        {
            j += 1;
        }
        if j > i {
            let names: Vec<&str> = declarations[i..=j]
                .iter()
                .map(|d| d.name.as_str())
                .collect();
            out.push_str(&format!(
                "  {}-{} {} ×{}: {}\n",
                d.start,
                declarations[j].end,
                d.kind,
                names.len(),
                names.join(", ")
            ));
        } else {
            let range = if d.start == d.end {
                d.start.to_string()
            } else {
                format!("{}-{}", d.start, d.end)
            };
            match &d.signature {
                Some(sig) => out.push_str(&format!("  {range} {sig}\n")),
                None => out.push_str(&format!("  {range} {} {}\n", d.kind, d.name)),
            }
        }
        i = j + 1;
    }
    out.push_str("Read only the line range you need.\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DataFlow, FlowType, Language, Zone, ZoneFrequency};
    use chrono::Utc;

    fn zone(path: &str, changes: usize) -> Zone {
        Zone {
            path: PathBuf::from(path),
            frequency: ZoneFrequency::Hot,
            last_modified: Utc::now(),
            change_count: changes,
        }
    }

    fn fingerprint() -> Fingerprint {
        let files = [
            "apps/web/lib/auth/server.ts",
            "apps/web/app/page.tsx",
            "apps/web/tests/auth.test.ts",
            "packages/shared/src/index.ts",
        ];
        Fingerprint {
            project_id: uuid::Uuid::nil(),
            entry_points: vec![
                PathBuf::from("apps/web/lib/auth/server.ts"),
                PathBuf::from("packages/shared/src/index.ts"),
            ],
            data_flows: vec![DataFlow {
                from: PathBuf::from("apps/web/lib/auth/server.ts"),
                to: PathBuf::from("@m/shared"),
                flow_type: FlowType::Import,
                resolved: Some(PathBuf::from("packages/shared/src/index.ts")),
            }],
            hot_zones: vec![
                zone("apps/web/lib/auth/server.ts", 9),
                zone("apps/web/tests/auth.test.ts", 12),
                zone("apps/web/package.json", 30),
                zone("apps/web/app/page.tsx", 1),
            ],
            cold_zones: vec![],
            frozen_zones: vec![],
            language: Language::TypeScript,
            generated_at: Utc::now(),
            files: files.iter().map(PathBuf::from).collect(),
            modules: vec![
                ModuleInfo {
                    path: "apps/web".into(),
                    name: "@m/web".into(),
                    manifest: "package.json".into(),
                },
                ModuleInfo {
                    path: "packages/shared".into(),
                    name: "@m/shared".into(),
                    manifest: "package.json".into(),
                },
            ],
            head: None,
            symbols: vec![crate::types::Symbol {
                name: "requireSession".into(),
                kind: "function".into(),
                file: "apps/web/lib/auth/server.ts".into(),
                line: 10,
            }],
            docs: vec![PathBuf::from("README.md")],
            format: crate::fingerprint::FINGERPRINT_FORMAT,
        }
    }

    #[test]
    fn test_map_rules() {
        let map = build(Path::new(""), "mono", &fingerprint());
        let web = map
            .modules
            .iter()
            .find(|m| m.path == Path::new("apps/web"))
            .unwrap();
        // lib/auth/server.ts matched an entry-point pattern but sits deep in
        // the module: it is not presented as the module's entry.
        assert_eq!(web.entry, None);
        // Manifests are not hot code, and one change is not hot.
        let hot: Vec<_> = web
            .hot
            .iter()
            .map(|(p, _)| p.display().to_string())
            .collect();
        assert_eq!(hot, ["tests/auth.test.ts", "lib/auth/server.ts"]);
        assert_eq!(web.uses, [PathBuf::from("packages/shared")]);

        let shared = map
            .modules
            .iter()
            .find(|m| m.path == Path::new("packages/shared"))
            .unwrap();
        assert_eq!(shared.entry.as_deref(), Some(Path::new("src/index.ts")));
        assert_eq!(shared.used_by, 1);

        let text = render(&map);
        assert!(!text.contains("package.json"), "{text}");
    }

    #[test]
    fn test_find_file_by_unique_suffix() {
        let fp = fingerprint();
        assert_eq!(
            find_file(Path::new(""), &fp, "auth/server.ts").unwrap(),
            PathBuf::from("apps/web/lib/auth/server.ts")
        );
        assert!(find_file(Path::new(""), &fp, "missing.ts").is_err());
    }
}

#[cfg(test)]
mod doc_tests {
    use super::*;

    #[test]
    fn test_key_docs_lists_root_docs_then_big_doc_dirs() {
        let docs: Vec<PathBuf> = [
            "CHANGELOG.md",
            "README.md",
            "docs/a.md",
            "docs/b.md",
            "docs/c.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        // README leads; a directory with fewer than three documents is noise.
        assert_eq!(key_docs(&docs), ["README.md", "CHANGELOG.md", "docs/ (3)"]);
    }

    #[test]
    fn test_render_search_puts_a_named_file_first() {
        let hits = vec![
            crate::index::SearchHit {
                kind: "file".to_string(),
                is_symbol: false,
                name: "README.md".into(),
                path: PathBuf::from("README.md"),
                line: 0,
                score: 1.0,
                is_test: false,
            },
            crate::index::SearchHit {
                kind: "symbol".to_string(),
                is_symbol: true,
                name: "Write the README".into(),
                path: PathBuf::from("docs/plan.md"),
                line: 4,
                score: 0.5,
                is_test: false,
            },
        ];
        let text = render_search(&hits, &[], "README", true);
        assert!(text.starts_with("Files:\n  README.md"), "{text}");
    }

    #[test]
    fn test_render_search_symbols_render_as_path_line() {
        let hits = vec![crate::index::SearchHit {
            kind: "function".to_string(),
            is_symbol: true,
            name: "requireSession".into(),
            path: PathBuf::from("apps/web/lib/auth/server.ts"),
            line: 10,
            score: 3.0,
            is_test: false,
        }];
        let text = render_search(&hits, &[], "requireSession", false);
        assert!(text.contains("apps/web/lib/auth/server.ts:10"), "{text}");
    }

    #[test]
    fn test_render_search_modules_render_by_name() {
        let modules = vec![crate::types::ModuleInfo {
            path: PathBuf::from("apps/web"),
            name: "@m/web".into(),
            manifest: "package.json".into(),
        }];
        let text = render_search(&[], &modules, "web", false);
        assert!(text.contains("apps/web (@m/web)"), "{text}");
    }

    #[test]
    fn test_render_search_modules_filtered_by_query() {
        let modules = vec![
            crate::types::ModuleInfo {
                path: PathBuf::from("apps/web"),
                name: "@m/web".into(),
                manifest: "package.json".into(),
            },
            crate::types::ModuleInfo {
                path: PathBuf::from("packages/shared"),
                name: "@m/shared".into(),
                manifest: "package.json".into(),
            },
            crate::types::ModuleInfo {
                path: PathBuf::from("packages/api"),
                name: "@m/api".into(),
                manifest: "package.json".into(),
            },
        ];

        // Query matching no module renders no module section.
        let text = render_search(&[], &modules, "nonexistent", false);
        assert!(
            !text.contains("Modules:"),
            "should have no Modules section: {text}"
        );
        assert!(
            text.contains("Nothing matches"),
            "should say Nothing matches: {text}"
        );

        // Query matching one module name renders exactly that module.
        let text = render_search(&[], &modules, "web", false);
        assert!(
            text.contains("apps/web (@m/web)"),
            "should show web module: {text}"
        );
        assert!(
            !text.contains("packages/shared"),
            "should not show shared: {text}"
        );
        assert!(
            !text.contains("packages/api"),
            "should not show api: {text}"
        );

        // Query matching a path segment renders all modules with that segment.
        let text = render_search(&[], &modules, "packages", false);
        assert!(
            text.contains("packages/shared"),
            "should show shared: {text}"
        );
        assert!(text.contains("packages/api"), "should show api: {text}");
        assert!(!text.contains("apps/web"), "should not show web: {text}");

        // Empty query renders no modules.
        let text = render_search(&[], &modules, "", false);
        assert!(
            !text.contains("Modules:"),
            "empty query should have no Modules section: {text}"
        );

        // Query with only whitespace renders no modules.
        let text = render_search(&[], &modules, "   ", false);
        assert!(
            !text.contains("Modules:"),
            "whitespace-only query should have no Modules section: {text}"
        );
    }

    #[test]
    fn test_render_outline_doc_sections() {
        let decls = vec![
            outline::Declaration {
                kind: "h1".into(),
                name: "Overview".into(),
                start: 1,
                end: 5,
                signature: None,
            },
            outline::Declaration {
                kind: "h2".into(),
                name: "Usage".into(),
                start: 10,
                end: 20,
                signature: None,
            },
        ];
        let text = render_outline(
            Path::new("docs/guide.md"),
            None,
            Path::new("docs/guide.md"),
            &decls,
            0,
        );
        assert!(text.contains("# Overview"), "{text}");
        assert!(text.contains("## Usage"), "{text}");
    }

    #[test]
    fn test_render_outline_code_declarations() {
        let decls = vec![
            outline::Declaration {
                kind: "function".into(),
                name: "main".into(),
                start: 1,
                end: 10,
                signature: Some("fn main()".into()),
            },
            outline::Declaration {
                kind: "const".into(),
                name: "A".into(),
                start: 15,
                end: 15,
                signature: None,
            },
            outline::Declaration {
                kind: "const".into(),
                name: "B".into(),
                start: 16,
                end: 16,
                signature: None,
            },
        ];
        let text = render_outline(
            Path::new("src/main.rs"),
            None,
            Path::new("src/main.rs"),
            &decls,
            0,
        );
        assert!(text.contains("fn main()"), "{text}");
        assert!(text.contains("const ×2: A, B"), "{text}");
    }

    #[test]
    fn test_render_related_lists_imports_importers_externals() {
        let imports = vec!["packages/shared/src/index.ts".into()];
        let importers = vec!["apps/web/app/page.tsx".into()];
        let external = vec!["@m/shared".into()];
        let text = render_related(
            Path::new(""),
            None,
            Path::new("apps/web/lib/session.ts"),
            &imports,
            &importers,
            &external,
        );
        assert!(text.contains("packages/shared/src/index.ts"), "{text}");
        assert!(text.contains("apps/web/app/page.tsx"), "{text}");
        assert!(text.contains("@m/shared"), "{text}");
    }
}
