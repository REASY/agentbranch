use crate::platform::detect::HostPlatform;
use crate::types::{SessionName, VmName};

pub fn base_instance_name() -> VmName {
    VmName::new("agbranch-base")
}

pub fn session_vm_name(session: &SessionName) -> VmName {
    VmName::for_session(session)
}

pub fn prepared_base_name_from_override(
    platform: HostPlatform,
    override_name: Option<&str>,
) -> VmName {
    if let Some(name) = override_name {
        return VmName::new(name);
    }

    match platform {
        HostPlatform::Macos => VmName::new("agbranch-base-macos"),
        HostPlatform::Linux => VmName::new("agbranch-base-linux"),
    }
}

pub fn prepared_base_name(platform: HostPlatform) -> VmName {
    let override_name = std::env::var("AGBRANCH_PREPARED_BASE_NAME").ok();
    prepared_base_name_from_override(platform, override_name.as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_base_override_wins_over_platform_default() {
        let vm = prepared_base_name_from_override(HostPlatform::Macos, Some("agbranch-smoke-base"));
        assert_eq!(vm.as_str(), "agbranch-smoke-base");
    }
}
