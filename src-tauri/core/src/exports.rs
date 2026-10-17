//! Reports stay in the owning process; IPC carries a destination and bounded metadata.
use crate::{
    details, diagnostics,
    store::{err, Core, Result},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    History,
    Quarantine,
    Diagnostics,
}

pub(crate) fn write_unlocked(
    core: &Core,
    kind: Kind,
    destination: &Path,
    app_update: Option<serde_json::Value>,
) -> Result<()> {
    let report = match kind {
        Kind::History => serde_json::to_value(core.snapshot().history).map_err(err)?,
        Kind::Quarantine => serde_json::to_value(details::export_unlocked(core)?).map_err(err)?,
        Kind::Diagnostics => {
            let mut report =
                serde_json::to_value(diagnostics::report_unlocked(core)?).map_err(err)?;
            if let Some(status) = app_update {
                report
                    .as_object_mut()
                    .unwrap()
                    .insert("appUpdate".into(), status);
            }
            report
        }
    };
    details::write_export_unlocked(
        core,
        destination,
        &serde_json::to_vec_pretty(&report).map_err(err)?,
    )
}
