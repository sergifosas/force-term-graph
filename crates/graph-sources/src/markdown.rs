//! Markdown vault source (Obsidian-style).
//!
//! - One node per `.md` file (key = path relative to the root, `/`-separated;
//!   label = file stem; category = top-level folder, or `note` at the root).
//! - Edges from `[[Note]]`, `[[Note|alias]]`, `[[Note#heading]]`,
//!   `[[folder/Note]]` and relative `[text](other.md)` links. Links inside
//!   code are ignored (the parser never reports them as links).
//! - Unresolvable links point at ghost nodes, like Obsidian's "unresolved".
//! - Optional `#tag` nodes (category `tag`).
//!
//! Resolution follows Obsidian: a bare name matches any note with that stem
//! (case-insensitive), preferring one in the linking note's folder; a name
//! with `/` is matched as a path from the vault root or the note's folder.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use graph_core::{EdgeKind, EdgeSpec, GraphData, NodeSpec};
use ignore::WalkBuilder;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

use crate::Source;

pub struct MarkdownSource {
    root: PathBuf,
    /// Create `#tag` nodes.
    pub tags: bool,
}

/// A link target as written in a note, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawLink {
    /// `[[target]]` — the part before `|` and `#`.
    Wiki(String),
    /// `[text](relative/path.md)`, percent-decoded, fragment removed.
    Path(String),
    /// `#tag` (without the `#`).
    Tag(String),
}

impl MarkdownSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            tags: true,
        }
    }

    /// Relative, `/`-separated paths of all notes (sorted, deterministic).
    fn note_paths(&self) -> Result<Vec<String>> {
        let mut out = Vec::new();
        // `ignore` skips hidden dirs (.obsidian, .git) and respects .gitignore.
        let walker = WalkBuilder::new(&self.root)
            .require_git(false)
            .sort_by_file_path(|a, b| a.cmp(b))
            .build();
        for entry in walker {
            let entry = entry?;
            let p = entry.path();
            let is_md = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if is_md && entry.file_type().is_some_and(|t| t.is_file()) {
                out.push(rel_key(&self.root, p));
            }
        }
        out.sort();
        Ok(out)
    }
}

impl Source for MarkdownSource {
    fn name(&self) -> &str {
        "md"
    }

    fn load(&self) -> Result<GraphData> {
        if !self.root.is_dir() {
            anyhow::bail!("{} is not a directory", self.root.display());
        }
        let notes = self.note_paths()?;
        let resolver = Resolver::new(&notes);
        let mut data = GraphData::default();
        let mut tags = BTreeSet::new();

        for key in &notes {
            data.nodes.push(NodeSpec {
                id: key.clone(),
                label: Some(stem(key).to_string()),
                category: Some(category(key)),
                meta: BTreeMap::new(),
            });
            let path = self.root.join(key);
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            for link in extract_links(&text) {
                let (target, kind) = match link {
                    RawLink::Wiki(t) => match resolver.resolve_name(key, &t) {
                        Some(found) => (found, EdgeKind::Link),
                        None if is_note_like(&t) => (t, EdgeKind::Link), // ghost
                        None => continue, // attachment (pdf, png…), not a note
                    },
                    RawLink::Path(p) => match resolver.resolve_path(key, &p) {
                        Some(found) => (found, EdgeKind::Link),
                        None => continue,
                    },
                    RawLink::Tag(t) if self.tags => {
                        let id = format!("#{t}");
                        tags.insert(id.clone());
                        (id, EdgeKind::Tag)
                    }
                    RawLink::Tag(_) => continue,
                };
                data.edges.push(EdgeSpec {
                    source: key.clone(),
                    target,
                    weight: 1.0,
                    kind,
                });
            }
        }
        for t in tags {
            data.nodes.push(NodeSpec {
                label: Some(t.clone()),
                id: t,
                category: Some("tag".into()),
                meta: BTreeMap::new(),
            });
        }
        Ok(data)
    }

    fn watch_paths(&self) -> Vec<PathBuf> {
        vec![self.root.clone()]
    }
}

/// Pulls link targets out of one Markdown document.
pub fn extract_links(text: &str) -> Vec<RawLink> {
    let body = strip_frontmatter(text);
    let mut out = Vec::new();
    let mut in_code = false;
    let opts = Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for ev in Parser::new_ext(body, opts) {
        match ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_) | CodeBlockKind::Indented)) => {
                in_code = true
            }
            Event::End(TagEnd::CodeBlock) => in_code = false,
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            }) => match link_type {
                LinkType::WikiLink { .. } => {
                    let target = dest_url.split('#').next().unwrap_or("").trim();
                    if !target.is_empty() {
                        out.push(RawLink::Wiki(target.to_string()));
                    }
                }
                LinkType::Inline
                | LinkType::Reference
                | LinkType::Collapsed
                | LinkType::Shortcut => {
                    if let Some(p) = local_md_path(&dest_url) {
                        out.push(RawLink::Path(p));
                    }
                }
                _ => {}
            },
            Event::Text(t) if !in_code => out.extend(find_tags(&t).map(RawLink::Tag)),
            _ => {}
        }
    }
    out
}

/// YAML frontmatter isn't Markdown; skip it so `#` lines in it aren't tags.
fn strip_frontmatter(text: &str) -> &str {
    let t = text.strip_prefix('\u{feff}').unwrap_or(text);
    if let Some(rest) = t
        .strip_prefix("---\n")
        .or_else(|| t.strip_prefix("---\r\n"))
    {
        for marker in ["\n---\n", "\n---\r\n", "\n...\n"] {
            if let Some(end) = rest.find(marker) {
                return &rest[end + marker.len()..];
            }
        }
    }
    t
}

/// `#tag` tokens: start of text or after whitespace/punctuation, at least one
/// non-digit, made of letters, digits, `_`, `-` and `/` (nested tags).
fn find_tags(text: &str) -> impl Iterator<Item = String> + '_ {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut tags = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (pos, c) = chars[i];
        let boundary = i == 0 || !(chars[i - 1].1.is_alphanumeric() || chars[i - 1].1 == '#');
        if c == '#' && boundary {
            let start = pos + 1;
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].1.is_alphanumeric() || matches!(chars[j].1, '_' | '-' | '/'))
            {
                j += 1;
            }
            let end = chars.get(j).map_or(text.len(), |c| c.0);
            let tag = text[start..end].trim_end_matches('/');
            if tag.chars().any(|c| !c.is_ascii_digit()) && !tag.is_empty() {
                tags.push(tag.to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    tags.into_iter()
}

/// A relative link to a local `.md` file, percent-decoded, without fragment.
fn local_md_path(url: &str) -> Option<String> {
    if url.contains("://") || url.starts_with("mailto:") || url.starts_with('#') {
        return None;
    }
    let path = url.split(['#', '?']).next()?;
    let decoded = percent_decode(path);
    decoded
        .to_ascii_lowercase()
        .ends_with(".md")
        .then_some(decoded)
}

fn percent_decode(s: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Bare names and names ending in `.md` are notes; anything with another
/// extension (`.png`, `.pdf`…) is an attachment.
fn is_note_like(target: &str) -> bool {
    match Path::new(target).extension().and_then(|e| e.to_str()) {
        None => true,
        Some(ext) if ext.eq_ignore_ascii_case("md") => true,
        // Real file extensions are short and alphanumeric with a letter;
        // "v1.2 release" or "Note v2.0" are just dotted names.
        Some(ext) => {
            !(ext.len() <= 5
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
                && ext.chars().any(|c| c.is_ascii_alphabetic()))
        }
    }
}

struct Resolver {
    /// lowercase stem → keys (sorted).
    by_stem: HashMap<String, Vec<String>>,
    /// lowercase key without `.md` → key.
    by_path: HashMap<String, String>,
}

impl Resolver {
    fn new(keys: &[String]) -> Self {
        let mut by_stem: HashMap<String, Vec<String>> = HashMap::new();
        let mut by_path = HashMap::new();
        for k in keys {
            by_stem
                .entry(stem(k).to_lowercase())
                .or_default()
                .push(k.clone());
            by_path.insert(strip_md(k).to_lowercase(), k.clone());
        }
        Self { by_stem, by_path }
    }

    /// Resolves a wikilink target written in note `from`.
    fn resolve_name(&self, from: &str, target: &str) -> Option<String> {
        let t = strip_md(target.trim().trim_start_matches("./")).to_lowercase();
        if t.contains('/') {
            return self.by_path.get(&t).cloned().or_else(|| {
                let rel = normalize(&format!("{}/{}", parent(from), t));
                self.by_path.get(&rel).cloned()
            });
        }
        let candidates = self.by_stem.get(&t)?;
        let dir = parent(from).to_lowercase();
        candidates
            .iter()
            .find(|c| parent(c).to_lowercase() == dir)
            .or_else(|| candidates.first())
            .cloned()
    }

    /// Resolves a relative Markdown link path written in note `from`.
    fn resolve_path(&self, from: &str, path: &str) -> Option<String> {
        let joined = if let Some(abs) = path.strip_prefix('/') {
            abs.to_string()
        } else {
            format!("{}/{}", parent(from), path)
        };
        let key = normalize(strip_md(&joined)).to_lowercase();
        self.by_path.get(&key).cloned()
    }
}

/// Collapses `.` and `..` segments and leading slashes.
fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn rel_key(root: &Path, p: &Path) -> String {
    let rel = p.strip_prefix(root).unwrap_or(p);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn strip_md(s: &str) -> &str {
    if s.len() >= 3 && s[s.len() - 3..].eq_ignore_ascii_case(".md") {
        &s[..s.len() - 3]
    } else {
        s
    }
}

fn stem(key: &str) -> &str {
    strip_md(key.rsplit('/').next().unwrap_or(key))
}

fn parent(key: &str) -> &str {
    key.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn category(key: &str) -> String {
    match key.split_once('/') {
        Some((top, _)) => top.to_string(),
        None => "note".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_wikilinks_paths_and_tags_but_not_code() {
        let md = "---\ntags: [#notatag]\n---\n# Title\n\
                  See [[Rust]], [[Graph View|the graph]] and [[Physics#Forces]].\n\
                  Also [other](sub/Other%20Note.md#x) and [web](https://x.y/a.md).\n\
                  Tagged #rust and #nested/tag, not a#b, not #123.\n\n\
                  ```\n[[InCode]] #nottag\n```\n\
                  Inline `[[AlsoCode]]`.\n";
        let links = extract_links(md);
        assert_eq!(
            links,
            vec![
                RawLink::Wiki("Rust".into()),
                RawLink::Wiki("Graph View".into()),
                RawLink::Wiki("Physics".into()),
                RawLink::Path("sub/Other Note.md".into()),
                RawLink::Tag("rust".into()),
                RawLink::Tag("nested/tag".into()),
            ]
        );
    }

    #[test]
    fn resolves_like_obsidian() {
        let keys: Vec<String> = ["Rust.md", "a/Rust.md", "a/b/Deep.md", "Index.md"]
            .map(String::from)
            .to_vec();
        let r = Resolver::new(&keys);
        // Same folder wins, then first match; case-insensitive.
        assert_eq!(
            r.resolve_name("a/x.md", "rust").as_deref(),
            Some("a/Rust.md")
        );
        assert_eq!(
            r.resolve_name("Index.md", "RUST").as_deref(),
            Some("Rust.md")
        );
        // Paths: from root, or relative to the note.
        assert_eq!(
            r.resolve_name("Index.md", "a/b/Deep").as_deref(),
            Some("a/b/Deep.md")
        );
        assert_eq!(
            r.resolve_name("a/x.md", "b/Deep.md").as_deref(),
            Some("a/b/Deep.md")
        );
        assert_eq!(r.resolve_name("Index.md", "Missing"), None);
        assert_eq!(
            r.resolve_path("a/b/Deep.md", "../../Index.md").as_deref(),
            Some("Index.md")
        );
    }

    #[test]
    fn attachments_are_not_notes() {
        assert!(is_note_like("Some Note"));
        assert!(is_note_like("v1.2 release"));
        assert!(is_note_like("Note v2.0"));
        assert!(is_note_like("Note.md"));
        assert_eq!(percent_decode("a%20b%zzc%C3%A9"), "a b%zzcé");
        assert!(!is_note_like("diagram.png"));
        assert!(!is_note_like("paper.pdf"));
    }
}
