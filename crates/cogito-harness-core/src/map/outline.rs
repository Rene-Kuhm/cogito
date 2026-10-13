//! Declarations in a source file, with the lines each one spans.
//!
//! This is what lets an agent read a file in pieces: it sees one line per
//! declaration, then reads only the range it needs instead of the whole file.
//! Detection is line-based on purpose. It runs on every file at scan time and
//! must never fail, so it trades precision on exotic syntax for speed and
//! robustness: a declaration starts on a line at (or near) column 0 that
//! begins with a declaring keyword.

use regex::Regex;
use std::sync::OnceLock;

/// One declaration in a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub name: String,
    /// `function`, `class`, `interface`, `type`, `const`, `struct`, `impl`…
    pub kind: String,
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    /// The declaring line, trimmed and shortened. Only kept for callables,
    /// where parameters are the useful part; elsewhere the name says enough.
    pub signature: Option<String>,
}

/// Longest signature kept, in characters.
const SIGNATURE_MAX: usize = 110;

struct Grammar {
    re: Regex,
    /// Capture group with the kind keyword, and with the name.
    kind: usize,
    name: usize,
}

fn grammar(extension: &str) -> Option<&'static Grammar> {
    static TS: OnceLock<Grammar> = OnceLock::new();
    static RUST: OnceLock<Grammar> = OnceLock::new();
    static PY: OnceLock<Grammar> = OnceLock::new();
    static GO: OnceLock<Grammar> = OnceLock::new();
    static OTHER: OnceLock<Grammar> = OnceLock::new();

    let build = |pattern: &str, kind, name| Grammar {
        re: Regex::new(pattern).expect("valid declaration pattern"),
        kind,
        name,
    };
    Some(match extension {
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => TS.get_or_init(|| {
            build(
                r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|class|interface|type|enum|const|let|var|namespace)\s+([A-Za-z_$][\w$]*)",
                1,
                2,
            )
        }),
        "rs" => RUST.get_or_init(|| {
            build(
                r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?(fn|struct|enum|trait|type|const|static|mod|union|impl|macro_rules!)\s*(?:<[^>]*>\s*)?([A-Za-z_][\w:]*(?:\s+for\s+[A-Za-z_][\w:<>, ]*)?)",
                1,
                2,
            )
        }),
        "py" => PY.get_or_init(|| build(r"^(?: {4})?(?:async\s+)?(def|class)\s+([A-Za-z_]\w*)", 1, 2)),
        "go" => GO.get_or_init(|| {
            build(r"^(func|type)\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)", 1, 2)
        }),
        "java" | "kt" | "kts" | "cs" | "swift" | "php" | "rb" | "scala" => OTHER.get_or_init(|| {
            build(
                r"^\s{0,4}(?:(?:public|private|protected|internal|export|open|final|abstract|static|sealed|data|override|async|partial|readonly)\s+)*(class|interface|enum|struct|record|object|trait|fun|func|def|function|module|protocol|extension)\s+([A-Za-z_][\w]*)",
                1,
                2,
            )
        }),
        _ => return None,
    })
}

fn is_callable(kind: &str) -> bool {
    matches!(
        kind,
        "function" | "function*" | "fn" | "def" | "func" | "fun"
    )
}

/// Declarations in `content`, a file with the given extension. Empty for
/// languages Cogito does not outline.
pub fn declarations(content: &str, extension: &str) -> Vec<Declaration> {
    let Some(grammar) = grammar(&extension.to_lowercase()) else {
        return Vec::new();
    };
    let lines: Vec<&str> = content.lines().collect();

    let mut found: Vec<Declaration> = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        let Some(caps) = grammar.re.captures(line) else {
            continue;
        };
        let (Some(kind), Some(name)) = (caps.get(grammar.kind), caps.get(grammar.name)) else {
            continue;
        };
        let kind = kind.as_str().trim_end_matches('!').to_string();
        let name = name.as_str().trim().to_string();
        let signature = is_callable(&kind).then(|| shorten(line.trim()));
        found.push(Declaration {
            name,
            kind,
            start: index + 1,
            end: index + 1,
            signature,
        });
    }

    // A declaration runs until the next one starts, minus trailing blank lines
    // and the comments that belong to the next declaration.
    let total = lines.len();
    for i in 0..found.len() {
        let next_start = found.get(i + 1).map_or(total + 1, |d| d.start);
        let mut end = next_start - 1;
        while end > found[i].start {
            let text = lines[end - 1].trim();
            if text.is_empty()
                || text.starts_with("//")
                || text.starts_with("/*")
                || text.starts_with('*')
                || text.starts_with('#') && !text.starts_with("#[")
                || text.starts_with("#[")
                || text.starts_with('@')
            {
                end -= 1;
            } else {
                break;
            }
        }
        found[i].end = end;
    }
    found
}

fn shorten(line: &str) -> String {
    let line = line.trim_end_matches('{').trim_end();
    if line.chars().count() <= SIGNATURE_MAX {
        return line.to_string();
    }
    let cut: String = line.chars().take(SIGNATURE_MAX).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_typescript_declarations_and_ranges() {
        let src = "\
import { z } from 'zod';

export const SCHEMA = 'v1';
export type Code = 'ok' | 'stale';

/** A finding. */
export interface Finding {
  code: Code;
}

export async function runPipeline(input: string, opts?: Options): Promise<void> {
  await work();
}
";
        let decls = declarations(src, "ts");
        let summary: Vec<_> = decls
            .iter()
            .map(|d| (d.kind.as_str(), d.name.as_str(), d.start, d.end))
            .collect();
        assert_eq!(
            summary,
            [
                ("const", "SCHEMA", 3, 3),
                ("type", "Code", 4, 4),
                // The doc comment above runPipeline is not part of Finding.
                ("interface", "Finding", 7, 9),
                ("function", "runPipeline", 11, 13),
            ]
        );
        assert_eq!(
            decls[3].signature.as_deref(),
            Some("export async function runPipeline(input: string, opts?: Options): Promise<void>")
        );
        assert_eq!(decls[0].signature, None, "names say enough for constants");
    }

    #[test]
    fn test_rust_declarations() {
        let src = "\
use std::path::Path;

/// Doc.
#[derive(Debug)]
pub struct Map {
    files: usize,
}

impl Map {
    pub fn new() -> Self { Self { files: 0 } }
}

impl Display for Map {}

pub(crate) async fn build(root: &Path) -> Map {
    todo!()
}
";
        let decls: Vec<_> = declarations(src, "rs")
            .into_iter()
            .map(|d| (d.kind, d.name, d.start, d.end))
            .collect();
        assert_eq!(
            decls,
            [
                ("struct".into(), "Map".into(), 5, 7),
                ("impl".into(), "Map".into(), 9, 11),
                ("impl".into(), "Display for Map".into(), 13, 13),
                ("fn".into(), "build".into(), 15, 17),
            ]
        );
    }

    #[test]
    fn test_python_and_go() {
        let py = "class Repo:\n    def get(self, id):\n        pass\n\ndef main():\n    pass\n";
        let names: Vec<_> = declarations(py, "py").into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Repo", "get", "main"]);

        let go = "package x\n\ntype Server struct{}\n\nfunc (s *Server) Start() error {\n\treturn nil\n}\n";
        let names: Vec<_> = declarations(go, "go").into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Server", "Start"]);
    }

    #[test]
    fn test_unknown_extension_is_empty() {
        assert!(declarations("whatever", "md").is_empty());
    }
}
