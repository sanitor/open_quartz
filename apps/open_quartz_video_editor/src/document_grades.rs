use super::*;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GradeVersions {
    pub active: GradeLabel,
    pub alternatives: Vec<GradeVersion>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GradeLabel {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GradeVersion {
    pub label: GradeLabel,
    pipeline: ClipPipeline,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effect_keys: Vec<EffectKeys>,
}

impl Default for GradeVersions {
    fn default() -> Self {
        Self {
            active: GradeLabel { id: "grade-0".into(), name: "Original".into() },
            alternatives: Vec::new(),
        }
    }
}

impl Clip {
    pub fn grade_options(&self) -> Vec<GradeLabel> {
        let Some(versions) = &self.grade_versions else { return vec![GradeVersions::default().active]; };
        let mut labels: Vec<_> = std::iter::once(&versions.active)
            .chain(versions.alternatives.iter().map(|version| &version.label))
            .cloned().collect();
        labels.sort_by(|a, b| a.id.cmp(&b.id));
        labels
    }

    pub(super) fn validate_grade_versions(&self) -> Result<(), String> {
        let Some(versions) = &self.grade_versions else { return Ok(()); };
        if self.title.is_some() { return Err("Generated text clips do not have grade versions".into()); }
        let mut ids = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for label in self.grade_options() {
            if label.id.is_empty() || !ids.insert(label.id) || label.name.trim().is_empty()
                || label.name.chars().count() > 80 || !names.insert(label.name)
            {
                return Err("Grade versions need distinct nonempty names".into());
            }
        }
        for version in &versions.alternatives {
            version.pipeline.validate()?;
            effect_motion::validate_effect_keys(&version.pipeline, &version.effect_keys)?;
        }
        Ok(())
    }
}

impl EditorDocument {
    pub fn edit_grade(&mut self, id: &str, action: &str, value: &str) -> Result<(), String> {
        let new_id = new_id("grade", self.revision);
        self.transact(|data| {
            let (track, index) = data.writable_clip(id)?;
            let clip = &mut data.tracks[track].clips[index];
            if clip.title.is_some() { return Err("Generated text clips do not have grade versions".into()); }
            let versions = clip.grade_versions.get_or_insert_with(Default::default);
            match action {
                "duplicate" => {
                    let name = (1..).map(|index| format!("Version {index}"))
                        .find(|name| versions.active.name != *name
                            && !versions.alternatives.iter().any(|grade| grade.label.name == *name))
                        .ok_or("No grade names available")?;
                    let label = std::mem::replace(&mut versions.active, GradeLabel { id: new_id, name });
                    versions.alternatives.push(GradeVersion {
                        label, pipeline: clip.pipeline.clone(), effect_keys: clip.effect_keys.clone(),
                    });
                }
                "rename" => versions.active.name = value.trim().to_owned(),
                "select" | "remove" => {
                    if action == "select" && versions.active.id == value { return Ok(()); }
                    let index = if action == "remove" {
                        versions.alternatives.iter().enumerate().min_by(|(_, a), (_, b)| a.label.id.cmp(&b.label.id))
                            .map(|(index, _)| index)
                    } else {
                        versions.alternatives.iter().position(|grade| grade.label.id == value)
                    }.ok_or("Keep at least one grade version")?;
                    let next = versions.alternatives.remove(index);
                    let previous = GradeVersion {
                        label: std::mem::replace(&mut versions.active, next.label),
                        pipeline: std::mem::replace(&mut clip.pipeline, next.pipeline),
                        effect_keys: std::mem::replace(&mut clip.effect_keys, next.effect_keys),
                    };
                    if action == "select" { versions.alternatives.push(previous); }
                }
                _ => return Err("Unknown grade action".into()),
            }
            Ok(())
        })
    }
}
