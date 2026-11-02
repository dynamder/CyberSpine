//! Parsing a rig manifest.
//!
//! The grammar of a name is [`Name`]'s job, the declaration rules are
//! [`Rig::validate`]'s, and this module only maps JSON onto them.

use cyberspine_rig::{Name, Rig, RigError, RigTable};
use serde::Deserialize;
use thiserror::Error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    rig: Identity,
    muscles: Vec<String>,
    sensors_f: Vec<String>,
    sensors_i: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    name: String,
    version: u32,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("the manifest is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Rig(#[from] RigError),
}

pub fn from_json(text: &str) -> Result<Rig, ManifestError> {
    let manifest: Manifest = serde_json::from_str(text)?;

    let mut rig = Rig::new(manifest.rig.name, manifest.rig.version);
    rig.muscles = names(RigTable::Muscles, manifest.muscles)?;
    rig.sensors_f = names(RigTable::SensorsF, manifest.sensors_f)?;
    rig.sensors_i = names(RigTable::SensorsI, manifest.sensors_i)?;
    rig.validate()?;
    Ok(rig)
}

fn names(table: RigTable, raw: Vec<String>) -> Result<Vec<Name>, RigError> {
    raw.iter()
        .enumerate()
        .map(|(index, name)| {
            Name::new(name).map_err(|_| RigError::invalid_name(table, index, name))
        })
        .collect()
}
