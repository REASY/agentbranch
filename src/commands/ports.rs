use crate::cli::PortsArgs;
use crate::commands::{find_existing_session, resolve_session_name};
use crate::db::connect::open_catalog;
use crate::db::ports::list_session_ports;
use crate::error::AppError;
use crate::lima::client::{LimaClient, LimactlClient};
use crate::lima::inspect::LimaInstanceStatus;
use crate::platform::host::HostContext;
use crate::ports::{PortProtocol, PublishedPort};
use crate::util::process::RealCommandRunner;
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublishedPortStatus {
    pub host: String,
    pub host_port: u16,
    pub guest_port: u16,
    pub protocol: PortProtocol,
    pub url: Option<String>,
    pub listening: Option<bool>,
}

pub fn run(args: PortsArgs) -> Result<(), AppError> {
    let (session_name_raw, session_name) = resolve_session_name(&args.session)?;
    let host = HostContext::detect()?;
    let conn = open_catalog(&host.state_roots.db)?;
    let session = find_existing_session(&conn, &session_name, &session_name_raw)?;
    let published = list_session_ports(&conn, &session_name)?;

    let runner = RealCommandRunner;
    let lima = LimactlClient::new(&runner);
    let listening = listening_guest_ports(&lima, &session.vm_name);
    let statuses = published
        .into_iter()
        .map(|port| port_status(port, listening.as_ref()))
        .collect::<Vec<_>>();

    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "session": session_name,
                "published": statuses,
            })
        );
    } else {
        println!("{}", render_human(&session_name_raw, &statuses));
    }
    Ok(())
}

fn listening_guest_ports(
    lima: &dyn LimaClient,
    vm_name: &crate::types::VmName,
) -> Option<BTreeSet<(PortProtocolKey, u16)>> {
    let running = lima.list_instances().ok()?.into_iter().any(|instance| {
        instance.name == vm_name.as_str() && instance.status == LimaInstanceStatus::Running
    });
    if !running {
        return None;
    }
    let output = lima.bash(vm_name, "ss -H -lntu 2>/dev/null || true").ok()?;
    Some(parse_listening_ports(&output.stdout))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum PortProtocolKey {
    Tcp,
    Udp,
}

impl From<PortProtocol> for PortProtocolKey {
    fn from(value: PortProtocol) -> Self {
        match value {
            PortProtocol::Tcp => Self::Tcp,
            PortProtocol::Udp => Self::Udp,
        }
    }
}

fn parse_listening_ports(output: &str) -> BTreeSet<(PortProtocolKey, u16)> {
    output
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            let protocol = match fields.first().copied()? {
                "tcp" => PortProtocolKey::Tcp,
                "udp" => PortProtocolKey::Udp,
                _ => return None,
            };
            let endpoint = fields.get(4)?;
            let port = endpoint.rsplit_once(':')?.1.parse::<u16>().ok()?;
            Some((protocol, port))
        })
        .collect()
}

fn port_status(
    port: PublishedPort,
    listening: Option<&BTreeSet<(PortProtocolKey, u16)>>,
) -> PublishedPortStatus {
    PublishedPortStatus {
        host: "127.0.0.1".to_owned(),
        host_port: port.host_port,
        guest_port: port.guest_port,
        protocol: port.protocol,
        url: port.localhost_url(),
        listening: listening.map(|ports| ports.contains(&(port.protocol.into(), port.guest_port))),
    }
}

fn render_human(session: &str, statuses: &[PublishedPortStatus]) -> String {
    if statuses.is_empty() {
        return format!(
            "No ports are published for session `{session}`.\n\
             Add `--publish GUEST_PORT` or `--publish HOST_PORT:GUEST_PORT` when launching."
        );
    }
    let mut lines = vec![format!("Published ports for {session}:")];
    for status in statuses {
        let state = match status.listening {
            Some(true) => "listening",
            Some(false) => "not listening",
            None => "VM stopped or unavailable",
        };
        let endpoint = status.url.clone().unwrap_or_else(|| {
            format!(
                "{}:{}/{}",
                status.host,
                status.host_port,
                status.protocol.as_str()
            )
        });
        lines.push(format!(
            "  {endpoint} -> guest :{}/{} ({state})",
            status.guest_port,
            status.protocol.as_str()
        ));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tcp_and_udp_listener_output() {
        let output = "\
tcp LISTEN 0 4096 0.0.0.0:3000 0.0.0.0:*\n\
udp UNCONN 0 0 [::]:5353 [::]:*\n";
        let parsed = parse_listening_ports(output);
        assert!(parsed.contains(&(PortProtocolKey::Tcp, 3000)));
        assert!(parsed.contains(&(PortProtocolKey::Udp, 5353)));
    }

    #[test]
    fn renders_urls_and_listener_state() {
        let published = "8080:3000".parse::<PublishedPort>().expect("port");
        let listening = BTreeSet::from([(PortProtocolKey::Tcp, 3000)]);
        let status = port_status(published, Some(&listening));
        assert_eq!(status.url.as_deref(), Some("http://127.0.0.1:8080"));
        assert_eq!(status.listening, Some(true));
        assert!(
            render_human("demo", &[status])
                .contains("http://127.0.0.1:8080 -> guest :3000/tcp (listening)")
        );
    }
}
