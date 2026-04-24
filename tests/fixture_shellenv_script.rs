use std::fs;
use std::path::PathBuf;

#[test]
fn shellenv_normalizes_compose_project_names_to_lowercase() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("guest")
        .join("shellenv.sh");
    let content = fs::read_to_string(&path).expect("shellenv.sh");

    assert!(
        content.contains("AGBRANCH_SESSION_SAFE_LOWER"),
        "shellenv should derive a lowercase-safe session identifier: {}",
        path.display()
    );
    assert!(
        content.contains("COMPOSE_PROJECT_NAME=\"agbranch_${AGBRANCH_SESSION_SAFE_LOWER}\""),
        "shellenv should export a lowercase compose project name: {}",
        path.display()
    );
    assert!(
        content.contains("tr '[:upper:]' '[:lower:]'"),
        "shellenv should lowercase session identifiers portably: {}",
        path.display()
    );
    assert!(
        content.contains("PATH=\"${HOME}/.agbranch/bin:${HOME}/.local/bin:${PATH}\""),
        "shellenv should prepend the session-local shim directory to PATH without assuming Rust is installed: {}",
        path.display()
    );
    assert!(
        !content.contains(".cargo/bin"),
        "shellenv should not hardcode Rust toolchain paths: {}",
        path.display()
    );
    assert!(
        !content.contains("UV_CACHE_DIR"),
        "shellenv should not export uv-specific state by default: {}",
        path.display()
    );
    assert!(
        !content.contains("UV_PROJECT_ENVIRONMENT"),
        "shellenv should not export uv virtualenv paths by default: {}",
        path.display()
    );
    assert!(
        !content.contains("CARGO_TARGET_DIR"),
        "shellenv should not export Cargo target paths by default: {}",
        path.display()
    );
    assert!(
        !content.contains("SBT_OPTS"),
        "shellenv should not export sbt-specific settings by default: {}",
        path.display()
    );
    assert!(
        !content.contains("sdkman-jdks.env"),
        "shellenv should not source SDKMAN JDK metadata by default: {}",
        path.display()
    );
    assert!(
        !content.contains("sdkman-init.sh"),
        "shellenv should not source SDKMAN init hooks by default: {}",
        path.display()
    );
}
