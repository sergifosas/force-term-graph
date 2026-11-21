use graph_sources::{JsonSource, Source, load_all};

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn loads_fixture_with_ghost_for_unresolved_target() {
    let sources: Vec<Box<dyn Source>> = vec![Box::new(JsonSource::new(fixture("small.json")))];
    let g = load_all(&sources).unwrap();
    assert_eq!(g.node_count(), 17, "16 defined + 1 ghost");
    assert_eq!(g.edge_count(), 18);
    let ghost = g.id_of("json:wikilinks").unwrap();
    assert!(g.node(ghost).ghost);
    assert_eq!(g.node(g.id_of("json:rust").unwrap()).category, "language");
}

#[test]
fn reports_missing_file_and_bad_json_with_context() {
    let err = JsonSource::new(fixture("nope.json")).load().unwrap_err();
    assert!(format!("{err:#}").contains("nope.json"));
    assert!(JsonSource::parse("{ not json").is_err());
}
