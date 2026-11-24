use open_quartz::project::DataType;

use super::Controller;
use crate::clip_pipeline::{ClipPipeline, OUTPUT_ID, SOURCE_ID};
use crate::document::{Clip, ClipTransform};
use crate::{EditorWindow, MonitorBox};

type Point = [f32; 2];

#[derive(Clone, Copy)]
struct Geometry {
    canvas: Point,
    center: Point,
    half: Point,
    angle: f32,
}

fn sample_size(pipeline: &ClipPipeline, asset: Point, canvas: Point) -> Point {
    let graph = pipeline.graph();
    let upstream = |id: &str| {
        let node = graph.nodes.iter().find(|node| node.id == id)?;
        node.data
            .inputs
            .iter()
            .filter(|port| port.data_type == DataType::Sampler2d)
            .find_map(|port| {
                graph
                    .edges
                    .iter()
                    .find(|edge| edge.target == id && edge.target_handle == port.id)
            })
            .map(|edge| edge.source.as_str())
    };
    let mut id = upstream(OUTPUT_ID);
    while let Some(source) = id {
        if source == SOURCE_ID {
            return asset;
        }
        let Some(node) = graph.nodes.iter().find(|node| node.id == source) else {
            break;
        };
        if node.data.auto_size == Some(false) {
            return [
                node.data.width.map_or(canvas[0], |n| n as f32),
                node.data.height.map_or(canvas[1], |n| n as f32),
            ];
        }
        id = upstream(source);
    }
    canvas
}

fn rotate([x, y]: Point, degrees: f32) -> Point {
    let (sin, cos) = degrees.to_radians().sin_cos();
    [cos * x - sin * y, sin * x + cos * y]
}

fn add([x, y]: Point, [dx, dy]: Point) -> Point {
    [x + dx, y + dy]
}
fn sub([x, y]: Point, [dx, dy]: Point) -> Point {
    [x - dx, y - dy]
}
fn mul([x, y]: Point, scale: f32) -> Point {
    [x * scale, y * scale]
}
fn dot([x, y]: Point, [dx, dy]: Point) -> f32 {
    x * dx + y * dy
}

impl Geometry {
    fn from_transform(canvas: Point, source: Point, transform: ClipTransform) -> Self {
        let fit = (canvas[0] / source[0]).min(canvas[1] / source[1]);
        Self {
            canvas,
            center: [
                canvas[0] * (0.5 + transform.x),
                canvas[1] * (0.5 + transform.y),
            ],
            half: mul(source, fit * transform.scale * 0.5),
            angle: transform.rotation,
        }
    }

    fn point(self, local: Point) -> Point {
        add(self.center, rotate(local, self.angle))
    }

    fn contains(self, point: Point) -> bool {
        let local = rotate(sub(point, self.center), -self.angle);
        local[0].abs() <= self.half[0] && local[1].abs() <= self.half[1]
    }

    fn overlay(self, locked: bool) -> MonitorBox {
        let [ax, ay] = self.point([-self.half[0], -self.half[1]]);
        let [bx, by] = self.point([self.half[0], -self.half[1]]);
        let [cx, cy] = self.point([self.half[0], self.half[1]]);
        let [dx, dy] = self.point([-self.half[0], self.half[1]]);
        let [tx, ty] = self.point([0.0, -self.half[1]]);
        MonitorBox {
            ax,
            ay,
            bx,
            by,
            cx,
            cy,
            dx,
            dy,
            tx,
            ty,
            angle: self.angle,
            visible: true,
            locked,
        }
    }
}

enum Handle {
    Move,
    Rotate,
    Corner(Point),
}

pub(super) struct MonitorGesture {
    pub clip: String,
    revision: u64,
    frame: u64,
    origin: Point,
    original: ClipTransform,
    geometry: Geometry,
    handle: Handle,
    pub value: ClipTransform,
}

impl MonitorGesture {
    fn update(&mut self, point: Point, snap: f32) {
        let g = self.geometry;
        let mut value = self.original;
        let delta = sub(point, self.origin);
        match self.handle {
            Handle::Move => {
                let mut center = add(g.center, delta);
                if delta != [0.0, 0.0] {
                    let half_x = rotate([g.half[0], 0.0], g.angle);
                    let half_y = rotate([0.0, g.half[1]], g.angle);
                    for axis in 0..2 {
                        let extent = half_x[axis].abs() + half_y[axis].abs();
                        let targets = [g.canvas[axis] / 2.0, extent, g.canvas[axis] - extent];
                        if let Some(target) = targets
                            .into_iter()
                            .filter(|target| (target - center[axis]).abs() <= snap)
                            .min_by(|a, b| {
                                (a - center[axis])
                                    .abs()
                                    .total_cmp(&(b - center[axis]).abs())
                            })
                        {
                            center[axis] = target;
                        }
                    }
                }
                value.x = (center[0] / g.canvas[0] - 0.5).clamp(-1.0, 1.0);
                value.y = (center[1] / g.canvas[1] - 0.5).clamp(-1.0, 1.0);
            }
            Handle::Rotate => {
                let angle_at = |point: Point| {
                    let [x, y] = sub(point, g.center);
                    y.atan2(x).to_degrees()
                };
                value.rotation = (g.angle + angle_at(point) - angle_at(self.origin) + 180.0)
                    .rem_euclid(360.0)
                    - 180.0;
            }
            Handle::Corner(sign) => {
                let corner = rotate([sign[0] * g.half[0], sign[1] * g.half[1]], g.angle);
                let diagonal = mul(corner, 2.0);
                // Bound the scale so the fixed corner stays fixed even at the
                // document position limits.
                let anchor = sub(g.center, corner);
                let mut minimum: f32 = 0.1 / value.scale;
                let mut maximum: f32 = 4.0 / value.scale;
                for axis in 0..2 {
                    if corner[axis].abs() < f32::EPSILON {
                        continue;
                    }
                    let a = (-0.5 * g.canvas[axis] - anchor[axis]) / corner[axis];
                    let b = (1.5 * g.canvas[axis] - anchor[axis]) / corner[axis];
                    minimum = minimum.max(a.min(b)).min(1.0);
                    maximum = maximum.min(a.max(b)).max(1.0);
                }
                let factor =
                    (1.0 + dot(delta, diagonal) / dot(diagonal, diagonal)).clamp(minimum, maximum);
                let center = add(anchor, mul(corner, factor));
                value.scale = (value.scale * factor).clamp(0.1, 4.0);
                value.x = (center[0] / g.canvas[0] - 0.5).clamp(-1.0, 1.0);
                value.y = (center[1] / g.canvas[1] - 0.5).clamp(-1.0, 1.0);
            }
        }
        self.value = if delta == [0.0, 0.0] {
            self.original
        } else {
            value
        };
    }
}

impl Controller {
    pub(super) fn connect_monitor(
        this: &std::rc::Rc<std::cell::RefCell<Self>>,
        window: &EditorWindow,
    ) {
        let weak = std::rc::Rc::downgrade(this);
        window.on_monitor_gesture(move |phase, kind, x, y, snap| {
            if let Some(controller) = weak.upgrade() {
                controller
                    .borrow_mut()
                    .monitor_event(&phase, &kind, [x, y], snap);
            }
        });
        let weak = std::rc::Rc::downgrade(this);
        window.on_transform_field(move |field, value| {
            if let Some(controller) = weak.upgrade() {
                controller.borrow_mut().set_transform_field(&field, &value);
            }
        });
    }

    fn geometry(&self, clip: &Clip, transform: ClipTransform) -> Option<Geometry> {
        let asset = self.document.asset(&clip.asset_id)?;
        let data = &self.document.data;
        let canvas = [data.width as f32, data.height as f32];
        let source = sample_size(
            &clip.pipeline,
            [asset.width as f32, asset.height as f32],
            canvas,
        );
        Some(Geometry::from_transform(canvas, source, transform))
    }

    fn monitor_ready(&self) -> bool {
        !self.asset_preview
            && !self.playing
            && self.displayed_frame == Some(self.playhead)
            && self.displayed_ticket == Some(self.ticket)
    }

    pub(super) fn refresh_monitor(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let selected = self
            .selected_clip
            .as_deref()
            .and_then(|id| self.document.clip(id));
        let draft = self.monitor_gesture.as_ref();
        let mut overlay = MonitorBox::default();
        let mut writable = false;
        if let Some(clip) = selected {
            let transform = draft.map_or(clip.transform, |draft| draft.value);
            let track = self
                .document
                .clip_track(&clip.id)
                .expect("selected clip has a track");
            writable = !track.locked;
            if track.visible
                && track
                    .clip_at(self.playhead)
                    .is_some_and(|active| active.id == clip.id)
                && (draft.is_some() || self.monitor_ready())
            {
                if let Some(geometry) = self.geometry(clip, transform) {
                    overlay = geometry.overlay(track.locked);
                }
            }
            let format = |v: f32| format!("{v:.3}").into();
            window.set_selected_clip_x(format(transform.x * self.document.data.width as f32));
            window.set_selected_clip_y(format(transform.y * self.document.data.height as f32));
            window.set_selected_clip_scale(format(transform.scale));
            window.set_selected_clip_rotation(format(transform.rotation));
            window.set_selected_clip_opacity(format(transform.opacity * 100.0));
            window.set_selected_clip_exposure(format(transform.exposure));
        }
        window.set_selected_clip_writable(writable);
        window.set_monitor_box(overlay);
    }

    pub(super) fn cancel_monitor(&mut self) {
        if self.monitor_gesture.take().is_some() {
            self.requested = None;
            self.request_preview();
        }
    }

    fn monitor_event(&mut self, phase: &str, kind: &str, point: Point, snap: f32) {
        match phase {
            "cancel" => self.cancel_monitor(),
            "pick" | "start" => {
                if self.playing {
                    self.pause();
                    self.refresh_monitor();
                    return;
                }
                if !self.monitor_ready() {
                    return;
                }
                let handle = match kind {
                    "rotate" => Handle::Rotate,
                    "nw" => Handle::Corner([-1.0, -1.0]),
                    "ne" => Handle::Corner([1.0, -1.0]),
                    "se" => Handle::Corner([1.0, 1.0]),
                    "sw" => Handle::Corner([-1.0, 1.0]),
                    _ => Handle::Move,
                };
                let preferred = if phase == "start" {
                    self.selected_clip
                        .as_deref()
                        .and_then(|id| self.document.clip(id))
                        .filter(|clip| {
                            self.document
                                .data
                                .active_clips(self.playhead)
                                .any(|(_, active)| active.id == clip.id)
                        })
                        .filter(|clip| {
                            !matches!(handle, Handle::Move)
                                || self
                                    .geometry(clip, clip.transform)
                                    .is_some_and(|g| g.contains(point))
                        })
                } else {
                    None
                };
                let clip = preferred
                    .or_else(|| {
                        if point[0] < 0.0
                            || point[1] < 0.0
                            || point[0] > self.document.data.width as f32
                            || point[1] > self.document.data.height as f32
                        {
                            return None;
                        }
                        self.document
                            .data
                            .active_clips(self.playhead)
                            .find_map(|(_, clip)| {
                                (clip.transform.opacity > 0.0
                                    && self
                                        .geometry(clip, clip.transform)
                                        .is_some_and(|g| g.contains(point)))
                                .then_some(clip)
                            })
                    })
                    .cloned();
                let Some(clip) = clip else {
                    return;
                };
                let track = self
                    .document
                    .clip_track(&clip.id)
                    .expect("active clip has a track");
                self.selected_track = track.id.clone();
                self.selected_clip = Some(clip.id.clone());
                self.selected_audio = None;
                self.selected_asset = Some(clip.asset_id.clone());
                if phase == "start" && !track.locked {
                    if let Some(geometry) = self.geometry(&clip, clip.transform) {
                        self.monitor_gesture = Some(MonitorGesture {
                            clip: clip.id,
                            revision: self.document.revision,
                            frame: self.playhead,
                            origin: point,
                            original: clip.transform,
                            geometry,
                            handle,
                            value: clip.transform,
                        });
                    }
                }
                self.refresh();
            }
            "move" | "finish" => {
                let Some(draft) = &mut self.monitor_gesture else {
                    return;
                };
                if draft.revision != self.document.revision || draft.frame != self.playhead {
                    self.cancel_monitor();
                    return;
                }
                let previous = draft.value;
                draft.update(point, snap);
                if phase == "finish" {
                    let draft = self.monitor_gesture.take().expect("active monitor gesture");
                    let result =
                        self.document
                            .set_transform(&draft.clip, draft.value, draft.revision);
                    self.finish_edit(result);
                } else if previous != draft.value {
                    self.requested = None;
                    self.request_preview();
                }
            }
            _ => {}
        }
    }

    fn set_transform_field(&mut self, field: &str, value: &str) {
        let Some(clip) = self
            .selected_clip
            .as_deref()
            .and_then(|id| self.document.clip(id))
        else {
            return;
        };
        let id = clip.id.clone();
        let mut transform = clip.transform;
        let result = (|| {
            if field == "reset" {
                transform = ClipTransform::default();
            } else {
                let value: f32 = value
                    .parse()
                    .map_err(|_| "Enter a numeric transform value")?;
                if !value.is_finite() {
                    return Err("Transform value must be finite".into());
                }
                match field {
                    "x" => transform.x = value / self.document.data.width as f32,
                    "y" => transform.y = value / self.document.data.height as f32,
                    "scale" => transform.scale = value,
                    "rotation" => transform.rotation = value,
                    "opacity" => transform.opacity = value / 100.0,
                    "exposure" => transform.exposure = value,
                    _ => return Err("Unknown transform field".into()),
                }
            }
            self.document
                .set_transform(&id, transform, self.document.revision)
        })();
        self.finish_edit(result);
    }
}
