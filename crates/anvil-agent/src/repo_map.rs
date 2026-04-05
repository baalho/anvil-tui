//! Repo map — lightweight workspace index for auto-context.
//!
//! Scans the workspace for source files and extracts top-level symbols
//! using regex (no tree-sitter dependency). The map is injected into the
//! system prompt so the model knows what files and symbols exist without
//! being told.
//!
//! Inspired by Aider's repo map concept. Key differences:
//! - Regex-based extraction (no parser dependency)
//! - Budget-capped output (max ~2000 tokens)
//! - Prioritizes recently modified files
//! - Auto-includes files when the user mentions a symbol

use std::collections::HashMap;
use std::path::Path;
use std::time::SystemTime;

/// A single file entry in the repo map.
#[derive(Debug, Clone)]
pub struct RepoFile {
    /// Path relative to workspace root.
    pub rel_path: String,
    /// Top-level symbols extracted from the file.
    pub symbols: Vec<String>,
    /// Last modification time.
    pub modified: Option<SystemTime>,
}

/// The repo map — an index of workspace files and their symbols.
#[derive(Debug, Default)]
pub struct RepoMap {
    files: Vec<RepoFile>,
    /// Symbol → file path index for quick lookup.
    symbol_index: HashMap<String, Vec<String>>,
}

impl RepoMap {
    /// Create an empty repo map (no files scanned).
    ///
    /// Used by harness agents that don't need workspace awareness —
    /// the orchestrator injects relevant context directly.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Scan a workspace and build the repo map.
    ///
    /// Walks the directory tree, skipping hidden dirs, `node_modules`,
    /// `target`, `dist`, `build`, `.git`, and `vendor`. Extracts symbols
    /// from source files using regex patterns.
    pub fn scan(workspace: &Path) -> Self {
        let mut files = Vec::new();
        let mut symbol_index: HashMap<String, Vec<String>> = HashMap::new();

        walk_dir(workspace, workspace, &mut |rel_path, abs_path| {
            let ext = abs_path.extension().and_then(|e| e.to_str()).unwrap_or("");

            let symbols = extract_symbols(abs_path, ext);
            let modified = abs_path.metadata().ok().and_then(|m| m.modified().ok());

            let rel_str = rel_path.to_string_lossy().to_string();

            for sym in &symbols {
                symbol_index
                    .entry(sym.to_lowercase())
                    .or_default()
                    .push(rel_str.clone());
            }

            files.push(RepoFile {
                rel_path: rel_str,
                symbols,
                modified,
            });
        });

        // Sort by modification time (newest first) for priority
        files.sort_by(|a, b| {
            let a_time = a.modified.unwrap_or(SystemTime::UNIX_EPOCH);
            let b_time = b.modified.unwrap_or(SystemTime::UNIX_EPOCH);
            b_time.cmp(&a_time)
        });

        Self {
            files,
            symbol_index,
        }
    }

    /// Generate a summary string for the system prompt.
    ///
    /// Budget: aims for ~2000 tokens (~8000 chars). Shows file paths
    /// with their key symbols. Recently modified files get priority.
    pub fn summary(&self, max_chars: usize) -> String {
        if self.files.is_empty() {
            return String::new();
        }

        let mut out = String::with_capacity(max_chars);
        out.push_str("## Repo Map\n\n");
        out.push_str("Files and key symbols in this workspace:\n\n");

        for file in &self.files {
            let line = if file.symbols.is_empty() {
                format!("- {}\n", file.rel_path)
            } else {
                let syms = file.symbols.join(", ");
                format!("- {}: {}\n", file.rel_path, syms)
            };

            if out.len() + line.len() > max_chars {
                out.push_str(&format!(
                    "\n({} more files not shown)\n",
                    self.files.len()
                        - self
                            .files
                            .iter()
                            .filter(|f| out.contains(&f.rel_path))
                            .count()
                ));
                break;
            }
            out.push_str(&line);
        }

        out
    }

    /// Find files that contain a symbol matching the query.
    ///
    /// Used for auto-context: when the user mentions a function or type name,
    /// automatically include the relevant file's content in the prompt.
    pub fn find_files_for_query(&self, query: &str) -> Vec<&str> {
        let mut matches = Vec::new();

        // Check each word in the query against the symbol index
        for word in query.split_whitespace() {
            let lower = word.to_lowercase();
            // Strip common punctuation
            let clean = lower.trim_matches(|c: char| !c.is_alphanumeric() && c != '_');
            if clean.len() < 3 {
                continue;
            }

            if let Some(paths) = self.symbol_index.get(clean) {
                for path in paths {
                    if !matches.contains(&path.as_str()) {
                        matches.push(path.as_str());
                    }
                }
            }
        }

        // Also check for filename mentions
        for word in query.split_whitespace() {
            let lower = word.to_lowercase();
            let clean = lower.trim_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '.');
            for file in &self.files {
                let file_lower = file.rel_path.to_lowercase();
                let filename = file_lower.rsplit('/').next().unwrap_or(&file_lower);
                if filename == clean && !matches.contains(&file.rel_path.as_str()) {
                    matches.push(file.rel_path.as_str());
                }
            }
        }

        // Limit to 5 files to avoid context bloat
        matches.truncate(5);
        matches
    }

    /// Total number of indexed files.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total number of indexed symbols.
    pub fn symbol_count(&self) -> usize {
        self.symbol_index.len()
    }
}

// ── Directory walking ────────────────────────────────────────────────

/// Directories to skip during scanning.
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    ".anvil",
    ".idea",
    ".vscode",
];

/// File extensions to index.
const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "py", "js", "ts", "jsx", "tsx", "go", "java", "c", "cpp", "h", "hpp", "rb", "ex", "exs",
    "zig", "swift", "kt", "scala", "lua", "sh", "bash", "toml", "yaml", "yml", "json", "md",
];

fn walk_dir(root: &Path, dir: &Path, callback: &mut dyn FnMut(&Path, &Path)) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip hidden files/dirs
        if name_str.starts_with('.') && name_str != "." {
            continue;
        }

        if path.is_dir() {
            if SKIP_DIRS.contains(&name_str.as_ref()) {
                continue;
            }
            walk_dir(root, &path, callback);
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if SOURCE_EXTENSIONS.contains(&ext) {
                if let Ok(rel) = path.strip_prefix(root) {
                    callback(rel, &path);
                }
            }
        }
    }
}

// ── Symbol extraction ────────────────────────────────────────────────

/// Extract top-level symbols from a source file using regex patterns.
///
/// This is intentionally simple — no parser, no AST. We extract function,
/// struct, class, and type definitions that appear at the start of a line.
/// False positives are acceptable; false negatives are not.
fn extract_symbols(path: &Path, ext: &str) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Limit to first 500 lines to avoid scanning huge files
    let lines: Vec<&str> = content.lines().take(500).collect();
    let mut symbols = Vec::new();

    for line in &lines {
        let trimmed = line.trim();
        if let Some(sym) = extract_symbol_from_line(trimmed, ext) {
            if !symbols.contains(&sym) {
                symbols.push(sym);
            }
        }
    }

    // Limit symbols per file to keep the map compact
    symbols.truncate(20);
    symbols
}

fn extract_symbol_from_line(line: &str, ext: &str) -> Option<String> {
    match ext {
        "rs" => {
            // pub fn name, fn name, pub struct Name, struct Name, enum Name, impl Name
            if let Some(rest) = line
                .strip_prefix("pub fn ")
                .or_else(|| line.strip_prefix("fn "))
                .or_else(|| line.strip_prefix("pub async fn "))
                .or_else(|| line.strip_prefix("async fn "))
            {
                return extract_ident(rest);
            }
            if let Some(rest) = line
                .strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("struct "))
                .or_else(|| line.strip_prefix("pub enum "))
                .or_else(|| line.strip_prefix("enum "))
                .or_else(|| line.strip_prefix("pub trait "))
                .or_else(|| line.strip_prefix("trait "))
                .or_else(|| line.strip_prefix("pub type "))
                .or_else(|| line.strip_prefix("type "))
            {
                return extract_ident(rest);
            }
            if let Some(rest) = line.strip_prefix("impl ") {
                // impl Foo or impl Foo for Bar
                return extract_ident(rest);
            }
            None
        }
        "py" => {
            if let Some(rest) = line.strip_prefix("def ") {
                return extract_ident(rest);
            }
            if let Some(rest) = line.strip_prefix("class ") {
                return extract_ident(rest);
            }
            if let Some(rest) = line.strip_prefix("async def ") {
                return extract_ident(rest);
            }
            None
        }
        "js" | "ts" | "jsx" | "tsx" => {
            if let Some(rest) = line
                .strip_prefix("function ")
                .or_else(|| line.strip_prefix("export function "))
                .or_else(|| line.strip_prefix("export default function "))
                .or_else(|| line.strip_prefix("async function "))
                .or_else(|| line.strip_prefix("export async function "))
            {
                return extract_ident(rest);
            }
            if let Some(rest) = line
                .strip_prefix("class ")
                .or_else(|| line.strip_prefix("export class "))
                .or_else(|| line.strip_prefix("export default class "))
            {
                return extract_ident(rest);
            }
            if let Some(rest) = line
                .strip_prefix("export interface ")
                .or_else(|| line.strip_prefix("interface "))
                .or_else(|| line.strip_prefix("export type "))
                .or_else(|| line.strip_prefix("type "))
            {
                return extract_ident(rest);
            }
            // const Name = ... (exported)
            if let Some(rest) = line
                .strip_prefix("export const ")
                .or_else(|| line.strip_prefix("export let "))
            {
                return extract_ident(rest);
            }
            None
        }
        "go" => {
            if let Some(rest) = line.strip_prefix("func ") {
                // func (r *Receiver) Name or func Name
                let rest = if rest.starts_with('(') {
                    // Method — skip receiver
                    rest.split(')').nth(1).unwrap_or("").trim()
                } else {
                    rest
                };
                return extract_ident(rest);
            }
            if let Some(rest) = line.strip_prefix("type ") {
                return extract_ident(rest);
            }
            None
        }
        "java" | "kt" | "scala" => {
            for prefix in &[
                "public class ",
                "class ",
                "public interface ",
                "interface ",
                "public enum ",
                "enum ",
                "public static ",
                "public ",
                "fun ",
                "data class ",
                "sealed class ",
                "object ",
            ] {
                if let Some(rest) = line.strip_prefix(prefix) {
                    return extract_ident(rest);
                }
            }
            None
        }
        "c" | "cpp" | "h" | "hpp" => {
            // Very basic: look for function-like patterns
            // Skip preprocessor, includes, comments
            if line.starts_with('#') || line.starts_with("//") || line.starts_with("/*") {
                return None;
            }
            // typedef struct Name
            if let Some(rest) = line.strip_prefix("typedef struct ") {
                return extract_ident(rest);
            }
            // struct Name {
            if let Some(rest) = line.strip_prefix("struct ") {
                return extract_ident(rest);
            }
            None
        }
        _ => None,
    }
}

/// Extract an identifier from the start of a string.
/// Stops at the first non-identifier character.
fn extract_ident(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let ident: String = s
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() || ident.len() < 2 {
        None
    } else {
        Some(ident)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn extract_rust_symbols() {
        assert_eq!(
            extract_symbol_from_line(
                "pub fn resolve_path(workspace: &Path) -> Result<PathBuf>",
                "rs"
            ),
            Some("resolve_path".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("struct Agent {", "rs"),
            Some("Agent".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("pub enum Mode {", "rs"),
            Some("Mode".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("impl Agent {", "rs"),
            Some("Agent".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("pub async fn turn(&mut self) -> Result<()>", "rs"),
            Some("turn".to_string())
        );
    }

    #[test]
    fn extract_python_symbols() {
        assert_eq!(
            extract_symbol_from_line("def process_data(items):", "py"),
            Some("process_data".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("class MyHandler:", "py"),
            Some("MyHandler".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("async def fetch_data():", "py"),
            Some("fetch_data".to_string())
        );
    }

    #[test]
    fn extract_js_ts_symbols() {
        assert_eq!(
            extract_symbol_from_line("export function createApp() {", "ts"),
            Some("createApp".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("export class Router {", "js"),
            Some("Router".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("export interface Config {", "ts"),
            Some("Config".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("export const DEFAULT_PORT = 8080;", "ts"),
            Some("DEFAULT_PORT".to_string())
        );
    }

    #[test]
    fn extract_go_symbols() {
        assert_eq!(
            extract_symbol_from_line("func NewServer(addr string) *Server {", "go"),
            Some("NewServer".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("func (s *Server) Start() error {", "go"),
            Some("Start".to_string())
        );
        assert_eq!(
            extract_symbol_from_line("type Config struct {", "go"),
            Some("Config".to_string())
        );
    }

    #[test]
    fn scan_workspace() {
        let dir = TempDir::new().unwrap();

        // Create a Rust file
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("main.rs"),
            "pub fn main() {\n    println!(\"hello\");\n}\n\nstruct Config {\n    port: u16,\n}\n",
        )
        .unwrap();

        // Create a Python file
        std::fs::write(
            dir.path().join("script.py"),
            "def process():\n    pass\n\nclass Handler:\n    pass\n",
        )
        .unwrap();

        // Create a file that should be skipped
        let nm = dir.path().join("node_modules");
        std::fs::create_dir_all(&nm).unwrap();
        std::fs::write(nm.join("junk.js"), "function junk() {}").unwrap();

        let map = RepoMap::scan(dir.path());
        assert_eq!(map.file_count(), 2); // main.rs + script.py, not junk.js

        let summary = map.summary(8000);
        assert!(summary.contains("main.rs"));
        assert!(summary.contains("script.py"));
        assert!(summary.contains("main"));
        assert!(summary.contains("Config"));
        assert!(summary.contains("process"));
        assert!(summary.contains("Handler"));
        assert!(!summary.contains("junk"));
    }

    #[test]
    fn find_files_for_query() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("agent.rs"),
            "pub struct Agent {\n}\n\npub fn turn() {}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("config.rs"), "pub struct Settings {\n}\n").unwrap();

        let map = RepoMap::scan(dir.path());

        // Symbol mention
        let files = map.find_files_for_query("how does the Agent work?");
        assert!(files.contains(&"agent.rs"));

        // Filename mention
        let files = map.find_files_for_query("look at config.rs");
        assert!(files.contains(&"config.rs"));

        // No match
        let files = map.find_files_for_query("hello world");
        assert!(files.is_empty());
    }
}
