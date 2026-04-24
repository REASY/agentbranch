use std::fs;
use std::path::PathBuf;

#[test]
fn system_provision_script_retries_apt_and_installs_unzip() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("provision")
        .join("00-system.sh");
    let content = fs::read_to_string(&path).expect("system provision script");

    assert!(
        content.contains("Acquire::Retries=5"),
        "system bootstrap should retry apt downloads: {}",
        path.display()
    );
    assert!(
        content.contains("Acquire::http::Timeout=60"),
        "system bootstrap should extend apt HTTP timeout: {}",
        path.display()
    );
    assert!(
        content.contains("Acquire::https::Timeout=60"),
        "system bootstrap should extend apt HTTPS timeout: {}",
        path.display()
    );
    assert!(
        content.contains("Acquire::ForceIPv4=true"),
        "system bootstrap should force IPv4 for apt fetches: {}",
        path.display()
    );
    assert!(
        content.contains("unzip"),
        "system bootstrap must install unzip before SDKMAN provisioning: {}",
        path.display()
    );
}
