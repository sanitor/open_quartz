use crate::player::PlaybackCapabilities;

#[derive(Clone, Debug)]
pub struct Environment {
    capabilities: PlaybackCapabilities,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            capabilities: PlaybackCapabilities { data_paths: vec![] },
        }
    }
}

impl Environment {
    pub fn headless() -> Self {
        Self::default()
    }

    pub(crate) fn capabilities(&self) -> PlaybackCapabilities {
        self.capabilities.clone()
    }
}
