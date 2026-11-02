//! Parsing and validating a rig manifest.

use std::collections::HashSet;

use cyberspine_core::rig::Rig;
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
    #[error("the rig name must not be empty")]
    EmptyName,
    #[error("{table}[{index}] {name:?} is not an identifier")]
    NotAnIdentifier {
        table: &'static str,
        index: usize,
        name: String,
    },
    #[error("{table}[{index}] {name:?} is reserved by the language")]
    ReservedName {
        table: &'static str,
        index: usize,
        name: String,
    },
    #[error("{name:?} is declared more than once")]
    DuplicateName { name: String },
}

const RESERVED: [&str; 2] = ["dt", "tick"];

pub fn from_json(text: &str) -> Result<Rig, ManifestError> {
    let manifest: Manifest = serde_json::from_str(text)?;
    if manifest.rig.name.is_empty() {
        return Err(ManifestError::EmptyName);
    }

    let mut seen = HashSet::new();
    Ok(Rig {
        name: manifest.rig.name.into_boxed_str(),
        version: manifest.rig.version,
        muscles: take("muscles", manifest.muscles, &mut seen)?,
        sensors_f: take("sensors_f", manifest.sensors_f, &mut seen)?,
        sensors_i: take("sensors_i", manifest.sensors_i, &mut seen)?,
    })
}

fn take(
    table: &'static str,
    names: Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<Vec<Box<str>>, ManifestError> {
    let mut out = Vec::with_capacity(names.len());
    for (index, name) in names.into_iter().enumerate() {
        if !is_identifier(&name) || is_register_shape(&name) {
            return Err(ManifestError::NotAnIdentifier { table, index, name });
        }
        if RESERVED.contains(&name.as_str()) {
            return Err(ManifestError::ReservedName { table, index, name });
        }
        if !seen.insert(name.clone()) {
            return Err(ManifestError::DuplicateName { name });
        }
        out.push(name.into_boxed_str());
    }
    Ok(out)
}

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// `f0`..`f63` / `i0`..`i63` are register spellings, so they cannot be names.
fn is_register_shape(name: &str) -> bool {
    let mut chars = name.chars();
    if !matches!(chars.next(), Some('f' | 'i')) {
        return false;
    }
    let rest: String = chars.collect();
    !rest.is_empty() && rest.chars().all(|character| character.is_ascii_digit())
}
