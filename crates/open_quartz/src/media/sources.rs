use std::collections::{HashMap, HashSet};

use super::native::{
    NativeVideoConfig, NativeVideoFrame, NativeVideoInfo, NativeVideoMetrics, NativeVideoSource,
};

/// Owns native decoders and keeps each borrowed frame alive throughout its upload.
#[derive(Default)]
pub struct VideoSources {
    sources: HashMap<String, NativeVideoSource>,
}

pub struct PreparedVideoSource {
    source: NativeVideoSource,
}

impl PreparedVideoSource {
    pub fn open(config: NativeVideoConfig, preview_only: bool) -> Result<Self, String> {
        let source = NativeVideoSource::open_hardware_required(config, preview_only)?;
        Ok(Self { source })
    }

    pub fn info(&self) -> &NativeVideoInfo {
        self.source.info()
    }
}

impl VideoSources {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn attach(
        &mut self,
        node_id: &str,
        config: NativeVideoConfig,
        preview_only: bool,
    ) -> Result<NativeVideoInfo, String> {
        let prepared = PreparedVideoSource::open(config, preview_only)?;
        Ok(self.attach_prepared(node_id, prepared))
    }

    pub fn attach_prepared(&mut self, node_id: &str, prepared: PreparedVideoSource) -> NativeVideoInfo {
        let info = prepared.source.info().clone();
        self.sources.insert(node_id.to_owned(), prepared.source);
        info
    }

    pub fn detach(&mut self, node_id: &str) {
        self.sources.remove(node_id);
    }

    pub fn retain(&mut self, node_ids: &HashSet<String>) {
        self.sources.retain(|node_id, _| node_ids.contains(node_id));
    }

    pub fn clear(&mut self) {
        self.sources.clear();
    }

    pub fn node_ids(&self) -> Vec<String> {
        self.sources.keys().cloned().collect()
    }

    /// The callback runs while the source holds its frame-slot borrow. GPU consumers
    /// must retain native surfaces until their submitted work completes.
    pub fn upload_frames(
        &mut self,
        mut upload: impl for<'frame> FnMut(
            &str,
            NativeVideoFrame<'frame>,
            u32,
            u32,
        ) -> Result<(), String>,
    ) -> Result<Vec<String>, String> {
        let mut dirty = Vec::new();
        for (node_id, source) in &mut self.sources {
            if source.upload_latest(&mut |frame, width, height| {
                if matches!(&frame, NativeVideoFrame::Rgba(_)) {
                    return Err(format!("Hardware-resident video is required for node {node_id}; CPU RGBA upload is disabled"));
                }
                upload(node_id, frame, width, height)
            })? {
                dirty.push(node_id.clone());
            }
        }
        Ok(dirty)
    }

    pub fn pause(&mut self) {
        for source in self.sources.values_mut() {
            source.pause();
        }
    }

    pub fn resume(&mut self) -> Result<(), String> {
        for source in self.sources.values_mut() {
            source.resume()?;
        }
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.sources.values().any(NativeVideoSource::is_active)
    }

    pub fn metrics(&self, node_id: &str) -> Result<NativeVideoMetrics, String> {
        self.sources
            .get(node_id)
            .map(NativeVideoSource::metrics)
            .ok_or_else(|| format!("Video node {node_id} is not attached"))
    }

    pub fn frame_counts(&self) -> (u64, u64) {
        self.sources.values().fold((0_u64, 0_u64), |(decoded, uploaded), source| {
            let metrics = source.metrics();
            (
                decoded.saturating_add(metrics.decoded_frames),
                uploaded.saturating_add(metrics.uploaded_frames),
            )
        })
    }
}

#[cfg(test)]
#[path = "sources_coverage_tests.rs"]
mod tests;
