use serde::Serialize;
use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PortProtocol {
    Tcp,
    Udp,
}

impl PortProtocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "tcp" => Some(Self::Tcp),
            "udp" => Some(Self::Udp),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct PublishedPort {
    pub host_port: u16,
    pub guest_port: u16,
    pub protocol: PortProtocol,
}

impl PublishedPort {
    pub fn localhost_url(self) -> Option<String> {
        (self.protocol == PortProtocol::Tcp).then(|| format!("http://127.0.0.1:{}", self.host_port))
    }
}

pub fn validate_published_ports(ports: &[PublishedPort]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for port in ports {
        if !seen.insert((port.host_port, port.protocol.as_str())) {
            return Err(format!(
                "host port {}/{} is published more than once",
                port.host_port,
                port.protocol.as_str()
            ));
        }
    }
    Ok(())
}

impl FromStr for PublishedPort {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let (ports, protocol) = match raw.rsplit_once('/') {
            Some((ports, protocol)) => (
                ports,
                PortProtocol::parse(protocol).ok_or_else(|| {
                    format!("invalid protocol `{protocol}` in `{raw}`; expected `tcp` or `udp`")
                })?,
            ),
            None => (raw, PortProtocol::Tcp),
        };
        let parts = ports.split(':').collect::<Vec<_>>();
        let (host_port, guest_port) = match parts.as_slice() {
            [port] => {
                let port = parse_port(port, raw)?;
                (port, port)
            }
            [host, guest] => (parse_port(host, raw)?, parse_port(guest, raw)?),
            _ => {
                return Err(format!(
                    "invalid port mapping `{raw}`; expected `GUEST_PORT` or `HOST_PORT:GUEST_PORT`"
                ));
            }
        };
        Ok(Self {
            host_port,
            guest_port,
            protocol,
        })
    }
}

impl fmt::Display for PublishedPort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}/{}",
            self.host_port,
            self.guest_port,
            self.protocol.as_str()
        )
    }
}

fn parse_port(value: &str, mapping: &str) -> Result<u16, String> {
    value
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| format!("invalid port in `{mapping}`; ports must be between 1 and 65535"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_full_and_udp_mappings() {
        assert_eq!(
            "3000".parse::<PublishedPort>().expect("short mapping"),
            PublishedPort {
                host_port: 3000,
                guest_port: 3000,
                protocol: PortProtocol::Tcp,
            }
        );
        assert_eq!(
            "8080:3000/udp"
                .parse::<PublishedPort>()
                .expect("full mapping"),
            PublishedPort {
                host_port: 8080,
                guest_port: 3000,
                protocol: PortProtocol::Udp,
            }
        );
    }

    #[test]
    fn rejects_invalid_ports_protocols_and_shapes() {
        for value in ["0", "70000", "abc", "1:2:3", "3000/sctp"] {
            assert!(
                value.parse::<PublishedPort>().is_err(),
                "{value} should fail"
            );
        }
    }

    #[test]
    fn rejects_duplicate_host_protocol_bindings() {
        let ports = vec![
            "8080:3000".parse().expect("port"),
            "8080:4000".parse().expect("port"),
        ];
        assert_eq!(
            validate_published_ports(&ports).expect_err("duplicate"),
            "host port 8080/tcp is published more than once"
        );
    }
}
