use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

pub(crate) const MAX_REFERENCES: usize = 8;
const MAX_ENCODED_LEN: usize = 825_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceStill {
    pub id: u64,
    pub label: String,
    pub png: Arc<str>,
}

impl ReferenceStill {
    pub fn validate(&self) -> Result<(), String> {
        if self.id == 0 || self.label.is_empty() || self.png.len() > MAX_ENCODED_LEN {
            return Err("Invalid reference still metadata".into());
        }
        Ok(())
    }

}

pub(crate) fn validate_references(stills: &[ReferenceStill]) -> Result<(), String> {
    if stills.len() > MAX_REFERENCES { return Err("The reference gallery holds up to eight stills".into()); }
    let mut ids = BTreeSet::new();
    for still in stills {
        if !ids.insert(still.id) { return Err("Duplicate reference still ID".into()); }
        still.validate()?;
    }
    Ok(())
}
