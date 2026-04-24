use std::fs;
use std::path::PathBuf;

#[test]
fn safe_sync_templates_embed_readiness_probe_scripts() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("lima");
    for name in [
        "provision/10-rust.sh",
        "provision/20-python-uv.sh",
        "provision/30-jvm-sdkman-zulu.sh",
        "provision/90-readiness.sh",
    ] {
        let path = root.join(name);
        assert!(
            !path.exists(),
            "built-in template should not ship optional language provisioning script: {}",
            path.display()
        );
    }

    for name in ["safe-sync-macos.yaml", "safe-sync-linux.yaml"] {
        let path = root.join(name);
        let content = fs::read_to_string(&path).expect("template");
        assert!(
            content.contains("probes:"),
            "template should declare probes: {}",
            path.display()
        );
        assert!(
            content.contains("script: |"),
            "template should embed probe script: {}",
            path.display()
        );
        assert!(
            content.contains("#!/bin/bash"),
            "template probe should start with a shebang: {}",
            path.display()
        );
        assert!(
            !content.contains("file: provision/90-readiness.sh"),
            "template should not reference probe file directly: {}",
            path.display()
        );
        assert!(
            !content.contains("file: provision/10-rust.sh"),
            "template should not hardcode Rust provisioning: {}",
            path.display()
        );
        assert!(
            !content.contains("file: provision/20-python-uv.sh"),
            "template should not hardcode Python/uv provisioning: {}",
            path.display()
        );
        assert!(
            !content.contains("file: provision/30-jvm-sdkman-zulu.sh"),
            "template should not hardcode JVM/SDKMAN provisioning: {}",
            path.display()
        );
        assert!(
            !content.contains(".cargo/bin/cargo"),
            "template probe should not require a Rust toolchain: {}",
            path.display()
        );
        assert!(
            !content.contains(".local/bin/uv"),
            "template probe should not require uv: {}",
            path.display()
        );
        assert!(
            !content.contains(".agbranch/sdkman-jdks.env"),
            "template probe should not require SDKMAN JDK metadata: {}",
            path.display()
        );
        assert!(
            !content.contains(".sdkman/bin/sdkman-init.sh"),
            "template probe should not require SDKMAN init hooks: {}",
            path.display()
        );
        assert!(
            content.contains("process.versions.node.split"),
            "template probe should assert a supported Node.js runtime for Gemini CLI: {}",
            path.display()
        );
        assert!(
            content.contains("runtime_ready() {"),
            "template probe should wrap runtime checks in a helper: {}",
            path.display()
        );
        assert!(
            content.contains("until runtime_ready; do"),
            "template probe should poll until the runtime is ready: {}",
            path.display()
        );
        assert!(
            content.contains("sleep 2"),
            "template probe should back off briefly between readiness checks: {}",
            path.display()
        );
        assert!(
            content.contains("gemini --version >/dev/null 2>&1"),
            "template probe should execute gemini --version, not just check PATH presence: {}",
            path.display()
        );
        if name == "safe-sync-macos.yaml" {
            assert!(
                !content.contains("\nrosetta:\n"),
                "macOS template should not use deprecated top-level rosetta: {}",
                path.display()
            );
            assert!(
                content.contains("vmOpts:"),
                "macOS template should configure Rosetta under vmOpts.vz: {}",
                path.display()
            );
            assert!(
                content.contains("vmOpts:\n  vz:\n    rosetta:"),
                "macOS template should keep Rosetta under vmOpts.vz: {}",
                path.display()
            );
        }
    }
}
