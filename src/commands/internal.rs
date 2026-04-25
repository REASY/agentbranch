//! Hidden troubleshooting commands. See `agbranch internal --help`.
//!
//! These commands are not part of the user-facing CLI surface. The
//! `extract-assets` subcommand is the one concrete tool: it forces
//! extraction of the embedded Lima bundle into the state-root cache
//! without calling Lima, so the standalone-binary E2E test can prove the
//! resolver works after the build tree is gone.

use crate::cli::{ExtractAssetsArgs, InternalAction, InternalArgs};
use crate::error::AppError;
use crate::lima::asset_resolver::{LimaAssetOrigin, lima_asset_dir};
use crate::platform::host::HostContext;
use serde::Serialize;

pub fn run(args: InternalArgs) -> Result<(), AppError> {
    match args.action {
        InternalAction::ExtractAssets(args) => extract_assets(args),
    }
}

#[derive(Debug, Serialize)]
struct ExtractAssetsReport {
    path: String,
    origin: &'static str,
    extracted_this_call: bool,
}

fn extract_assets(args: ExtractAssetsArgs) -> Result<(), AppError> {
    let host = HostContext::detect()?;
    let resolved = lima_asset_dir(&host)?;
    let (origin, extracted_this_call) = match resolved.origin {
        LimaAssetOrigin::EnvOverride => ("env_override", false),
        LimaAssetOrigin::StateRootCache {
            extracted_this_call,
        } => ("state_root_cache", extracted_this_call),
    };
    let report = ExtractAssetsReport {
        path: resolved.path.display().to_string(),
        origin,
        extracted_this_call,
    };
    if args.json {
        println!(
            "{}",
            serde_json::to_string(&report)
                .map_err(crate::error::observability::ObservabilityError::Json)?
        );
    } else if extracted_this_call {
        println!("extracted lima assets to {}", report.path);
    } else {
        println!("lima assets already present at {}", report.path);
    }
    Ok(())
}
