use std::fs;
use std::path::PathBuf;

#[test]
fn network_guard_allows_dns_to_discovered_uplinks_and_lima_dnat_targets() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("provision")
        .join("05-network-guard.sh");
    let content = fs::read_to_string(&path).expect("network guard script");

    assert!(
        content.contains("resolvectl dns"),
        "network guard should discover DNS uplinks: {}",
        path.display()
    );
    assert!(
        content.contains("--dport 53"),
        "network guard should exempt DNS traffic specifically: {}",
        path.display()
    );
    assert!(
        content.contains("iptables -t nat -S LIMADNS"),
        "network guard should allow Lima hostResolver DNAT targets: {}",
        path.display()
    );
}
