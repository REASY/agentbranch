#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitialAttachTarget {
    Shell,
    Agent,
}

pub fn initial_attach_target(
    has_agent: bool,
    attach: bool,
    detach: bool,
    json: bool,
) -> Option<InitialAttachTarget> {
    if detach || json {
        return None;
    }
    if has_agent {
        return Some(InitialAttachTarget::Agent);
    }
    attach.then_some(InitialAttachTarget::Shell)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_launches_attach_by_default_for_compatibility() {
        assert_eq!(
            initial_attach_target(true, false, false, false),
            Some(InitialAttachTarget::Agent)
        );
    }

    #[test]
    fn explicit_attach_uses_shell_without_an_agent() {
        assert_eq!(
            initial_attach_target(false, true, false, false),
            Some(InitialAttachTarget::Shell)
        );
    }

    #[test]
    fn detach_and_json_never_attach() {
        assert_eq!(initial_attach_target(true, false, true, false), None);
        assert_eq!(initial_attach_target(true, false, false, true), None);
    }

    #[test]
    fn agent_is_the_explicit_attach_target_when_present() {
        assert_eq!(
            initial_attach_target(true, true, false, false),
            Some(InitialAttachTarget::Agent)
        );
    }
}
