use open_quartz_sdk::{
    DataType, FramebufferFormat, InputMode, NodeData, NodeType, TextureFilter, TextureWrap,
    VideoSourceType,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FormKind {
    Sampling,
    Framebuffer,
    Video,
}

impl FormKind {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Sampling => "sampling",
            Self::Framebuffer => "framebuffer",
            Self::Video => "video",
        }
    }
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "sampling" => Some(Self::Sampling),
            "framebuffer" => Some(Self::Framebuffer),
            "video" => Some(Self::Video),
            _ => None,
        }
    }
    pub(crate) fn supports(self, data: &NodeData) -> bool {
        match self {
            Self::Sampling => {
                matches!(data.node_type, NodeType::Shader | NodeType::Constant)
                    || (data.node_type == NodeType::Input
                        && data.input_data_type == Some(DataType::Sampler2d))
            }
            Self::Framebuffer => {
                data.node_type == NodeType::Input && data.input_mode == Some(InputMode::Framebuffer)
            }
            Self::Video => {
                data.node_type == NodeType::Input && data.input_mode == Some(InputMode::Video)
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeFormsDraft {
    pub(crate) filter: String,
    pub(crate) wrap: String,
    pub(crate) fb_format: String,
    pub(crate) fb_width: String,
    pub(crate) fb_height: String,
    pub(crate) fb_stride: String,
    pub(crate) video_source: String,
    pub(crate) video_device: String,
    pub(crate) video_loop: String,
    pub(crate) video_rate: String,
}

impl NodeFormsDraft {
    pub(crate) fn from_data(data: &NodeData) -> Self {
        Self {
            filter: match data.tex_filter.unwrap_or(TextureFilter::Linear) {
                TextureFilter::Linear => "0",
                TextureFilter::Nearest => "1",
            }
            .into(),
            wrap: match data.tex_wrap.unwrap_or(TextureWrap::Clamp) {
                TextureWrap::Clamp => "0",
                TextureWrap::Repeat => "1",
                TextureWrap::Mirror => "2",
            }
            .into(),
            fb_format: match data.fb_format.unwrap_or(FramebufferFormat::Rgba8) {
                FramebufferFormat::Rgba8 => "0",
                FramebufferFormat::Rgba32f => "1",
                FramebufferFormat::Rg8 => "2",
                FramebufferFormat::Rg32f => "3",
                FramebufferFormat::R8 => "4",
                FramebufferFormat::R32f => "5",
                FramebufferFormat::Nv12 => "6",
            }
            .into(),
            fb_width: data.fb_width.map(|v| v.to_string()).unwrap_or_default(),
            fb_height: data.fb_height.map(|v| v.to_string()).unwrap_or_default(),
            fb_stride: data
                .fb_stride
                .filter(|v| *v != 0)
                .map(|v| v.to_string())
                .unwrap_or_default(),
            video_source: if data.video_source_type == Some(VideoSourceType::Camera) {
                "1"
            } else {
                "0"
            }
            .into(),
            video_device: data.video_device_id.clone().unwrap_or_default(),
            video_loop: data.video_loop.unwrap_or(true).to_string(),
            video_rate: data.video_playback_rate.unwrap_or(1.0).to_string(),
        }
    }

    pub(crate) fn change(&mut self, key: &str, value: &str) -> Option<FormKind> {
        let (field, kind) = match key {
            "filter" => (&mut self.filter, FormKind::Sampling),
            "wrap" => (&mut self.wrap, FormKind::Sampling),
            "fb-format" => (&mut self.fb_format, FormKind::Framebuffer),
            "fb-width" => (&mut self.fb_width, FormKind::Framebuffer),
            "fb-height" => (&mut self.fb_height, FormKind::Framebuffer),
            "fb-stride" => (&mut self.fb_stride, FormKind::Framebuffer),
            "video-source" => (&mut self.video_source, FormKind::Video),
            "video-device" => (&mut self.video_device, FormKind::Video),
            "video-loop" => (&mut self.video_loop, FormKind::Video),
            "video-rate" => (&mut self.video_rate, FormKind::Video),
            _ => return None,
        };
        *field = value.into();
        Some(kind)
    }

    pub(crate) fn restore_group(&mut self, data: &NodeData, kind: FormKind) {
        let fresh = Self::from_data(data);
        match kind {
            FormKind::Sampling => {
                self.filter = fresh.filter;
                self.wrap = fresh.wrap;
            }
            FormKind::Framebuffer => {
                self.fb_format = fresh.fb_format;
                self.fb_width = fresh.fb_width;
                self.fb_height = fresh.fb_height;
                self.fb_stride = fresh.fb_stride;
            }
            FormKind::Video => {
                self.video_source = fresh.video_source;
                self.video_device = fresh.video_device;
                self.video_loop = fresh.video_loop;
                self.video_rate = fresh.video_rate;
            }
        }
    }

    fn same_group(&self, other: &Self, kind: FormKind) -> bool {
        match kind {
            FormKind::Sampling => self.filter == other.filter && self.wrap == other.wrap,
            FormKind::Framebuffer => {
                self.fb_format == other.fb_format
                    && self.fb_width == other.fb_width
                    && self.fb_height == other.fb_height
                    && self.fb_stride == other.fb_stride
            }
            FormKind::Video => {
                self.video_source == other.video_source
                    && self.video_device == other.video_device
                    && self.video_loop == other.video_loop
                    && self.video_rate == other.video_rate
            }
        }
    }

    pub(crate) fn dirty(&self, data: &NodeData, kind: FormKind) -> bool {
        kind.supports(data)
            && !self.same_group(&Self::from_data(data), kind)
            && self
                .updated_data(data, kind)
                .map_or(true, |next| next != *data)
    }

    pub(crate) fn updated_data(&self, data: &NodeData, kind: FormKind) -> Result<NodeData, String> {
        if !kind.supports(data) {
            return Err("This node does not support this form".into());
        }
        let original = Self::from_data(data);
        let mut next = data.clone();
        match kind {
            FormKind::Sampling => {
                let filter = match self.filter.as_str() {
                    "0" => TextureFilter::Linear,
                    "1" => TextureFilter::Nearest,
                    _ => return Err("Invalid texture filter".into()),
                };
                let wrap = match self.wrap.as_str() {
                    "0" => TextureWrap::Clamp,
                    "1" => TextureWrap::Repeat,
                    "2" => TextureWrap::Mirror,
                    _ => return Err("Invalid texture wrap".into()),
                };
                if self.filter != original.filter {
                    next.tex_filter = Some(filter);
                }
                if self.wrap != original.wrap {
                    next.tex_wrap = Some(wrap);
                }
            }
            FormKind::Framebuffer => {
                let format = match self.fb_format.as_str() {
                    "0" => FramebufferFormat::Rgba8,
                    "1" => FramebufferFormat::Rgba32f,
                    "2" => FramebufferFormat::Rg8,
                    "3" => FramebufferFormat::Rg32f,
                    "4" => FramebufferFormat::R8,
                    "5" => FramebufferFormat::R32f,
                    "6" => FramebufferFormat::Nv12,
                    _ => return Err("Invalid framebuffer format".into()),
                };
                let dimension = |text: &str| {
                    text.trim()
                        .parse::<u32>()
                        .ok()
                        .filter(|v| (1..=8192).contains(v))
                        .ok_or_else(|| {
                            "Framebuffer dimensions must be integers from 1 to 8192".to_owned()
                        })
                };
                let width = dimension(&self.fb_width)?;
                let height = dimension(&self.fb_height)?;
                let stride = match self.fb_stride.trim() {
                    "" | "0" => None,
                    value => Some(value.parse::<u32>().ok().filter(|v| *v > 0).ok_or(
                        "Stride must be blank, zero (auto), or a positive u32 byte count",
                    )?),
                };
                if self.fb_format != original.fb_format {
                    next.fb_format = Some(format);
                }
                if Some(width) != data.fb_width {
                    next.fb_width = Some(width);
                }
                if Some(height) != data.fb_height {
                    next.fb_height = Some(height);
                }
                if stride != data.fb_stride.filter(|v| *v != 0) {
                    next.fb_stride = stride;
                }
            }
            FormKind::Video => {
                let source = match self.video_source.as_str() {
                    "0" => VideoSourceType::File,
                    "1" => VideoSourceType::Camera,
                    _ => return Err("Invalid video source".into()),
                };
                let looping = self
                    .video_loop
                    .parse::<bool>()
                    .map_err(|_| "Invalid loop setting")?;
                let rate = self
                    .video_rate
                    .trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && (0.1..=4.0).contains(v))
                    .ok_or("Playback rate must be between 0.1 and 4.0")?;
                if self.video_source != original.video_source {
                    next.video_source_type = Some(source);
                }
                if self.video_device != original.video_device {
                    next.video_device_id =
                        (!self.video_device.is_empty()).then(|| self.video_device.clone());
                }
                if self.video_loop != original.video_loop {
                    next.video_loop = Some(looping);
                }
                if rate != data.video_playback_rate.unwrap_or(1.0) {
                    next.video_playback_rate = Some(rate);
                }
            }
        }
        Ok(next)
    }

    pub(crate) fn groups(&self, data: &NodeData) -> Vec<crate::InspectorFormGroup> {
        [FormKind::Sampling, FormKind::Framebuffer, FormKind::Video].into_iter().filter(|kind| kind.supports(data)).map(|kind| {
            let fields = match kind {
                FormKind::Sampling => vec![choice("filter", "Filter", &self.filter, &["Linear", "Nearest"]), choice("wrap", "Wrap", &self.wrap, &["Clamp", "Repeat", "Mirror"])],
                FormKind::Framebuffer => vec![choice("fb-format", "Format", &self.fb_format, &["RGBA8", "RGBA32F", "RG8", "RG32F", "R8", "R32F", "NV12"]), text("fb-width", "Width", &self.fb_width), text("fb-height", "Height", &self.fb_height), text("fb-stride", "Stride bytes/row (blank or 0 = auto)", &self.fb_stride)],
                FormKind::Video => {
                    let mut fields = vec![choice("video-source", "Source", &self.video_source, &["File", "Camera"])];
                    if self.video_source == "1" { fields.push(text("video-device", "Camera device ID (no enumeration)", &self.video_device)); }
                    fields.push(crate::InspectorFormField { key: "video-loop".into(), label: "Loop".into(), value: self.video_loop.as_str().into(), kind: "toggle".into(), checked: self.video_loop == "true", ..Default::default() });
                    fields.push(text("video-rate", "Playback rate (0.1–4.0)", &self.video_rate)); fields
                }
            };
            crate::InspectorFormGroup {
                key: kind.key().into(), title: match kind { FormKind::Sampling => "Sampling", FormKind::Framebuffer => "Framebuffer", FormKind::Video => "Video" }.into(),
                hint: match kind { FormKind::Sampling => "Output sampling settings", FormKind::Framebuffer => "Configuration only · raw resources are not loaded by this shell", FormKind::Video => "Configuration only · video/camera resources are not opened by this shell" }.into(),
                fields: slint::ModelRc::new(slint::VecModel::from(fields)), dirty: self.dirty(data, kind), error: self.updated_data(data, kind).err().unwrap_or_default().into(),
            }
        }).collect()
    }
}

fn text(key: &str, label: &str, value: &str) -> crate::InspectorFormField {
    crate::InspectorFormField {
        key: key.into(),
        label: label.into(),
        value: value.into(),
        kind: "text".into(),
        ..Default::default()
    }
}
fn choice(key: &str, label: &str, value: &str, options: &[&str]) -> crate::InspectorFormField {
    crate::InspectorFormField {
        key: key.into(),
        label: label.into(),
        value: value.into(),
        choice: value.parse().unwrap_or(-1),
        kind: "choice".into(),
        options: slint::ModelRc::new(slint::VecModel::from(
            options
                .iter()
                .map(|v| (*v).into())
                .collect::<Vec<slint::SharedString>>(),
        )),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(mode: InputMode) -> NodeData {
        NodeData {
            node_type: NodeType::Input,
            input_mode: Some(mode),
            input_data_type: Some(DataType::Sampler2d),
            ..Default::default()
        }
    }

    #[test]
    fn sampling_changes_only_edited_fields_and_effective_defaults_are_noops() {
        let data = NodeData {
            node_type: NodeType::Shader,
            ..Default::default()
        };
        let mut draft = NodeFormsDraft::from_data(&data);
        assert!(!draft.dirty(&data, FormKind::Sampling));
        assert_eq!(draft.updated_data(&data, FormKind::Sampling).unwrap(), data);
        draft.wrap = "2".into();
        let next = draft.updated_data(&data, FormKind::Sampling).unwrap();
        assert_eq!(next.tex_wrap, Some(TextureWrap::Mirror));
        assert_eq!(next.tex_filter, None);
        draft.filter = "1".into();
        assert_eq!(
            draft
                .updated_data(&data, FormKind::Sampling)
                .unwrap()
                .tex_filter,
            Some(TextureFilter::Nearest)
        );
        draft.wrap = "9".into();
        assert!(draft.updated_data(&data, FormKind::Sampling).is_err());
        assert!(draft.dirty(&data, FormKind::Sampling));
    }

    #[test]
    fn framebuffer_requires_exact_dimensions_and_optional_u32_stride() {
        let mut data = input(InputMode::Framebuffer);
        data.raw_data_url = Some("data:application/octet-stream;base64,AQIDBA==".into());
        let mut draft = NodeFormsDraft::from_data(&data);
        assert!(!draft.dirty(&data, FormKind::Framebuffer));
        assert!(draft.updated_data(&data, FormKind::Framebuffer).is_err());
        for text in ["0", "8193", "-1", "1.5", "4294967296", ""] {
            draft.fb_width = text.into();
            draft.fb_height = "2".into();
            assert!(
                draft.updated_data(&data, FormKind::Framebuffer).is_err(),
                "{text}"
            );
        }
        draft.fb_width = "8192".into();
        draft.fb_height = "1".into();
        for (index, expected) in [
            FramebufferFormat::Rgba8,
            FramebufferFormat::Rgba32f,
            FramebufferFormat::Rg8,
            FramebufferFormat::Rg32f,
            FramebufferFormat::R8,
            FramebufferFormat::R32f,
            FramebufferFormat::Nv12,
        ]
        .into_iter()
        .enumerate()
        {
            draft.fb_format = index.to_string();
            let next = draft.updated_data(&data, FormKind::Framebuffer).unwrap();
            assert_eq!(next.fb_format.unwrap_or(FramebufferFormat::Rgba8), expected);
            assert_eq!(next.raw_data_url, data.raw_data_url);
        }
        for text in ["-1", "0.5", "4294967296", "NaN"] {
            draft.fb_stride = text.into();
            assert!(draft.updated_data(&data, FormKind::Framebuffer).is_err());
        }
        draft.fb_stride = u32::MAX.to_string();
        assert_eq!(
            draft
                .updated_data(&data, FormKind::Framebuffer)
                .unwrap()
                .fb_stride,
            Some(u32::MAX)
        );
        data.fb_stride = Some(8);
        draft.fb_stride = "0".into();
        assert_eq!(
            draft
                .updated_data(&data, FormKind::Framebuffer)
                .unwrap()
                .fb_stride,
            None
        );
    }

    #[test]
    fn video_validates_native_rate_and_preserves_file_and_url_across_source_changes() {
        let mut data = input(InputMode::Video);
        data.video_file_path = Some("/example/movie.mp4".into());
        data.video_url = Some("blob:retained".into());
        let mut draft = NodeFormsDraft::from_data(&data);
        assert_eq!(draft.updated_data(&data, FormKind::Video).unwrap(), data);
        for text in ["0", "0.09", "4.01", "NaN", "inf", ""] {
            draft.video_rate = text.into();
            assert!(
                draft.updated_data(&data, FormKind::Video).is_err(),
                "{text}"
            );
        }
        for rate in ["0.1", "4"] {
            draft.video_rate = rate.into();
            draft.video_source = "1".into();
            draft.video_device = "camera-id".into();
            draft.video_loop = "false".into();
            let next = draft.updated_data(&data, FormKind::Video).unwrap();
            assert_eq!(next.video_source_type, Some(VideoSourceType::Camera));
            assert_eq!(next.video_loop, Some(false));
            assert_eq!(next.video_device_id.as_deref(), Some("camera-id"));
            assert_eq!(next.video_file_path, data.video_file_path);
            assert_eq!(next.video_url, data.video_url);
        }
        data.video_device_id = Some("old".into());
        draft.video_device.clear();
        assert_eq!(
            draft
                .updated_data(&data, FormKind::Video)
                .unwrap()
                .video_device_id,
            None
        );
    }

    #[test]
    fn unsupported_nodes_reject_forms_and_group_restore_preserves_other_drafts() {
        let mut data = input(InputMode::Video);
        let mut draft = NodeFormsDraft::from_data(&data);
        draft.filter = "1".into();
        draft.video_rate = "2.5".into();
        draft.restore_group(&data, FormKind::Sampling);
        assert_eq!(draft.video_rate, "2.5");
        assert_eq!(draft.filter, "0");
        assert!(draft.updated_data(&data, FormKind::Framebuffer).is_err());
        data.node_type = NodeType::Renderer;
        for kind in [FormKind::Sampling, FormKind::Framebuffer, FormKind::Video] {
            assert!(!draft.dirty(&data, kind));
            assert!(draft.updated_data(&data, kind).is_err());
        }
    }
}
