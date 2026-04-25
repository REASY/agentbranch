use crate::error::lima::LimaError;
use serde::Deserialize;
use serde_json::Deserializer;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LimaInstance {
    pub name: String,
    #[serde(rename = "dir")]
    pub instance_dir: String,
    #[serde(rename = "sshConfigFile")]
    pub ssh_config_file: String,
    #[serde(rename = "vmType")]
    pub vm_type: String,
    #[serde(rename = "status")]
    pub raw_status: String,
    #[serde(default)]
    pub arch: Option<String>,
    #[serde(default)]
    pub cpus: Option<u16>,
    #[serde(default)]
    pub memory: Option<u64>,
    #[serde(default)]
    pub disk: Option<u64>,
    #[serde(default)]
    pub protected: bool,
    #[serde(rename = "sshLocalPort", default)]
    pub ssh_local_port: Option<u16>,
    #[serde(rename = "sshAddress", default)]
    pub ssh_address: Option<String>,
    #[serde(default)]
    pub config: LimaConfig,
    #[serde(skip, default)]
    pub status: LimaInstanceStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
pub struct LimaConfig {
    #[serde(default)]
    pub mounts: Vec<LimaMount>,
    #[serde(default)]
    pub rosetta: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LimaMount {
    pub location: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LimaInstanceStatus {
    Running,
    Stopped,
    #[default]
    Other,
}

impl LimaInstance {
    pub fn has_host_mounts(&self) -> bool {
        !self.config.mounts.is_empty()
    }

    pub fn has_deprecated_top_level_rosetta(&self) -> bool {
        self.config.rosetta.is_some()
    }

    pub fn is_running(&self) -> bool {
        self.status == LimaInstanceStatus::Running
    }
}

pub fn parse_instances(json: &str) -> Result<Vec<LimaInstance>, LimaError> {
    if json.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut items = match serde_json::from_str::<Vec<LimaInstance>>(json) {
        Ok(items) => items,
        Err(_) => Deserializer::from_str(json)
            .into_iter::<LimaInstance>()
            .collect::<Result<Vec<_>, _>>()?,
    };
    for item in &mut items {
        item.status = match item.raw_status.as_str() {
            "Running" => LimaInstanceStatus::Running,
            "Stopped" => LimaInstanceStatus::Stopped,
            _ => LimaInstanceStatus::Other,
        };
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_running_and_stopped_instances() {
        let json = r#"
        [
          {"name":"agbranch-base","status":"Running","vmType":"vz","dir":"/tmp/base","sshConfigFile":"/tmp/base/ssh.config"},
          {"name":"agbranch-feat-a","status":"Stopped","vmType":"qemu","dir":"/tmp/feat","sshConfigFile":"/tmp/feat/ssh.config"}
        ]
        "#;

        let instances = parse_instances(json).expect("parse should succeed");
        assert_eq!(instances[0].status, LimaInstanceStatus::Running);
        assert_eq!(instances[1].status, LimaInstanceStatus::Stopped);
    }

    #[test]
    fn treats_empty_json_output_as_no_instances() {
        let instances = parse_instances("").expect("empty output should mean no instances");
        assert!(instances.is_empty());
    }

    #[test]
    fn parses_single_instance_object_output() {
        let json = r#"{"name":"agbranch-base","status":"Running","vmType":"vz","dir":"/tmp/base","sshConfigFile":"/tmp/base/ssh.config"}"#;

        let instances = parse_instances(json).expect("single object output should parse");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].status, LimaInstanceStatus::Running);
    }

    #[test]
    fn parses_mounts_and_top_level_rosetta_config() {
        let json = r#"
        {
          "name":"agbranch-base",
          "status":"Running",
          "vmType":"vz",
          "dir":"/tmp/base",
          "sshConfigFile":"/tmp/base/ssh.config",
          "config":{
            "mounts":[{"location":"/Users/tester","mountPoint":"/Users/tester","writable":false}],
            "rosetta":{"enabled":true}
          }
        }
        "#;

        let instances = parse_instances(json).expect("single object output should parse");
        assert_eq!(instances.len(), 1);
        assert!(instances[0].has_host_mounts());
        assert!(instances[0].has_deprecated_top_level_rosetta());
    }
}
