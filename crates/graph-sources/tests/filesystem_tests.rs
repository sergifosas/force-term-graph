use std::fs;
use std::path::PathBuf;

use graph_core::EdgeKind;
use graph_sources::{FsSource, Source, load_all};

/// A throwaway directory tree, removed on drop.
struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("graph-fs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in ["src/nested", "target/debug", ".hidden"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        for file in [
            "Cargo.toml",
            "src/main.rs",
            "src/lib.rs",
            "src/nested/deep.rs",
            "target/debug/app.exe",
            ".hidden/secret.txt",
            "README",
        ] {
            fs::write(root.join(file), "x").unwrap();
        }
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        Tree(root)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn load(src: FsSource) -> graph_core::Graph {
    let sources: Vec<Box<dyn Source>> = vec![Box::new(src)];
    load_all(&sources).unwrap()
}

#[test]
fn builds_containment_tree_respecting_gitignore_and_hidden() {
    let t = Tree::new("basic");
    let g = load(FsSource::new(&t.0));
    let keys: Vec<&str> = g.nodes().iter().map(|n| n.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "fs:.",
            "fs:Cargo.toml",
            "fs:README",
            "fs:src",
            "fs:src/lib.rs",
            "fs:src/main.rs",
            "fs:src/nested",
            "fs:src/nested/deep.rs",
        ]
    );
    // A tree: n - 1 edges, all containment.
    assert_eq!(g.edge_count(), keys.len() - 1);
    assert!(g.edges().iter().all(|e| e.kind == EdgeKind::Contains));
    assert!(g.nodes().iter().all(|n| !n.ghost));

    let node = |k: &str| g.node(g.id_of(k).unwrap());
    assert_eq!(node("fs:src").category, "dir");
    assert_eq!(node("fs:src/main.rs").category, "rs");
    assert_eq!(node("fs:README").category, "file");
    assert_eq!(node("fs:src/nested/deep.rs").label, "deep.rs");
    assert!(node("fs:.").label.starts_with("graph-fs-basic"));
}

#[test]
fn depth_all_and_node_limits() {
    let t = Tree::new("limits");

    let mut shallow = FsSource::new(&t.0);
    shallow.max_depth = Some(1);
    assert!(load(shallow).id_of("fs:src/main.rs").is_none());

    let mut all = FsSource::new(&t.0);
    all.all = true;
    let g = load(all);
    assert!(g.id_of("fs:target/debug/app.exe").is_some());
    assert!(g.id_of("fs:.hidden/secret.txt").is_some());

    let mut capped = FsSource::new(&t.0);
    capped.max_nodes = 3;
    let data = capped.load().unwrap();
    assert_eq!(data.nodes.len(), 3);
    assert_eq!(data.nodes[0].meta["truncated_at"], 3);
}
