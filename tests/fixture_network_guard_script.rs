use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::tempdir;

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
    assert!(
        content.contains("iptables -I FORWARD 1") && content.contains("iptables -I DOCKER-USER 1"),
        "network guard should run before Docker forwarding accepts: {}",
        path.display()
    );
    assert!(
        content.contains("ip6tables -I OUTPUT 1")
            && content.contains("ip6tables -I FORWARD 1")
            && content.contains("fc00::/7")
            && content.contains("fe80::/10"),
        "network guard should cover private and link-local IPv6 egress: {}",
        path.display()
    );
    assert!(
        content.contains("agbranch-network-guard.service")
            && content.contains("WantedBy=multi-user.target"),
        "network guard should be reapplied after guest boots: {}",
        path.display()
    );
}

#[cfg(unix)]
#[test]
fn network_guard_applies_private_network_rejects_idempotently() {
    let root = tempdir().expect("tempdir");
    let bin = root.path().join("bin");
    fs::create_dir(&bin).expect("bin");
    let log = root.path().join("rules.log");
    let stub = bin.join("stub");
    fs::write(
        &stub,
        r#"#!/bin/sh
name=$(basename "$0")
printf '%s %s\n' "$name" "$*" >> "$AGBRANCH_RULE_LOG"
case "$name $*" in
  "install "*) exit 0 ;;
  "resolvectl dns"*) printf '%s\n' 'Link 2 (eth0): 1.1.1.1 2606:4700:4700::1111'; exit 0 ;;
  "ip -o -4"*) printf '%s\n' '172.18.0.0/16 dev docker0'; exit 0 ;;
  "ip -o -6"*) printf '%s\n' 'fd00:abcd::/64 dev docker0'; exit 0 ;;
  "iptables -t nat -S LIMADNS"*)
    printf '%s\n' '-A LIMADNS -d 192.168.5.2/32 -p udp -m udp --dport 53 -j DNAT --to-destination 192.168.5.3:60053'
    exit 0
    ;;
  "iptables -C"*|"ip6tables -C"*) exit 1 ;;
  "iptables -S DOCKER-USER"|"ip6tables -S DOCKER-USER") exit 0 ;;
esac
exit 0
"#,
    )
    .expect("stub");
    let mut permissions = fs::metadata(&stub).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub, permissions).expect("permissions");
    for name in ["install", "iptables", "ip6tables", "resolvectl", "ip"] {
        std::os::unix::fs::symlink(&stub, bin.join(name)).expect("stub link");
    }

    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lima")
        .join("provision")
        .join("05-network-guard.sh");
    let system_path = std::env::var("PATH").expect("PATH");
    let path = format!("{}:{system_path}", bin.display());
    for _ in 0..2 {
        let status = Command::new("bash")
            .arg(&script)
            .arg("--apply-only")
            .env("PATH", &path)
            .env("AGBRANCH_RULE_LOG", &log)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("network guard");
        assert!(status.success(), "network guard should apply");
    }

    let applied = fs::read_to_string(&log).expect("rule log");
    for rule in [
        "iptables -A AGBRANCH_OUTPUT_GUARD -d 10.0.0.0/8 -j REJECT",
        "iptables -A AGBRANCH_FORWARD_GUARD -d 192.168.0.0/16 -j REJECT",
        "ip6tables -A AGBRANCH_OUTPUT_GUARD -d fc00::/7 -j REJECT",
        "ip6tables -A AGBRANCH_FORWARD_GUARD -d fe80::/10 -j REJECT",
        "iptables -I DOCKER-USER 1 -j AGBRANCH_FORWARD_GUARD",
    ] {
        assert!(
            applied.matches(rule).count() == 2,
            "expected each application to install `{rule}` exactly once:\n{applied}"
        );
    }
    let dns_allow = applied
        .find("iptables -A AGBRANCH_OUTPUT_GUARD -p udp -d 1.1.1.1 --dport 53 -j RETURN")
        .expect("DNS allow");
    let private_reject = applied
        .find("iptables -A AGBRANCH_OUTPUT_GUARD -d 10.0.0.0/8 -j REJECT")
        .expect("private reject");
    assert!(
        dns_allow < private_reject,
        "DNS exemption must precede private-network rejects"
    );
}
