use std::fs;
use std::path::PathBuf;

#[test]
fn agent_cli_provision_script_pins_supported_node_major() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("provision")
        .join("10-agent-clis.sh");
    let content = fs::read_to_string(&path).expect("provision script");

    assert!(
        content.contains("NODE_MAJOR=20"),
        "agent CLI provisioning must pin Node.js 20+ for Gemini CLI compatibility: {}",
        path.display()
    );
    assert!(
        content.contains("deb.nodesource.com/node_${NODE_MAJOR}.x"),
        "agent CLI provisioning should install Node from a Node 20+ source: {}",
        path.display()
    );
    assert!(
        content.contains("NODE_VERSION=\"20.20.2-1nodesource1\"")
            && content.contains("Pin-Priority: 600")
            && content.contains("\"nodejs=${NODE_VERSION}\"")
            && content.contains("command -v npm"),
        "agent CLI provisioning must select an exact NodeSource build that includes npm: {}",
        path.display()
    );
    assert!(
        content.contains("gpg --batch --yes --dearmor -o /etc/apt/keyrings/nodesource.gpg"),
        "agent CLI provisioning should refresh the NodeSource keyring non-interactively on reruns: {}",
        path.display()
    );
    assert!(
        content.contains("Acquire::ForceIPv4=true"),
        "agent CLI provisioning should force IPv4 for apt fetches: {}",
        path.display()
    );
    for package in [
        "\"@openai/codex@${CODEX_VERSION}\"",
        "\"@anthropic-ai/claude-code@${CLAUDE_CODE_VERSION}\"",
        "\"@google/gemini-cli@${GEMINI_CLI_VERSION}\"",
    ] {
        assert!(
            content.contains(package),
            "agent CLI provisioning must pin {package}: {}",
            path.display()
        );
    }
    assert!(
        content.contains("VERSION_STAMP=")
            && content.contains("cat \"${MARKER_FILE}\"")
            && content.contains("printf '%s\\n' \"${VERSION_STAMP}\""),
        "the provisioning marker must encode installed versions so upgrades rerun: {}",
        path.display()
    );
}
