use crate::lima::inspect::LimaInstance;
use crate::platform::detect::HostPlatform;
use crate::types::DiskSize;
use crate::util::ids::prepared_base_name_from_override;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseMetadata {
    pub schema_version: u16,
    pub prepared_at: String,
    pub provision_fingerprint: String,
    /// Origin of `provision_fingerprint`: `"baked_in"` for the compile-time
    /// constant, `"env_override"` for fingerprints recomputed from an
    /// override tree. Absent for metadata written by older binaries — those
    /// are treated as `"baked_in"` for backward compatibility. Unknown
    /// values are treated as invalid metadata, matching the
    /// `schema_version` policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision_fingerprint_source: Option<String>,
    pub agent_cli_versions: BTreeMap<String, String>,
}

/// Canonical string values for `BaseMetadata::provision_fingerprint_source`.
pub const PROVISION_SOURCE_BAKED_IN: &str = "baked_in";
pub const PROVISION_SOURCE_ENV_OVERRIDE: &str = "env_override";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameSource {
    Default,
    EnvOverride,
}

impl NameSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::EnvOverride => "env_override",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessIssue {
    Missing,
    MetadataMissing,
    Stale,
    Unprotected,
    /// `lima_assets` inspect reports that the override path is invalid.
    LimaAssetsOverrideInvalid,
    /// `lima_assets` inspect reports `WouldExtract`: cache is absent or
    /// stale, so the next mutating command must extract before proceeding.
    LimaAssetsCacheNeedsExtraction,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaseSummary {
    pub name: String,
    pub name_source: String,
    pub status: String,
    pub location: Option<String>,
    pub protected: bool,
    pub prepared: bool,
    pub size_bytes: Option<u64>,
    pub disk_bytes: Option<u64>,
    pub created_at: Option<String>,
    pub prepared_at: Option<String>,
    pub provision_fingerprint: String,
    pub prepared_provision_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provision_fingerprint_source: Option<String>,
    pub provision_stale: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    pub agent_cli_versions: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lima_assets: Option<crate::lima::asset_inspect::LimaAssetInspect>,
}

pub struct BaseSummaryInput {
    pub name: String,
    pub name_source: NameSource,
    pub instance: Option<LimaInstance>,
    pub metadata: Option<BaseMetadata>,
    pub metadata_valid: bool,
    pub current_fingerprint: String,
    /// Which lineage the running binary is resolving assets from. When this
    /// differs from the metadata's `provision_fingerprint_source`, the base
    /// was stamped under a different lineage and `stale_reason` becomes
    /// `lineage_mismatch`.
    pub current_fingerprint_source: CurrentFingerprintSource,
    pub size_bytes: Option<u64>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurrentFingerprintSource {
    BakedIn,
    EnvOverride,
}

impl CurrentFingerprintSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BakedIn => PROVISION_SOURCE_BAKED_IN,
            Self::EnvOverride => PROVISION_SOURCE_ENV_OVERRIDE,
        }
    }
}

pub fn summarize_expected_base(
    platform: HostPlatform,
    instances: &[LimaInstance],
    current_fingerprint: &str,
) -> BaseSummary {
    summarize_expected_base_with_lineage(
        platform,
        instances,
        current_fingerprint,
        CurrentFingerprintSource::BakedIn,
    )
}

pub fn summarize_expected_base_with_lineage(
    platform: HostPlatform,
    instances: &[LimaInstance],
    current_fingerprint: &str,
    current_fingerprint_source: CurrentFingerprintSource,
) -> BaseSummary {
    let override_name = std::env::var("AGBRANCH_PREPARED_BASE_NAME").ok();
    let name_source = if override_name.is_some() {
        NameSource::EnvOverride
    } else {
        NameSource::Default
    };
    let name = prepared_base_name_from_override(platform, override_name.as_deref());
    let instance = instances
        .iter()
        .find(|instance| instance.name == name.as_str())
        .cloned();
    let Some(instance_ref) = instance.as_ref() else {
        return BaseSummary::missing(name.as_str(), name_source, current_fingerprint);
    };

    let instance_dir = PathBuf::from(&instance_ref.instance_dir);
    let (metadata, metadata_valid) = read_metadata(&instance_dir).unwrap_or((None, false));
    let size_bytes = allocated_size(&instance_dir).ok();
    let created_at = created_at(&instance_dir).ok().flatten();

    let mut summary = BaseSummary::from_parts(BaseSummaryInput {
        name: name.as_str().to_owned(),
        name_source,
        instance,
        metadata,
        metadata_valid,
        current_fingerprint: current_fingerprint.to_owned(),
        current_fingerprint_source,
        size_bytes,
        created_at,
    });
    if summary.disk_bytes.is_none() {
        summary.disk_bytes = read_lima_yaml_disk_bytes(&instance_dir).ok().flatten();
    }
    summary
}

pub fn metadata_path(instance_dir: &Path) -> PathBuf {
    instance_dir.join("agbranch-base.json")
}

pub fn write_metadata_atomic(instance_dir: &Path, metadata: &BaseMetadata) -> std::io::Result<()> {
    let target = metadata_path(instance_dir);
    let mut temp = tempfile::NamedTempFile::new_in(instance_dir)?;
    let bytes = serde_json::to_vec_pretty(metadata)?;
    temp.write_all(&bytes)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(&target)
        .map_err(|err| std::io::Error::new(err.error.kind(), err.error))?;
    Ok(())
}

pub fn read_metadata(instance_dir: &Path) -> Result<(Option<BaseMetadata>, bool), std::io::Error> {
    let path = metadata_path(instance_dir);
    if !path.exists() {
        return Ok((None, false));
    }
    let contents = fs::read_to_string(path)?;
    let metadata = serde_json::from_str::<BaseMetadata>(&contents);
    match metadata {
        Ok(metadata) => {
            let valid = metadata.schema_version == 1;
            Ok((Some(metadata), valid))
        }
        Err(_) => Ok((None, false)),
    }
}

pub fn allocated_size(path: &Path) -> std::io::Result<u64> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_file() {
        return Ok(allocated_file_size(&metadata));
    }
    if metadata.file_type().is_symlink() {
        return Ok(0);
    }

    let mut total = allocated_file_size(&metadata);
    for entry in fs::read_dir(path)? {
        total = total.saturating_add(allocated_size(&entry?.path())?);
    }
    Ok(total)
}

#[cfg(unix)]
fn allocated_file_size(metadata: &fs::Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().saturating_mul(512)
}

#[cfg(not(unix))]
fn allocated_file_size(metadata: &fs::Metadata) -> u64 {
    metadata.len()
}

fn created_at(path: &Path) -> std::io::Result<Option<String>> {
    let metadata = fs::metadata(path)?;
    let Ok(created) = metadata.created() else {
        return Ok(None);
    };
    Ok(Some(format_rfc3339_seconds(OffsetDateTime::from(created))))
}

fn read_lima_yaml_disk_bytes(instance_dir: &Path) -> std::io::Result<Option<u64>> {
    let path = instance_dir.join("lima.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let contents = fs::read_to_string(path)?;
    Ok(contents
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find_map(|(key, value)| {
            (key.trim() == "disk")
                .then(|| value.trim().trim_matches(['"', '\'']))
                .and_then(|value| DiskSize::parse(value).ok())
                .map(|disk| disk.as_bytes())
        }))
}

impl BaseSummary {
    pub fn missing(name: &str, name_source: NameSource, current_fingerprint: &str) -> Self {
        Self {
            name: name.to_owned(),
            name_source: name_source.as_str().to_owned(),
            status: "missing".to_owned(),
            location: None,
            protected: false,
            prepared: false,
            size_bytes: None,
            disk_bytes: None,
            created_at: None,
            prepared_at: None,
            provision_fingerprint: current_fingerprint.to_owned(),
            prepared_provision_fingerprint: None,
            provision_fingerprint_source: None,
            provision_stale: None,
            stale_reason: None,
            agent_cli_versions: BTreeMap::new(),
            lima_assets: None,
        }
    }

    pub fn from_parts(input: BaseSummaryInput) -> Self {
        let Some(instance) = input.instance else {
            return Self::missing(&input.name, input.name_source, &input.current_fingerprint);
        };

        // Metadata is considered valid only when schema_version == 1 AND
        // provision_fingerprint_source is absent (older metadata) or one of
        // the two canonical values. Unknown values are treated like an
        // unknown schema_version.
        let metadata_has_known_lineage = input
            .metadata
            .as_ref()
            .map(|m| {
                matches!(
                    m.provision_fingerprint_source.as_deref(),
                    None | Some(PROVISION_SOURCE_BAKED_IN) | Some(PROVISION_SOURCE_ENV_OVERRIDE)
                )
            })
            .unwrap_or(true);
        let metadata = input.metadata.filter(|metadata| {
            input.metadata_valid && metadata.schema_version == 1 && metadata_has_known_lineage
        });
        let prepared = metadata.is_some();
        let prepared_at = metadata
            .as_ref()
            .map(|metadata| normalize_rfc3339_to_seconds(&metadata.prepared_at));
        let prepared_provision_fingerprint = metadata
            .as_ref()
            .map(|metadata| metadata.provision_fingerprint.clone());
        let metadata_source = metadata.as_ref().map(|m| {
            m.provision_fingerprint_source
                .as_deref()
                .unwrap_or(PROVISION_SOURCE_BAKED_IN)
                .to_owned()
        });
        let provision_stale = prepared_provision_fingerprint
            .as_ref()
            .map(|fingerprint| fingerprint != &input.current_fingerprint);
        let stale_reason = match (provision_stale, metadata_source.as_deref()) {
            (Some(true), Some(stamped)) if stamped != input.current_fingerprint_source.as_str() => {
                Some("lineage_mismatch".to_owned())
            }
            (Some(true), _) => Some("fingerprint_diverged".to_owned()),
            _ => None,
        };
        let agent_cli_versions = metadata
            .as_ref()
            .map(|metadata| metadata.agent_cli_versions.clone())
            .unwrap_or_default();

        Self {
            name: input.name,
            name_source: input.name_source.as_str().to_owned(),
            status: lower_lima_status(&instance.raw_status),
            location: Some(instance.instance_dir),
            protected: instance.protected,
            prepared,
            size_bytes: input.size_bytes,
            disk_bytes: instance.disk,
            created_at: input.created_at,
            prepared_at,
            provision_fingerprint: input.current_fingerprint,
            prepared_provision_fingerprint,
            provision_fingerprint_source: metadata_source,
            provision_stale,
            stale_reason,
            agent_cli_versions,
            lima_assets: None,
        }
    }

    pub fn readiness_issue(&self) -> Option<ReadinessIssue> {
        // Base readiness comes first; asset readiness is a secondary gate
        // below, so scripts that care only about the base keep their
        // existing priority ordering.
        if self.status == "missing" {
            return Some(ReadinessIssue::Missing);
        }
        if !self.prepared {
            return Some(ReadinessIssue::MetadataMissing);
        }
        if self.provision_stale == Some(true) {
            return Some(ReadinessIssue::Stale);
        }
        if !self.protected {
            return Some(ReadinessIssue::Unprotected);
        }

        if let Some(assets) = self.lima_assets.as_ref() {
            match &assets.state {
                crate::lima::asset_inspect::LimaAssetInspectState::Cache { .. }
                | crate::lima::asset_inspect::LimaAssetInspectState::EnvOverride { .. } => {}
                crate::lima::asset_inspect::LimaAssetInspectState::InvalidOverride { .. } => {
                    return Some(ReadinessIssue::LimaAssetsOverrideInvalid);
                }
                crate::lima::asset_inspect::LimaAssetInspectState::WouldExtract { .. } => {
                    return Some(ReadinessIssue::LimaAssetsCacheNeedsExtraction);
                }
            }
        }

        None
    }

    pub fn require_ready_error(&self) -> &'static str {
        match self.readiness_issue() {
            Some(ReadinessIssue::Missing) => "base is missing: run agbranch base prepare",
            Some(ReadinessIssue::MetadataMissing) => {
                "base metadata is missing or invalid: run agbranch base prepare"
            }
            Some(ReadinessIssue::Stale) => "base is stale: run agbranch base prepare --rebuild",
            Some(ReadinessIssue::Unprotected) => "base is unprotected: run agbranch base prepare",
            Some(ReadinessIssue::LimaAssetsOverrideInvalid) => {
                "lima assets override is invalid: set or fix AGBRANCH_LIMA_ASSETS_DIR (see agbranch base show for details)"
            }
            Some(ReadinessIssue::LimaAssetsCacheNeedsExtraction) => {
                "lima assets cache needs extraction: run any command to extract (or agbranch internal extract-assets)"
            }
            None => "base is ready",
        }
    }

    pub fn render_human(&self) -> String {
        if self.status == "missing" {
            return format!("Base {} is missing.\nRun: agbranch base prepare", self.name);
        }

        let protection = if self.protected {
            "protected"
        } else {
            "unprotected"
        };
        let size = self
            .size_bytes
            .map(format_gib)
            .unwrap_or_else(|| "unknown".to_owned());
        let disk = self
            .disk_bytes
            .map(format_gib)
            .unwrap_or_else(|| "unknown".to_owned());
        let created = self.created_at.as_deref().unwrap_or("unknown");
        let prepared = self
            .prepared_at
            .as_deref()
            .unwrap_or("unknown - run agbranch base prepare");
        let fingerprint = self
            .prepared_provision_fingerprint
            .as_deref()
            .unwrap_or("unknown - run agbranch base prepare");
        let versions = if self.agent_cli_versions.is_empty() {
            "unknown".to_owned()
        } else {
            self.agent_cli_versions
                .iter()
                .map(|(name, version)| format!("{name} {version}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut lines = vec![
            format!("Name:         {}", self.name),
            format!("Status:       {} ({protection})", self.status),
            format!(
                "Location:     {}",
                self.location.as_deref().unwrap_or("unknown")
            ),
            format!("Size on disk: {size}"),
            format!("Disk (max):   {disk}"),
            format!("Created:      {created}"),
            format!("Prepared:     {prepared}"),
            format!("Fingerprint:  {fingerprint}"),
            format!("Agent CLIs:   {versions}"),
        ];
        if self.provision_stale == Some(true) {
            lines.push("Stale:        yes - run agbranch base prepare --rebuild".to_owned());
        }
        if let Some(assets) = self.lima_assets.as_ref() {
            lines.push(format!(
                "Assets:       {}",
                crate::lima::asset_inspect::render_assets_line(assets)
            ));
        }
        lines.join("\n")
    }
}

fn lower_lima_status(status: &str) -> String {
    status.to_ascii_lowercase()
}

fn format_gib(bytes: u64) -> String {
    let gib = bytes as f64 / 1024.0 / 1024.0 / 1024.0;
    if (gib - gib.round()).abs() < 0.05 {
        format!("{:.0} GiB", gib.round())
    } else {
        format!("{gib:.1} GiB")
    }
}

pub(crate) fn format_rfc3339_seconds(timestamp: OffsetDateTime) -> String {
    timestamp
        .replace_nanosecond(0)
        .expect("zero nanoseconds should be valid")
        .format(&Rfc3339)
        .expect("RFC3339 formatting should succeed")
}

fn normalize_rfc3339_to_seconds(value: &str) -> String {
    OffsetDateTime::parse(value, &Rfc3339)
        .map(format_rfc3339_seconds)
        .unwrap_or_else(|_| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lima::inspect::{LimaInstance, LimaInstanceStatus};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn instance(status: &str, protected: bool) -> LimaInstance {
        let mut instance: LimaInstance = serde_json::from_value(json!({
            "name": "agbranch-base-macos",
            "dir": "/tmp/agbranch-base-macos",
            "sshConfigFile": "/tmp/agbranch-base-macos/ssh.config",
            "vmType": "vz",
            "status": status,
            "protected": protected,
            "disk": 107374182400_u64
        }))
        .expect("instance");
        instance.status = match status {
            "Running" => LimaInstanceStatus::Running,
            "Stopped" => LimaInstanceStatus::Stopped,
            _ => LimaInstanceStatus::Other,
        };
        instance
    }

    fn metadata(fingerprint: &str) -> BaseMetadata {
        BaseMetadata {
            schema_version: 1,
            prepared_at: "2026-04-25T04:24:06Z".to_owned(),
            provision_fingerprint: fingerprint.to_owned(),
            provision_fingerprint_source: Some(PROVISION_SOURCE_BAKED_IN.to_owned()),
            agent_cli_versions: BTreeMap::from([
                ("claude".to_owned(), "1.2.3".to_owned()),
                ("codex".to_owned(), "0.9.0".to_owned()),
            ]),
        }
    }

    #[test]
    fn missing_base_summary_is_not_ready() {
        let summary =
            BaseSummary::missing("agbranch-base-macos", NameSource::Default, "sha256:current");

        assert_eq!(summary.status, "missing");
        assert_eq!(summary.readiness_issue(), Some(ReadinessIssue::Missing));
        assert_eq!(
            summary.require_ready_error(),
            "base is missing: run agbranch base prepare"
        );
    }

    #[test]
    fn ready_base_summary_uses_metadata_and_lima_fields() {
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(metadata("sha256:current")),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: Some(4_187_593_114),
            created_at: Some("2026-04-25T04:11:27Z".to_owned()),
        });

        assert_eq!(summary.status, "stopped");
        assert!(summary.protected);
        assert!(summary.prepared);
        assert_eq!(summary.provision_stale, Some(false));
        assert_eq!(summary.readiness_issue(), None);
        assert!(
            summary
                .render_human()
                .contains("Status:       stopped (protected)")
        );
    }

    #[test]
    fn stale_and_unprotected_are_distinct_readiness_issues() {
        let stale = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(metadata("sha256:old")),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });
        let unprotected = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", false)),
            metadata: Some(metadata("sha256:current")),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });

        assert_eq!(stale.readiness_issue(), Some(ReadinessIssue::Stale));
        assert_eq!(
            stale.require_ready_error(),
            "base is stale: run agbranch base prepare --rebuild"
        );
        assert_eq!(
            unprotected.readiness_issue(),
            Some(ReadinessIssue::Unprotected)
        );
        assert_eq!(
            unprotected.require_ready_error(),
            "base is unprotected: run agbranch base prepare"
        );
    }

    #[test]
    fn invalid_metadata_is_not_prepared_and_stale_is_unknown() {
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::EnvOverride,
            instance: Some(instance("Broken", true)),
            metadata: Some(BaseMetadata {
                schema_version: 2,
                ..metadata("sha256:current")
            }),
            metadata_valid: false,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });

        assert_eq!(summary.name_source, "env_override");
        assert_eq!(summary.status, "broken");
        assert!(!summary.prepared);
        assert_eq!(summary.provision_stale, None);
        assert_eq!(
            summary.readiness_issue(),
            Some(ReadinessIssue::MetadataMissing)
        );
    }

    #[test]
    fn base_stamped_under_baked_in_reports_lineage_mismatch_when_inspected_under_override() {
        // Metadata was written under baked-in lineage; current binary is
        // operating under env_override. Even though the stamped fingerprint
        // happens to mean "old baked-in value," the current fingerprint is
        // recomputed from the override tree and the two don't match.
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:baked-in".to_owned(),
                provision_fingerprint_source: Some(PROVISION_SOURCE_BAKED_IN.to_owned()),
                agent_cli_versions: BTreeMap::new(),
            }),
            metadata_valid: true,
            current_fingerprint: "sha256:from-override".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::EnvOverride,
            size_bytes: None,
            created_at: None,
        });

        assert_eq!(summary.provision_stale, Some(true));
        assert_eq!(summary.stale_reason.as_deref(), Some("lineage_mismatch"));
    }

    #[test]
    fn base_stamped_under_override_reports_lineage_mismatch_when_inspected_under_default() {
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:from-override".to_owned(),
                provision_fingerprint_source: Some(PROVISION_SOURCE_ENV_OVERRIDE.to_owned()),
                agent_cli_versions: BTreeMap::new(),
            }),
            metadata_valid: true,
            current_fingerprint: "sha256:baked-in".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });

        assert_eq!(summary.provision_stale, Some(true));
        assert_eq!(summary.stale_reason.as_deref(), Some("lineage_mismatch"));
    }

    #[test]
    fn stale_with_matching_lineage_reports_fingerprint_diverged_not_lineage_mismatch() {
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:old".to_owned(),
                provision_fingerprint_source: Some(PROVISION_SOURCE_BAKED_IN.to_owned()),
                agent_cli_versions: BTreeMap::new(),
            }),
            metadata_valid: true,
            current_fingerprint: "sha256:new".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });

        assert_eq!(summary.provision_stale, Some(true));
        assert_eq!(
            summary.stale_reason.as_deref(),
            Some("fingerprint_diverged")
        );
    }

    fn ready_baseline_summary() -> BaseSummary {
        BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(metadata("sha256:current")),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        })
    }

    fn lima_assets_cache() -> crate::lima::asset_inspect::LimaAssetInspect {
        crate::lima::asset_inspect::LimaAssetInspect {
            state: crate::lima::asset_inspect::LimaAssetInspectState::Cache {
                path: PathBuf::from("/tmp/lima"),
                cache_fingerprint: "sha256:bundle".to_owned(),
            },
            bundle_fingerprint: "sha256:bundle".to_owned(),
        }
    }

    fn lima_assets_would_extract() -> crate::lima::asset_inspect::LimaAssetInspect {
        crate::lima::asset_inspect::LimaAssetInspect {
            state: crate::lima::asset_inspect::LimaAssetInspectState::WouldExtract {
                reason: crate::lima::asset_inspect::CacheWouldExtractReason::Absent,
            },
            bundle_fingerprint: "sha256:bundle".to_owned(),
        }
    }

    fn lima_assets_invalid_override() -> crate::lima::asset_inspect::LimaAssetInspect {
        crate::lima::asset_inspect::LimaAssetInspect {
            state: crate::lima::asset_inspect::LimaAssetInspectState::InvalidOverride {
                path: PathBuf::from("/tmp/fork"),
                reasons: vec![crate::lima::asset_inspect::OverrideInvalidReason::NotADirectory],
            },
            bundle_fingerprint: "sha256:bundle".to_owned(),
        }
    }

    #[test]
    fn require_ready_passes_when_base_ready_and_assets_cache() {
        let mut summary = ready_baseline_summary();
        summary.lima_assets = Some(lima_assets_cache());
        assert_eq!(summary.readiness_issue(), None);
    }

    #[test]
    fn require_ready_fails_when_assets_would_extract_even_if_base_ready() {
        let mut summary = ready_baseline_summary();
        summary.lima_assets = Some(lima_assets_would_extract());
        assert_eq!(
            summary.readiness_issue(),
            Some(ReadinessIssue::LimaAssetsCacheNeedsExtraction)
        );
        assert_eq!(
            summary.require_ready_error(),
            "lima assets cache needs extraction: run any command to extract (or agbranch internal extract-assets)"
        );
    }

    #[test]
    fn require_ready_fails_when_assets_invalid_override_even_if_base_ready() {
        let mut summary = ready_baseline_summary();
        summary.lima_assets = Some(lima_assets_invalid_override());
        assert_eq!(
            summary.readiness_issue(),
            Some(ReadinessIssue::LimaAssetsOverrideInvalid)
        );
        assert_eq!(
            summary.require_ready_error(),
            "lima assets override is invalid: set or fix AGBRANCH_LIMA_ASSETS_DIR (see agbranch base show for details)"
        );
    }

    #[test]
    fn require_ready_base_failures_take_priority_over_asset_failures() {
        // Base is missing AND cache would extract — base failure must win.
        let mut summary =
            BaseSummary::missing("agbranch-base-macos", NameSource::Default, "sha256:current");
        summary.lima_assets = Some(lima_assets_would_extract());
        assert_eq!(summary.readiness_issue(), Some(ReadinessIssue::Missing));
    }

    #[test]
    fn unknown_provision_fingerprint_source_is_invalid_metadata() {
        // Metadata carries an unknown lineage value. This must be treated
        // like an unknown schema_version: prepared = false, no stale reason.
        let summary = BaseSummary::from_parts(BaseSummaryInput {
            name: "agbranch-base-macos".to_owned(),
            name_source: NameSource::Default,
            instance: Some(instance("Stopped", true)),
            metadata: Some(BaseMetadata {
                schema_version: 1,
                prepared_at: "2026-04-25T04:24:06Z".to_owned(),
                provision_fingerprint: "sha256:current".to_owned(),
                provision_fingerprint_source: Some("martian".to_owned()),
                agent_cli_versions: BTreeMap::new(),
            }),
            metadata_valid: true,
            current_fingerprint: "sha256:current".to_owned(),
            current_fingerprint_source: CurrentFingerprintSource::BakedIn,
            size_bytes: None,
            created_at: None,
        });

        assert!(!summary.prepared);
        assert_eq!(
            summary.readiness_issue(),
            Some(ReadinessIssue::MetadataMissing)
        );
    }
}
