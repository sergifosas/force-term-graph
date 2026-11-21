use std::path::Path;
use std::process::Command;

fn graph() -> Command {
    Command::new(env!("CARGO_BIN_EXE_graph"))
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

#[test]
fn renders_json_fixture_to_png() {
    let out = std::env::temp_dir().join(format!("graph-cli-test-{}.png", std::process::id()));
    let status = graph()
        .args(["json"])
        .arg(fixture("small.json"))
        .args(["--png"])
        .arg(&out)
        .args(["--size", "640x400"])
        .status()
        .unwrap();
    assert!(status.success());
    let bytes = std::fs::read(&out).unwrap();
    std::fs::remove_file(&out).ok();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    // IHDR width/height (big-endian) at offsets 16..24.
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 640);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 400);
}

#[test]
fn md_and_fs_sources_render() {
    let out = std::env::temp_dir().join(format!("graph-cli-md-{}.png", std::process::id()));
    for args in [
        vec!["md".into(), fixture("vault")],
        vec!["fs".into(), fixture("vault")],
    ] {
        let o = graph()
            .args(&args)
            .arg("--png")
            .arg(&out)
            .args(["--size", "320x200"])
            .output()
            .unwrap();
        assert!(
            o.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    std::fs::remove_file(&out).ok();
}

#[test]
fn bare_graph_in_a_folder_without_notes_suggests_alternatives() {
    let empty = std::env::temp_dir().join(format!("graph-cli-empty-{}", std::process::id()));
    std::fs::create_dir_all(&empty).unwrap();
    let o = graph()
        .current_dir(&empty)
        .args(["--png", "x.png"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&empty).ok();
    assert!(!o.status.success());
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("no Markdown notes") && err.contains("graph fs"),
        "{err}"
    );
}

#[test]
fn rejects_bad_size_and_missing_file() {
    let o = graph()
        .args(["demo", "--png", "x.png", "--size", "big"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    let o = graph()
        .args(["json", "does-not-exist.json", "--png", "x.png"])
        .output()
        .unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("does-not-exist.json"));
}

#[test]
fn pane_without_a_terminal_explains_itself() {
    // (Without --pane, a non-terminal falls back to the GPU window.)
    let o = graph().args(["demo", "--pane"]).output().unwrap();
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--png"));
}
