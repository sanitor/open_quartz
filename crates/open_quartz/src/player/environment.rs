use crate::execution::runtime::RuntimeCapabilities;

#[derive(Clone, Debug)]
pub struct Environment {
    capabilities: RuntimeCapabilities,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            capabilities: RuntimeCapabilities { data_paths: vec![] },
        }
    }
}

impl Environment {
    pub fn headless() -> Self {
        Self::default()
    }

    pub(crate) fn capabilities(&self) -> RuntimeCapabilities {
        self.capabilities.clone()
    }
}
