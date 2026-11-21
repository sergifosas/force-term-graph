use graph_core::EdgeKind;
use graph_sources::{MarkdownSource, Source, load_all};

fn vault() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/vault")
}

#[test]
fn vault_produces_expected_notes_ghosts_tags_and_edges() {
    let sources: Vec<Box<dyn Source>> = vec![Box::new(MarkdownSource::new(vault()))];
    let g = load_all(&sources).unwrap();

    let id = |k: &str| g.id_of(k).unwrap_or_else(|| panic!("missing node {k}"));
    let linked = |a: &str, b: &str| g.neighbors(id(a)).contains(&id(b));

    // 4 notes (hidden .obsidian skipped) + 1 ghost + 2 tags.
    assert_eq!(
        g.node_count(),
        7,
        "{:#?}",
        g.nodes().iter().map(|n| &n.key).collect::<Vec<_>>()
    );
    assert!(g.id_of("md:.obsidian/hidden.md").is_none());

    let index = "md:Index.md";
    let rust = "md:Rust.md";
    let fg = "md:projects/Force Graph.md";
    let gv = "md:Graph View.md";
    assert_eq!(g.node(id(fg)).label, "Force Graph");
    assert_eq!(g.node(id(fg)).category, "projects");
    assert_eq!(g.node(id(rust)).category, "note");

    assert!(linked(index, rust));
    assert!(linked(index, gv), "aliased link");
    assert!(linked(rust, fg), "path link");
    assert!(linked(fg, gv), "heading link");
    assert!(linked(fg, index), "relative markdown link");
    assert!(g.node(id("md:Missing Note")).ghost);
    assert!(linked(index, "md:Missing Note"));
    assert!(linked(index, "md:#project") && linked(gv, "md:#project"));
    assert!(linked(rust, "md:#lang"));

    // Not notes / not tags.
    assert!(g.id_of("md:NotALink").is_none(), "link inside code block");
    assert!(g.id_of("md:diagram.png").is_none(), "attachment embed");
    assert!(g.id_of("md:#notatag").is_none());
    assert!(g.id_of("md:#frontmatter-is-not-a-tag").is_none());

    // Undirected duplicates collapse: 9 unique relations.
    assert_eq!(g.edge_count(), 9);
    assert!(g.edges().iter().filter(|e| e.kind == EdgeKind::Tag).count() == 3);
}

#[test]
fn missing_directory_is_an_error() {
    assert!(MarkdownSource::new("definitely/not/here").load().is_err());
}
