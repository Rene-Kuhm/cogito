//! The project's layout: which sub-projects it has, and which file each
//! import points at.
//!
//! Both are computed once, at scan time, from the list of project files, so
//! answering "what does this file use" or "who uses it" later needs no file
//! reads at all.

use crate::types::{Language, ModuleInfo};
use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

/// Manifest files that make a directory a module, in priority order: when a
/// directory has several, the first one names it.
const MANIFESTS: &[&str] = &[
    "package.json",
    "Cargo.toml",
    "go.mod",
    "pyproject.toml",
    "setup.py",
    "composer.json",
    "Gemfile",
    "pom.xml",
    "build.gradle.kts",
    "build.gradle",
    "Package.swift",
];

/// Extensions tried, in order, for an import written without one.
const JS_EXTENSIONS: &[&str] = &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Every directory other than the root that holds a manifest.
///
/// `files` are all project files relative to `root`.
pub(crate) fn find_modules(root: &Path, files: &[PathBuf]) -> Vec<ModuleInfo> {
    let mut by_dir: HashMap<PathBuf, (usize, &str)> = HashMap::new();
    for file in files {
        let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(rank) = MANIFESTS.iter().position(|m| *m == name) else {
            continue;
        };
        let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
        if dir.as_os_str().is_empty() {
            continue; // the root is the project itself, not one of its modules
        }
        let slot = by_dir.entry(dir).or_insert((rank, MANIFESTS[rank]));
        if rank < slot.0 {
            *slot = (rank, MANIFESTS[rank]);
        }
    }

    let mut modules: Vec<ModuleInfo> = by_dir
        .into_iter()
        .map(|(dir, (_, manifest))| {
            let name =
                manifest_name(&root.join(&dir).join(manifest), manifest).unwrap_or_else(|| {
                    dir.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default()
                });
            ModuleInfo {
                path: dir,
                name,
                manifest: manifest.to_string(),
            }
        })
        .collect();
    modules.sort_by(|a, b| a.path.cmp(&b.path));
    modules
}

/// The package name a manifest declares, if it declares one.
fn manifest_name(path: &Path, manifest: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let name = match manifest {
        "package.json" | "composer.json" => serde_json::from_str::<serde_json::Value>(&text)
            .ok()?
            .get("name")?
            .as_str()?
            .to_string(),
        "go.mod" => text
            .lines()
            .find_map(|l| l.trim().strip_prefix("module "))?
            .trim()
            .to_string(),
        "Cargo.toml" | "pyproject.toml" => {
            // The first `name = "…"` after the [package] / [project] header.
            let header = if manifest == "Cargo.toml" {
                "[package]"
            } else {
                "[project]"
            };
            let section = &text[text.find(header)?..];
            let re = regex::Regex::new(r#"(?m)^\s*name\s*=\s*"([^"]+)""#).ok()?;
            re.captures(section)?.get(1)?.as_str().to_string()
        }
        _ => return None,
    };
    (!name.is_empty()).then_some(name)
}

/// Resolves the import strings found in source files to project files.
pub(crate) struct Resolver {
    files: HashSet<PathBuf>,
    /// Workspace package name → its directory, for `@scope/pkg` imports.
    packages: HashMap<String, PathBuf>,
    /// Go module path from go.mod, e.g. `example.com/app`.
    go_module: Option<String>,
}

impl Resolver {
    pub(crate) fn new(root: &Path, files: &[PathBuf], modules: &[ModuleInfo]) -> Self {
        let packages = modules
            .iter()
            .filter(|m| m.manifest == "package.json")
            .map(|m| (m.name.clone(), m.path.clone()))
            .collect();
        // Extract Go module path from the root-level go.mod if present.
        // `find_modules` skips root-level manifests, so we check the files list directly.
        let go_module = files
            .iter()
            .find(|f| f.as_os_str() == "go.mod")
            .map(|f| root.join(f))
            .and_then(|f| manifest_name(&f, "go.mod"));
        Self {
            files: files.iter().cloned().collect(),
            packages,
            go_module,
        }
    }

    /// The project file `spec`, imported from `from`, refers to.
    ///
    /// `from` is relative to the project root. `None` means an external
    /// package, or an import this resolver does not understand.
    pub(crate) fn resolve(&self, from: &Path, spec: &str, language: Language) -> Option<PathBuf> {
        match language {
            Language::TypeScript | Language::JavaScript => self.resolve_js(from, spec),
            Language::Rust => self.resolve_rust(from, spec),
            Language::Python => self.resolve_python(from, spec),
            Language::Go => self.resolve_go(from, spec),
            Language::Java => self.resolve_java(from, spec),
            _ => None,
        }
    }

    fn resolve_js(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        let dir = from.parent().unwrap_or(Path::new(""));

        if spec.starts_with("./") || spec.starts_with("../") || spec == "." || spec == ".." {
            return self.js_file(&normalize(&dir.join(spec))?);
        }

        // `@/x` and `~/x`: the common alias for the root of the package the
        // importing file lives in (Next.js, Vite, Nuxt templates).
        if let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) {
            let base = self.nearest_config_dir(dir)?;
            return self
                .js_file(&base.join(rest))
                .or_else(|| self.js_file(&base.join("src").join(rest)));
        }

        // A workspace package: `@scope/name`, `@scope/name/sub`, `name/sub`.
        let mut parts = spec.splitn(3, '/');
        let first = parts.next()?;
        let (name, sub) = if first.starts_with('@') {
            let second = parts.next()?;
            (format!("{first}/{second}"), parts.next())
        } else {
            let rest: Vec<&str> = parts.collect();
            (
                first.to_string(),
                (!rest.is_empty()).then(|| spec[first.len() + 1..].as_ref()),
            )
        };
        let package_dir = self.packages.get(&name)?;
        match sub {
            Some(sub) => self
                .js_file(&package_dir.join(sub))
                .or_else(|| self.js_file(&package_dir.join("src").join(sub))),
            None => self.package_entry(package_dir),
        }
    }

    /// The file a bare package import loads.
    fn package_entry(&self, dir: &Path) -> Option<PathBuf> {
        ["src/index", "index", "src/main", "main", "lib/index"]
            .iter()
            .find_map(|candidate| self.js_file(&dir.join(candidate)))
    }

    /// The closest directory at or above `dir` with a tsconfig or jsconfig.
    fn nearest_config_dir(&self, dir: &Path) -> Option<PathBuf> {
        let mut current = Some(dir);
        while let Some(d) = current {
            if ["tsconfig.json", "jsconfig.json"]
                .iter()
                .any(|c| self.files.contains(&d.join(c)))
            {
                return Some(d.to_path_buf());
            }
            current = d.parent();
        }
        None
    }

    /// The file a JS/TS import path points at, trying the extensions and
    /// index files a bundler or `tsc` would.
    fn js_file(&self, base: &Path) -> Option<PathBuf> {
        if self.files.contains(base) {
            return Some(base.to_path_buf());
        }
        // ESM TypeScript imports `./x.js` for a source file named `x.ts`.
        if let Some(ext) = base.extension().and_then(|e| e.to_str())
            && matches!(ext, "js" | "jsx" | "mjs" | "cjs")
        {
            let stem = base.with_extension("");
            for ts in ["ts", "tsx", "mts", "cts"] {
                let candidate = stem.with_extension(ts);
                if self.files.contains(&candidate) {
                    return Some(candidate);
                }
            }
        }
        for ext in JS_EXTENSIONS {
            let candidate = PathBuf::from(format!("{}.{ext}", base.display()));
            if self.files.contains(&candidate) {
                return Some(candidate);
            }
        }
        for ext in JS_EXTENSIONS {
            let candidate = base.join(format!("index.{ext}"));
            if self.files.contains(&candidate) {
                return Some(candidate);
            }
        }
        None
    }

    /// `crate::a::b`, `self::a`, `super::a` → the `.rs` file of the deepest
    /// module path that exists.
    fn resolve_rust(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        let mut segments: Vec<&str> = spec.split("::").filter(|s| !s.is_empty()).collect();
        let anchor = segments.first().copied()?;
        let base = match anchor {
            "crate" => self.crate_src(from)?,
            "self" => module_dir(from),
            "super" => module_dir(from).parent().map(Path::to_path_buf)?,
            _ => return None,
        };
        segments.remove(0);

        // `use crate::a::b::Item`: try a/b, then a — the tail may be an item.
        for len in (1..=segments.len()).rev() {
            let path: PathBuf = segments[..len].iter().collect();
            for candidate in [
                base.join(&path).with_extension("rs"),
                base.join(&path).join("mod.rs"),
            ] {
                if self.files.contains(&candidate) {
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// `src/` of the crate `from` belongs to.
    fn crate_src(&self, from: &Path) -> Option<PathBuf> {
        let mut current = from.parent();
        while let Some(dir) = current {
            if self.files.contains(&dir.join("Cargo.toml")) {
                return Some(dir.join("src"));
            }
            current = dir.parent();
        }
        None
    }

    /// Python resolution for:
    /// - `from pkg.auth import x` / `from pkg.auth import x as y`
    /// - `import pkg.auth`
    /// - relative: `from .auth import x`, `from ..pkg import y`
    ///
    /// Candidate files for a dotted path `a.b.c`:
    /// 1. `a/b/c.py`
    /// 2. `a/b/c/__init__.py`
    ///
    /// Tried first from the importing file's package directory, then from
    /// `src/`, then from the project root.
    fn resolve_python(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        // For relative imports, use the file's parent directory directly.
        // For absolute imports, use the project root.
        let base = if spec.starts_with('.') {
            // Relative import: leading dots indicate upward traversal from the
            // importing file's package. `.pkg` means current package's `pkg`,
            // `..pkg` means parent package's `pkg`, etc.
            let rest = spec.trim_start_matches('.');
            let dots = spec.len() - rest.len();
            let from_dir = from.parent().unwrap_or(Path::new(""));
            let mut base = from_dir.to_path_buf();
            // Always go up (dots - 1) levels. `.x` goes up 0, `..x` goes up 1.
            for _ in 0..dots.saturating_sub(1) {
                base = base.parent().map(Path::to_path_buf)?;
            }
            if rest.is_empty() {
                base // e.g., `from .. import x`
            } else {
                base.join(rest.replace('.', "/"))
            }
        } else {
            PathBuf::from(spec.replace('.', "/"))
        };

        // Candidate paths tried in order: a/b/c.py, a/b/c/__init__.py.
        for candidate in [base.with_extension("py"), base.join("__init__.py")] {
            if self.files.contains(&candidate) {
                return Some(candidate);
            }
        }

        // Try from `src/` if not already under it.
        if !base.starts_with("src") {
            for candidate in [
                PathBuf::from("src").join(&base).with_extension("py"),
                PathBuf::from("src").join(&base).join("__init__.py"),
            ] {
                if self.files.contains(&candidate) {
                    return Some(candidate);
                }
            }
        }

        None
    }

    /// Go resolution for:
    /// - `import "example.com/app/internal/auth"` → `internal/auth/` package
    /// - `./x` and `../x` relative imports
    ///
    /// Go imports address packages (directories). The resolved path is a `.go`
    /// file within that package:
    /// 1. `<dir>/<basename-of-dir>.go` (preferred, e.g. `auth/auth.go`)
    /// 2. the single `.go` file in that directory
    /// 3. lexicographically first `.go` file
    fn resolve_go(&self, from: &Path, spec: &str) -> Option<PathBuf> {
        let Some(module) = &self.go_module else {
            return None;
        };

        let dir = if spec == module {
            PathBuf::from(".")
        } else if let Some(rest) = spec.strip_prefix(module) {
            let rest = rest.trim_start_matches('/');
            if rest.is_empty() {
                PathBuf::from(".")
            } else {
                PathBuf::from(rest.replace('.', "/"))
            }
        } else if spec.starts_with("./") || spec.starts_with("../") {
            // Relative import: resolve against the file's directory.
            // Go relative imports are relative to the directory containing the
            // source file. `../lib` means the `lib` package in the parent
            // directory of the file's own directory.
            let from_dir = from.parent().unwrap_or(Path::new(""));
            // Count leading `..` or `.` to determine levels to go up.
            // `../` = 1 level up, `../../` = 2 levels up, etc.
            let mut levels = 0usize;
            let mut chars = spec.chars().peekable();
            while chars.peek() == Some(&'.') {
                chars.next();
                levels += 1;
            }
            // The path after leading dots (skip the `/` separator if present).
            let after_dots: String = chars.collect();
            let rest = after_dots.trim_start_matches('/');
            let mut dir = from_dir.to_path_buf();
            // Go up `levels` levels.
            for _ in 0..levels {
                let parent = dir.parent()?;
                dir = parent.to_path_buf();
            }
            if rest.is_empty() {
                dir
            } else {
                dir.join(rest.replace('.', "/"))
            }
        } else {
            // External package — not resolvable
            return None;
        };

        self.go_package_file(&dir)
    }

    /// Find the primary `.go` file for a Go package directory.
    fn go_package_file(&self, dir: &Path) -> Option<PathBuf> {
        // Prefer <dir>/<basename>.go
        if let Some(basename) = dir.file_name().and_then(|n| n.to_str()) {
            let preferred = dir.join(format!("{basename}.go"));
            if self.files.contains(&preferred) {
                return Some(preferred);
            }
        }

        // Find all .go files in the directory
        let mut go_files: Vec<PathBuf> = self
            .files
            .iter()
            .filter(|f| {
                f.parent() == Some(dir) && f.extension().map(|e| e == "go").unwrap_or(false)
            })
            .cloned()
            .collect();

        go_files.sort();
        go_files.into_iter().next()
    }

    /// Java resolution for:
    /// - `import com.acme.Auth;` → `com/acme/Auth.java`
    /// - `import com.acme.util.*;` → any `.java` file in `com/acme/util/`
    ///
    /// Source roots are discovered from the file set: directories ending in
    /// `src/main/java`, `src/test/java`, or a `src` path component, plus the
    /// project root (`.`).
    #[allow(clippy::collapsible_if)]
    fn resolve_java(&self, _from: &Path, spec: &str) -> Option<PathBuf> {
        // Discover source roots from known files.
        let mut source_roots: Vec<PathBuf> = vec![PathBuf::from(".")];
        for file in &self.files {
            let components: Vec<_> = file.components().collect();
            for i in 0..components.len() {
                let path = components[..=i].iter().collect::<PathBuf>();
                let path_str = path.display().to_string();
                // `ends_with("src")` on a string would also match `my_src`, so
                // the bare `src` root is matched on the last path component.
                let is_bare_src = path.file_name().and_then(|n| n.to_str()) == Some("src");
                if path_str.ends_with("src/main/java")
                    || path_str.ends_with("src/test/java")
                    || (is_bare_src && !source_roots.contains(&path))
                {
                    if !source_roots.contains(&path) {
                        source_roots.push(path);
                    }
                }
            }
        }

        // Convert import spec like `com.acme.Auth` to path `com/acme/Auth.java`
        let spec = spec.replace('.', "/");
        let java_file = format!("{spec}.java");

        for root in &source_roots {
            // First try: specific class file
            let candidate = root.join(&java_file);
            if self.files.contains(&candidate) {
                return Some(candidate);
            }
        }

        // Second try: for package-level imports, find any .java file in the package directory
        // `spec` is already converted from dots to slashes above.
        for root in &source_roots {
            let pkg_path = root.join(&spec);
            // Find all .java files in this package directory
            let mut java_files: Vec<PathBuf> = self
                .files
                .iter()
                .filter(|f| {
                    f.parent() == Some(&pkg_path)
                        && f.extension().map(|e| e == "java").unwrap_or(false)
                })
                .cloned()
                .collect();
            if !java_files.is_empty() {
                java_files.sort();
                return Some(java_files.into_iter().next().unwrap());
            }
        }

        None
    }
}

/// The directory holding a Rust file's child modules: `src/a/` for
/// `src/a.rs`, and the file's own directory for `mod.rs`, `lib.rs`, `main.rs`.
fn module_dir(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("")).to_path_buf();
    match file.file_stem().and_then(|s| s.to_str()) {
        Some("mod" | "lib" | "main") | None => dir,
        Some(stem) => dir.join(stem),
    }
}

/// Resolve `.` and `..` lexically. `None` if the path climbs above the root.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::Normal(part) => out.push(part),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resolver(files: &[&str], packages: &[(&str, &str)]) -> Resolver {
        Resolver {
            files: files.iter().map(PathBuf::from).collect(),
            packages: packages
                .iter()
                .map(|(n, d)| (n.to_string(), PathBuf::from(d)))
                .collect(),
            go_module: None,
        }
    }

    fn resolver_with_go(files: &[&str], packages: &[(&str, &str)], go_module: &str) -> Resolver {
        Resolver {
            files: files.iter().map(PathBuf::from).collect(),
            packages: packages
                .iter()
                .map(|(n, d)| (n.to_string(), PathBuf::from(d)))
                .collect(),
            go_module: Some(go_module.to_string()),
        }
    }

    fn js(r: &Resolver, from: &str, spec: &str) -> Option<String> {
        r.resolve(Path::new(from), spec, Language::TypeScript)
            .map(|p| p.display().to_string())
    }

    #[test]
    fn test_relative_imports_try_extensions_and_index_files() {
        let r = resolver(
            &["src/a.ts", "src/util/index.ts", "src/b.tsx", "src/c.ts"],
            &[],
        );
        assert_eq!(js(&r, "src/a.ts", "./b").as_deref(), Some("src/b.tsx"));
        assert_eq!(
            js(&r, "src/a.ts", "./util").as_deref(),
            Some("src/util/index.ts")
        );
        // ESM TypeScript: `./c.js` is the source file `c.ts`.
        assert_eq!(js(&r, "src/a.ts", "./c.js").as_deref(), Some("src/c.ts"));
        assert_eq!(js(&r, "src/a.ts", "./missing"), None);
        assert_eq!(js(&r, "src/a.ts", "../../outside"), None);
    }

    #[test]
    fn test_alias_resolves_from_the_nearest_tsconfig() {
        let r = resolver(
            &[
                "apps/web/tsconfig.json",
                "apps/web/lib/auth.ts",
                "apps/web/app/page.tsx",
            ],
            &[],
        );
        assert_eq!(
            js(&r, "apps/web/app/page.tsx", "@/lib/auth").as_deref(),
            Some("apps/web/lib/auth.ts")
        );
    }

    #[test]
    fn test_workspace_packages_resolve_to_their_entry_or_subpath() {
        let r = resolver(
            &[
                "packages/shared/src/index.ts",
                "packages/shared/src/types.ts",
            ],
            &[("@app/shared", "packages/shared")],
        );
        assert_eq!(
            js(&r, "apps/web/x.ts", "@app/shared").as_deref(),
            Some("packages/shared/src/index.ts")
        );
        assert_eq!(
            js(&r, "apps/web/x.ts", "@app/shared/types").as_deref(),
            Some("packages/shared/src/types.ts")
        );
        assert_eq!(js(&r, "apps/web/x.ts", "react"), None, "external package");
    }

    #[test]
    fn test_rust_paths_resolve_to_module_files() {
        let r = resolver(
            &[
                "core/Cargo.toml",
                "core/src/lib.rs",
                "core/src/storage/mod.rs",
                "core/src/storage/sqlite.rs",
                "core/src/types.rs",
            ],
            &[],
        );
        let rs = |from: &str, spec: &str| {
            r.resolve(Path::new(from), spec, Language::Rust)
                .map(|p| p.display().to_string())
        };
        assert_eq!(
            rs("core/src/lib.rs", "crate::storage::sqlite::open").as_deref(),
            Some("core/src/storage/sqlite.rs")
        );
        assert_eq!(
            rs("core/src/lib.rs", "crate::types::Project").as_deref(),
            Some("core/src/types.rs")
        );
        assert_eq!(
            rs("core/src/storage/mod.rs", "self::sqlite").as_deref(),
            Some("core/src/storage/sqlite.rs")
        );
        assert_eq!(
            rs("core/src/storage/sqlite.rs", "super::super::types").as_deref(),
            None,
            "super::super is not followed"
        );
    }

    #[test]
    fn test_python_imports_resolve_to_module_files() {
        let r = resolver(
            &[
                "pkg/__init__.py",
                "pkg/auth.py",
                "pkg/utils/__init__.py",
                "pkg/utils/helpers.py",
                "app/main.py",
            ],
            &[],
        );
        let py = |from: &str, spec: &str| {
            r.resolve(Path::new(from), spec, Language::Python)
                .map(|p| p.display().to_string())
        };
        // Absolute import from pkg.auth
        assert_eq!(
            py("app/main.py", "pkg.auth").as_deref(),
            Some("pkg/auth.py")
        );
        // from import
        assert_eq!(
            py("app/main.py", "pkg.utils.helpers").as_deref(),
            Some("pkg/utils/helpers.py")
        );
        // External package must not resolve
        assert_eq!(py("app/main.py", "os"), None, "stdlib is external");
    }

    #[test]
    fn test_python_relative_imports_resolve() {
        let r = resolver(
            &[
                "pkg/__init__.py",
                "pkg/a.py",
                "pkg/b.py",
                "pkg/sub/__init__.py",
                "pkg/sub/c.py",
            ],
            &[],
        );
        let py = |from: &str, spec: &str| {
            r.resolve(Path::new(from), spec, Language::Python)
                .map(|p| p.display().to_string())
        };
        // from .b import x
        assert_eq!(py("pkg/a.py", ".b").as_deref(), Some("pkg/b.py"));
        // from .sub.c import x
        assert_eq!(py("pkg/a.py", ".sub.c").as_deref(), Some("pkg/sub/c.py"));
        // from .. import x (from pkg/sub/c.py)
        assert_eq!(py("pkg/sub/c.py", "..").as_deref(), Some("pkg/__init__.py"));
    }

    #[test]
    fn test_go_imports_resolve_to_package_files() {
        let r = resolver_with_go(
            &[
                "go.mod",
                "internal/auth/auth.go",
                "internal/auth/types.go",
                "cmd/main.go",
                "lib/utils.go",
            ],
            &[],
            "example.com/app",
        );
        let go = |from: &str, spec: &str| {
            r.resolve(Path::new(from), spec, Language::Go)
                .map(|p| p.display().to_string())
        };
        // Absolute import under the module
        assert_eq!(
            go("cmd/main.go", "example.com/app/internal/auth").as_deref(),
            Some("internal/auth/auth.go")
        );
        // Relative import
        assert_eq!(
            go("internal/auth/types.go", "../lib").as_deref(),
            Some("lib/utils.go")
        );
        // External package must not resolve
        assert_eq!(go("cmd/main.go", "fmt"), None, "stdlib is external");
    }

    #[test]
    fn test_java_imports_resolve_to_source_files() {
        let r = resolver(
            &[
                "src/main/java/com/acme/Auth.java",
                "src/main/java/com/acme/App.java",
                "src/main/java/com/acme/util/Helper.java",
                "src/test/java/com/acme/AuthTest.java",
            ],
            &[],
        );
        let java = |from: &str, spec: &str| {
            r.resolve(Path::new(from), spec, Language::Java)
                .map(|p| p.display().to_string())
        };
        assert_eq!(
            java("src/main/java/com/acme/App.java", "com.acme.Auth").as_deref(),
            Some("src/main/java/com/acme/Auth.java")
        );
        assert_eq!(
            java("src/main/java/com/acme/App.java", "com.acme.util.Helper").as_deref(),
            Some("src/main/java/com/acme/util/Helper.java")
        );
        // External package must not resolve
        assert_eq!(
            java("src/main/java/com/acme/App.java", "java.util.List"),
            None,
            "stdlib is external"
        );
    }
}
