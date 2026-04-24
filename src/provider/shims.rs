use crate::provider::registry::provider_spec;
use crate::types::ProviderKind;

pub fn render_provider_shim(kind: ProviderKind) -> String {
    let spec = provider_spec(kind);
    let unrestricted = spec.unrestricted_args.join(" ");
    let provider = kind.as_str();
    format!(
        r#"#!/bin/bash
set -euo pipefail

strip_path_entry() {{
  local needle="$1"
  local path_value="$2"
  local part
  local out=()
  local IFS=':'
  read -r -a parts <<< "$path_value"
  for part in "${{parts[@]}}"; do
    if [ -n "$part" ] && [ "$part" != "$needle" ]; then
      out+=("$part")
    fi
  done
  (
    IFS=:
    printf '%s' "${{out[*]}}"
  )
}}

export AGBRANCH_SESSION="${{AGBRANCH_SESSION:?AGBRANCH_SESSION must be set}}"
source "$HOME/.agbranch/shellenv.sh"
AUTH_FILE="$HOME/.agbranch/secrets/${{AGBRANCH_SESSION}}/agent.env"
if [ -f "$AUTH_FILE" ]; then
  set -a
  . "$AUTH_FILE"
  set +a
fi
PATH="$(strip_path_entry "$HOME/.agbranch/bin" "$PATH")"
export PATH

run_provider() {{
  if [ "$#" -eq 0 ]; then
    {binary} {unrestricted}
  else
    {binary} "$@"
  fi
}}

TMUX_WINDOW_TARGET=""
if [ -n "${{TMUX:-}}" ]; then
  TMUX_WINDOW_TARGET="$(tmux display-message -p '#S:#I' 2>/dev/null || true)"
  if [ -n "$TMUX_WINDOW_TARGET" ]; then
    tmux set-window-option -t "$TMUX_WINDOW_TARGET" @agbranch_provider "{provider}" >/dev/null 2>&1 || true
  fi
fi

cleanup_tmux_window() {{
  if [ -n "$TMUX_WINDOW_TARGET" ]; then
    tmux set-window-option -u -t "$TMUX_WINDOW_TARGET" @agbranch_provider >/dev/null 2>&1 || true
  fi
}}

trap cleanup_tmux_window EXIT

run_provider "$@"
"#,
        binary = spec.binary_name,
        unrestricted = unrestricted,
        provider = provider,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_shim_bootstraps_shellenv_auth_and_executes_unrestricted_when_no_args() {
        let script = render_provider_shim(ProviderKind::Claude);

        assert!(script.contains("source \"$HOME/.agbranch/shellenv.sh\""));
        assert!(
            script.contains("AUTH_FILE=\"$HOME/.agbranch/secrets/${AGBRANCH_SESSION}/agent.env\"")
        );
        assert!(script.contains("if [ -f \"$AUTH_FILE\" ]; then"));
        assert!(script.contains("PATH=\"$(strip_path_entry \"$HOME/.agbranch/bin\" \"$PATH\")\""));
        assert!(script.contains("run_provider() {"));
        assert!(script.contains("claude --dangerously-skip-permissions"));
        assert!(script.contains("claude \"$@\""));
        assert!(script.contains("@agbranch_provider \"claude\""));
        assert!(script.contains("trap cleanup_tmux_window EXIT"));
        assert!(script.contains("run_provider \"$@\""));
    }

    #[test]
    fn provider_shim_uses_provider_specific_unrestricted_defaults() {
        let codex = render_provider_shim(ProviderKind::Codex);
        assert!(codex.contains("codex --dangerously-bypass-approvals-and-sandbox"));

        let script = render_provider_shim(ProviderKind::Gemini);
        assert!(script.contains("gemini --approval-mode=yolo"));
    }
}
