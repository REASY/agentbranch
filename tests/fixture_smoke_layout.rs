use std::path::PathBuf;

#[test]
fn sandbox_fixture_contains_expected_assets() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("e2e")
        .join("fixtures")
        .join("sandbox-workspace");

    for relative in ["README.md", ".gitignore"] {
        assert!(
            root.join(relative).exists(),
            "missing fixture asset: {relative}"
        );
    }

    for relative in ["Cargo.toml", "src/lib.rs"] {
        assert!(
            !root.join(relative).exists(),
            "fixture should not assume a built-in Rust toolchain: {relative}"
        );
    }

    assert!(
        !root.join("compose.yml").exists(),
        "compose fixture should be removed"
    );
}
