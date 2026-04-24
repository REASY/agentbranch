use std::fs;
use std::path::PathBuf;

#[test]
fn docker_compose_provision_script_forces_ipv4_for_apt() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("provision")
        .join("20-docker-compose.sh");
    let content = fs::read_to_string(&path).expect("docker compose provision script");

    assert!(
        content.contains("Acquire::ForceIPv4=true"),
        "docker compose provisioning should force IPv4 for apt fetches: {}",
        path.display()
    );
    assert!(
        content.contains("docker-compose-plugin"),
        "docker compose provisioning must install the compose plugin: {}",
        path.display()
    );
}
