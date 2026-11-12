//! GPU-only image ingress and presentation around a native Metal graph.
use std::{ffi::{c_char, c_void, CStr}, ptr::NonNull, sync::Arc, time::Instant};
use crate::{catalog_model::CatalogModel, gpu::{GpuCounters, GpuDeviceOwner, GpuError, OutputLease}, gpu_model::ModelPlan};

#[derive(Clone, Copy, Debug)]
pub struct CatalogOptions { pub score_threshold: f32, pub iou_threshold: f32 }
impl Default for CatalogOptions { fn default() -> Self { Self { score_threshold: 0.25, iou_threshold: 0.45 } } }

#[cfg(target_os="macos")]
unsafe extern "C" {
    fn oq_catalog_graph_create(device:*mut c_void,json:*const u8,json_len:usize,blob:*const u8,blob_len:usize,error:*mut c_char,cap:usize)->*mut c_void;
    fn oq_catalog_graph_release(graph:*mut c_void);
    fn oq_catalog_buffer_create(device:*mut c_void,bytes:usize,error:*mut c_char,cap:usize)->*mut c_void;
    fn oq_catalog_buffer_release(buffer:*mut c_void);
    fn oq_catalog_graph_encode(graph:*mut c_void,queue:*mut c_void,input:*mut c_void,output:*mut c_void,diagnostic:*mut c_void,error:*mut c_char,cap:usize)->i32;
}
fn native_error(error:&[c_char])->GpuError {
    // Native ABI always terminates its fixed-capacity error string.
    GpuError::Validation(unsafe{CStr::from_ptr(error.as_ptr())}.to_string_lossy().into_owned())
}
struct NativeGraph(NonNull<c_void>);
// Graph execution is serialized by &mut CatalogGpuSession on the render thread.
unsafe impl Send for NativeGraph {}
impl NativeGraph {
    #[cfg(target_os="macos")]
    fn new(device:&wgpu::Device,model:&CatalogModel)->Result<Self,GpuError> {
        let native=unsafe{device.as_hal::<wgpu::hal::api::Metal>()}.ok_or(GpuError::WrongDevice)?;
        let raw=objc2::rc::Retained::as_ptr(native.raw_device()) as *mut c_void;
        let mut error=[0 as c_char;4096];
        let graph=unsafe{oq_catalog_graph_create(raw,model.graph_json.as_ptr(),model.graph_json.len(),model.tensor_blob.as_ptr(),model.tensor_blob.len(),error.as_mut_ptr(),error.len())};
        NonNull::new(graph).map(Self).ok_or_else(||native_error(&error))
    }
    #[cfg(not(target_os="macos"))]
    fn new(_: &wgpu::Device,_:&CatalogModel)->Result<Self,GpuError>{Err(GpuError::UnsupportedBackend("catalog residency requires native Metal on macOS".into()))}
    #[cfg(target_os="macos")]
    fn encode(&mut self,queue:&wgpu::Queue,input:&NativeBuffer,output:&NativeBuffer,diagnostic:Option<&NativeBuffer>)->Result<(),GpuError>{
        let native=unsafe{queue.as_hal::<wgpu::hal::api::Metal>()}.ok_or(GpuError::WrongDevice)?;
        let raw=native.as_raw() as *const _ as *mut c_void;
        let mut error=[0 as c_char;4096];
        let result=unsafe{oq_catalog_graph_encode(self.0.as_ptr(),raw,input.raw.as_ptr(),output.raw.as_ptr(),diagnostic.map_or(std::ptr::null_mut(),|b|b.raw.as_ptr()),error.as_mut_ptr(),error.len())};
        if result==0 {Err(native_error(&error))}else{Ok(())}
    }
    #[cfg(not(target_os="macos"))]
    fn encode(&mut self,_:&wgpu::Queue,_:&NativeBuffer,_:&NativeBuffer,_:Option<&NativeBuffer>)->Result<(),GpuError>{Err(GpuError::UnsupportedBackend("catalog residency requires Metal".into()))}
}
impl Drop for NativeGraph {fn drop(&mut self){#[cfg(target_os="macos")]unsafe{oq_catalog_graph_release(self.0.as_ptr())}}}
struct NativeBuffer {raw:NonNull<c_void>, buffer:Arc<wgpu::Buffer>}
// Immutable Objective-C handles; all mutable payload access is GPU command work.
unsafe impl Send for NativeBuffer {} unsafe impl Sync for NativeBuffer {}
impl NativeBuffer {
    #[cfg(target_os="macos")]
    fn new(device:&wgpu::Device,bytes:u64)->Result<Self,GpuError>{
        if bytes==0 || bytes>device.limits().max_buffer_size || bytes>u64::from(device.limits().max_storage_buffer_binding_size){return Err(GpuError::DeviceLimit(format!("resident tensor needs {bytes} bytes; request sufficient adapter storage/buffer limits")));}
        let native=unsafe{device.as_hal::<wgpu::hal::api::Metal>()}.ok_or(GpuError::WrongDevice)?;
        let raw_device=objc2::rc::Retained::as_ptr(native.raw_device()) as *mut c_void;
        let mut error=[0 as c_char;4096];
        let raw=NonNull::new(unsafe{oq_catalog_buffer_create(raw_device,bytes as usize,error.as_mut_ptr(),error.len())}).ok_or_else(||native_error(&error))?;
        let retained=unsafe{objc2::rc::Retained::<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLBuffer>>::retain(raw.as_ptr().cast())}.expect("native returned a retained Metal buffer");
        let hal=unsafe{wgpu::hal::metal::Device::buffer_from_raw(retained,bytes)};
        let buffer=unsafe{device.create_buffer_from_hal::<wgpu::hal::api::Metal>(hal,&wgpu::BufferDescriptor{label:Some("private native graph tensor"),size:bytes,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false})};
        Ok(Self{raw,buffer:Arc::new(buffer)})
    }
    #[cfg(not(target_os="macos"))]
    fn new(_:&wgpu::Device,_:u64)->Result<Self,GpuError>{Err(GpuError::UnsupportedBackend("catalog residency requires Metal".into()))}
}
impl Drop for NativeBuffer{fn drop(&mut self){#[cfg(target_os="macos")]unsafe{oq_catalog_buffer_release(self.raw.as_ptr())}}}

struct Slot {
    texture:Arc<wgpu::Texture>,view:Arc<wgpu::TextureView>,raw:NativeBuffer,
    depth:wgpu::Buffer,depth_range:wgpu::Buffer,reduce_binding:wgpu::BindGroup,reduce_final_binding:wgpu::BindGroup,
    draw_binding:Option<wgpu::BindGroup>,detector:Option<crate::catalog_detect::DetectionSlot>,
    width:u32,height:u32, _frame_lease: Option<Arc<dyn std::any::Any+Send+Sync>>,
    #[cfg(feature = "gpu-diagnostics")]
    logits:Option<NativeBuffer>,
}
#[derive(Clone)]
pub struct CatalogGpuOutput {slot:Arc<OutputLease<Slot>>,submission:wgpu::SubmissionIndex}
impl CatalogGpuOutput {
    pub fn texture(&self)->&wgpu::Texture{&self.slot.texture}
    pub fn texture_arc(&self)->Arc<wgpu::Texture>{self.slot.texture.clone()}
    pub fn view_arc(&self)->Arc<wgpu::TextureView>{self.slot.view.clone()}
    pub fn lease(&self)->Arc<dyn std::any::Any+Send+Sync>{self.slot.clone()}
    pub fn width(&self)->u32{self.slot.width} pub fn height(&self)->u32{self.slot.height}
    pub fn submission_index(&self)->&wgpu::SubmissionIndex{&self.submission}
    /// Original selected tensor, exported as FP32 on GPU (including UInt8 class IDs).
    pub fn raw_result_buffer(&self)->Arc<wgpu::Buffer>{self.slot.raw.buffer.clone()}
    /// Explicit diagnostic only; production depth presentation stays on the GPU.
    pub fn depth_range_buffer(&self)->Option<Arc<wgpu::Buffer>> {
        (self.slot.width>0).then(||Arc::new(self.slot.depth_range.clone()))
    }
    /// Header [count,0,0,0], then count 32-byte xyxy/score/class/index/reserved records.
    pub fn depth_partial_buffer(&self)->Option<Arc<wgpu::Buffer>> { (self.slot.width>0).then(||Arc::new(self.slot.depth.clone())) }
    pub fn detection_buffer(&self)->Option<Arc<wgpu::Buffer>>{self.slot.detector.as_ref().map(|d|d.buffer.clone())}
    #[cfg(feature = "gpu-diagnostics")]
    pub fn diagnostic_logits_buffer(&self)->Option<Arc<wgpu::Buffer>>{self.slot.logits.as_ref().map(|b|b.buffer.clone())}
}
struct Binding {texture:wgpu::Texture,output:wgpu::Texture,pre:wgpu::BindGroup,post:wgpu::BindGroup}
pub struct CatalogGpuSession {
    owner:GpuDeviceOwner,model:CatalogModel,native:NativeGraph,input:NativeBuffer,
    options:CatalogOptions,kind:u32,parameters:wgpu::Buffer,source_size:(u32,u32),
    preprocess:wgpu::ComputePipeline,postprocess:wgpu::ComputePipeline,
    reduce:wgpu::ComputePipeline,reduce_final:wgpu::ComputePipeline,reduce_layout:wgpu::BindGroupLayout,
    detector:Option<crate::catalog_detect::Detector>,draw_boxes:Option<wgpu::RenderPipeline>,slots:Vec<Arc<Slot>>,bindings:Vec<Binding>,validated:Vec<wgpu::Texture>,counters:GpuCounters,
    frame_allocator:Option<Arc<dyn crate::gpu::GpuFrameAllocator>>,
    persistent_frame_lease:Option<Arc<dyn std::any::Any+Send+Sync>>,
    prepared_frame_lease:[Option<(u32,u32,Option<Arc<dyn std::any::Any+Send+Sync>>)>;2],
    output_waker:Option<Arc<dyn Fn()+Send+Sync>>,
}
fn pipeline(device:&wgpu::Device,source:&str,entry:&str)->wgpu::ComputePipeline {
    let module=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("resident catalog image operation"),source:wgpu::ShaderSource::Wgsl(source.into())});
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some(entry),layout:None,module:&module,entry_point:Some(entry),compilation_options:Default::default(),cache:None})
}
fn entry(binding:u32,buffer:&wgpu::Buffer)->wgpu::BindGroupEntry<'_>{wgpu::BindGroupEntry{binding,resource:buffer.as_entire_binding()}}
fn buffer_bytes(shape:&[usize])->Result<u64,GpuError>{shape.iter().try_fold(4u64,|n,&v|n.checked_mul(v as u64)).filter(|n|*n>0).ok_or_else(||GpuError::InvalidImage("resident tensor shape overflow".into()))}
impl CatalogGpuSession {
    pub fn prepare(device:Arc<wgpu::Device>,queue:Arc<wgpu::Queue>,model:CatalogModel,options:CatalogOptions)->Result<Self,GpuError>{
        if device.adapter_info().backend!=wgpu::Backend::Metal||device.adapter_info().device_type==wgpu::DeviceType::Cpu{return Err(GpuError::UnsupportedBackend("catalog graph requires native hardware Metal".into()));}
        if !options.score_threshold.is_finite()||!options.iou_threshold.is_finite()||!(0.0..=1.0).contains(&options.score_threshold)||!(0.0..=1.0).contains(&options.iou_threshold){return Err(GpuError::InvalidImage("invalid detection thresholds".into()));}
        let native=NativeGraph::new(&device,&model)?;
        let input=NativeBuffer::new(&device,buffer_bytes(&model.input_shape)?)?;
        let kind=match model.model_id.as_str(){"realesrgan-x4"=>0,"u2netp"|"modnet"=>1,"midas-small"=>2,"yolo26n-sem"=>3,"yolov8n"=>4,_=>return Err(GpuError::UnsupportedModel(model.model_id))};
        let preprocess=pipeline(&device,include_str!("gpu/catalog_preprocess.wgsl"),"main");
        let postprocess=pipeline(&device,include_str!("gpu/catalog_postprocess.wgsl"),"main");
        let reduce_shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("resident depth range"),source:wgpu::ShaderSource::Wgsl(include_str!("gpu/catalog_reduce.wgsl").into())});
        let reduce_layout=device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor{label:Some("resident depth range layout"),entries:&[
            wgpu::BindGroupLayoutEntry{binding:0,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:true},has_dynamic_offset:false,min_binding_size:None},count:None},
            wgpu::BindGroupLayoutEntry{binding:1,visibility:wgpu::ShaderStages::COMPUTE,ty:wgpu::BindingType::Buffer{ty:wgpu::BufferBindingType::Storage{read_only:false},has_dynamic_offset:false,min_binding_size:None},count:None}]});
        let reduce_pipeline_layout=device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor{label:Some("resident depth range pipeline layout"),bind_group_layouts:&[Some(&reduce_layout)],immediate_size:0});
        let reduce=device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("resident depth partial"),layout:Some(&reduce_pipeline_layout),module:&reduce_shader,entry_point:Some("partial"),compilation_options:Default::default(),cache:None});
        let reduce_final=device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor{label:Some("resident depth final"),layout:Some(&reduce_pipeline_layout),module:&reduce_shader,entry_point:Some("finish"),compilation_options:Default::default(),cache:None});
        let parameters=device.create_buffer(&wgpu::BufferDescriptor{label:Some("resident image scalar controls"),size:48,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
        let detector=if kind==4{Some(crate::catalog_detect::Detector::new(&device))}else{None};
        let draw_boxes=if kind==4 {
            let shader=device.create_shader_module(wgpu::ShaderModuleDescriptor{label:Some("resident YOLO boxes"),source:wgpu::ShaderSource::Wgsl(include_str!("gpu/catalog_draw_boxes.wgsl").into())});
            Some(device.create_render_pipeline(&wgpu::RenderPipelineDescriptor{label:Some("resident YOLO box renderer"),layout:None,vertex:wgpu::VertexState{module:&shader,entry_point:Some("vs"),compilation_options:Default::default(),buffers:&[]},primitive:Default::default(),depth_stencil:None,multisample:Default::default(),fragment:Some(wgpu::FragmentState{module:&shader,entry_point:Some("fs"),compilation_options:Default::default(),targets:&[Some(wgpu::ColorTargetState{format:wgpu::TextureFormat::Rgba8Unorm,blend:None,write_mask:wgpu::ColorWrites::ALL})]}),multiview_mask:None,cache:None}))
        } else {None};
        let counters=GpuCounters{weight_upload_bytes:model.tensor_blob.len()as u64,scratch_bytes:buffer_bytes(&model.input_shape)?,..Default::default()};
        Ok(Self{owner:GpuDeviceOwner{device,queue},model,native,input,options,kind,parameters,source_size:(0,0),preprocess,postprocess,reduce,reduce_final,reduce_layout,detector,draw_boxes,slots:Vec::with_capacity(3),bindings:Vec::with_capacity(8),validated:Vec::with_capacity(8),counters,frame_allocator:None,persistent_frame_lease:None,prepared_frame_lease:[None,None],output_waker:None})
    }
    pub fn set_frame_allocator(&mut self, allocator:Option<Arc<dyn crate::gpu::GpuFrameAllocator>>)->Result<(),GpuError>{
        if self.frame_allocator.is_some()||!self.slots.is_empty(){return Err(GpuError::InvalidImage("frame allocator must be installed once before output allocation".into()));}
        let persistent_frame_lease=if let Some(a)=&allocator{
            let bytes=self.input.buffer.size().checked_add(self.parameters.size()).and_then(|n|self.detector.as_ref().map_or(Some(n),|d|n.checked_add(d.persistent_buffer_bytes())))
                .ok_or_else(||GpuError::InvalidImage("persistent working-set size overflow".into()))?;
            Some(a.reserve(0,bytes)?.ok_or(GpuError::Capacity)?)
        }else{None};
        self.frame_allocator=allocator;self.persistent_frame_lease=persistent_frame_lease;Ok(())
    }
    /// Wake after the final external consumer releases a reusable output slot.
    pub fn set_output_waker(&mut self,waker:Arc<dyn Fn()+Send+Sync>){self.output_waker=Some(waker);}
    pub fn prepare_outputs(&mut self,width:u32,height:u32,minimum:usize)->Result<(),GpuError>{
        if minimum==0||minimum>2{return Err(GpuError::InvalidImage("resident output preflight supports one or two slots".into()));}
        self.resize_profile(width,height)?;
        let scale=if self.kind==0{4}else{1};let ow=width.checked_mul(scale).ok_or_else(||GpuError::InvalidImage("output overflow".into()))?;let oh=height.checked_mul(scale).ok_or_else(||GpuError::InvalidImage("output overflow".into()))?;
        // Reclaim obsolete idle profiles before reserving their replacement. Bindings
        // own the underlying textures/buffers, so evict those alongside the slots.
        if self.slots.iter().any(|slot|(slot.width!=ow||slot.height!=oh)&&Arc::strong_count(slot)==1){
            self.bindings.clear();
            self.slots.retain(|slot|(slot.width==ow&&slot.height==oh)||Arc::strong_count(slot)>1);
        }
        for pending in &mut self.prepared_frame_lease{if pending.as_ref().is_some_and(|(w,h,_)|*w!=ow||*h!=oh){*pending=None;}}
        let count=self.slots.iter().filter(|s|s.width==ow&&s.height==oh).count()
            +self.prepared_frame_lease.iter().flatten().count();
        let writable=self.prepared_frame_lease.iter().any(Option::is_some)
            ||self.slots.iter().any(|slot|slot.width==ow&&slot.height==oh&&Arc::strong_count(slot)==1);
        // Pool size is not availability: two retained outputs may need a third
        // slot, while three retained outputs must stop speculative source demand.
        let additional=minimum.saturating_sub(count).max(usize::from(!writable));
        if self.slots.len()+self.prepared_frame_lease.iter().flatten().count()+additional>3{return Err(GpuError::Capacity);}
        for _ in 0..additional {
            let index=self.prepared_frame_lease.iter().position(Option::is_none).ok_or(GpuError::Capacity)?;
            let lease=self.reserve_slot(ow,oh)?;
            self.prepared_frame_lease[index]=Some((ow,oh,lease));
        }
        Ok(())
    }
    pub fn prepare_output(&mut self,width:u32,height:u32)->Result<(),GpuError>{self.prepare_outputs(width,height,1)}
    fn reserve_slot(&self,ow:u32,oh:u32)->Result<Option<Arc<dyn std::any::Any+Send+Sync>>,GpuError>{
        let texture=u64::from(ow).checked_mul(u64::from(oh)).and_then(|n|n.checked_mul(4)).ok_or_else(||GpuError::InvalidImage("output allocation size overflow".into()))?;
        let raw=buffer_bytes(&self.model.output_shape)?;
        #[cfg(feature="gpu-diagnostics")]
        let logits=self.model.diagnostic_logits_shape.as_ref().map(|s|buffer_bytes(s)).transpose()?.unwrap_or(0);
        #[cfg(not(feature="gpu-diagnostics"))]
        let logits=0;
        let bytes=texture.checked_add(raw).and_then(|n|n.checked_add(logits)).and_then(|n|n.checked_add(2048+8+8)).and_then(|n|if self.kind==4{n.checked_add(16+8400*32)}else{Some(n)}).ok_or_else(||GpuError::InvalidImage("output working-set size overflow".into()))?;
        if let Some(a)=&self.frame_allocator{a.reserve(1,bytes)?.map(Some).ok_or(GpuError::Capacity)}else{Ok(None)}
    }
    pub fn model_plan(&self)->&ModelPlan{&self.model.plan}
    pub fn device_owner(&self)->&GpuDeviceOwner{&self.owner}
    pub fn counters(&self)->GpuCounters{self.counters}
    /// Explicit diagnostic consumer only; production never maps the input tensor.
    pub fn input_buffer(&self)->Arc<wgpu::Buffer>{self.input.buffer.clone()}
    #[cfg(feature = "gpu-diagnostics")]
    pub fn diagnostic_output_shape(&self) -> &[usize] { &self.model.output_shape }
    #[cfg(feature = "gpu-diagnostics")]
    pub fn diagnostic_logits_shape(&self)->Option<&[usize]>{self.model.diagnostic_logits_shape.as_deref()}
    fn resize_profile(&mut self,width:u32,height:u32)->Result<(),GpuError>{
        if self.kind!=0||self.model.input_shape[2..]==[height as usize,width as usize]{return Ok(());}
        let output_width=width.checked_mul(4).ok_or_else(||GpuError::InvalidImage("4x dimension overflow".into()))?;
        let output_height=height.checked_mul(4).ok_or_else(||GpuError::InvalidImage("4x dimension overflow".into()))?;
        let mut model_json:serde_json::Value=serde_json::from_slice(&self.model.graph_json).map_err(|e|GpuError::InvalidImage(e.to_string()))?;
        let input_shape=vec![1,3,height as usize,width as usize];let output_shape=vec![1,3,output_height as usize,output_width as usize];
        let persistent_frame_lease=if let Some(a)=&self.frame_allocator{
            let bytes=buffer_bytes(&input_shape)?.checked_add(self.parameters.size()).and_then(|n|self.detector.as_ref().map_or(Some(n),|d|n.checked_add(d.persistent_buffer_bytes())))
                .ok_or_else(||GpuError::InvalidImage("persistent working-set size overflow".into()))?;
            Some(a.reserve(0,bytes)?.ok_or(GpuError::Capacity)?)
        }else{None};
        model_json["inputs"][0]["shape"]=serde_json::json!(input_shape);model_json["outputs"][0]["shape"]=serde_json::json!(output_shape);
        // Recompile only on a source-size transition; original weights/metadata stay intact.
        let old_json=std::mem::replace(&mut self.model.graph_json,serde_json::to_vec(&model_json).map_err(|e|GpuError::InvalidImage(e.to_string()))?);
        let native=match NativeGraph::new(&self.owner.device,&self.model){Ok(n)=>n,Err(e)=>{self.model.graph_json=old_json;return Err(e)}};
        let input=NativeBuffer::new(&self.owner.device,buffer_bytes(&input_shape)?)?;
        self.native=native;self.input=input;self.model.input_shape=input_shape;self.model.output_shape=output_shape;
        if self.frame_allocator.is_some(){self.persistent_frame_lease=persistent_frame_lease;}
        self.model.plan.input_width=width;self.model.plan.input_height=height;self.model.plan.output_width=output_width;self.model.plan.output_height=output_height;
        self.bindings.clear();self.slots.retain(|slot|Arc::strong_count(slot)>1);Ok(())
    }
    pub fn submit_texture(&mut self,texture:&wgpu::Texture,width:u32,height:u32)->Result<CatalogGpuOutput,GpuError>{
        let started=Instant::now();
        if width==0||height==0||texture.width()!=width||texture.height()!=height||texture.dimension()!=wgpu::TextureDimension::D2||texture.depth_or_array_layers()!=1||texture.mip_level_count()!=1||texture.sample_count()!=1||!texture.usage().contains(wgpu::TextureUsages::TEXTURE_BINDING)||!matches!(texture.format(),wgpu::TextureFormat::Rgba8Unorm|wgpu::TextureFormat::Bgra8Unorm){return Err(GpuError::InvalidImage("requires matching byte-domain RGBA8/BGRA8 single-layer source".into()));}
        let scale=if self.kind==0{4}else{1};let ow=width.checked_mul(scale).ok_or_else(||GpuError::InvalidImage("output overflow".into()))?;let oh=height.checked_mul(scale).ok_or_else(||GpuError::InvalidImage("output overflow".into()))?;
        let limits=self.owner.device.limits();if ow>limits.max_texture_dimension_2d||oh>limits.max_texture_dimension_2d{return Err(GpuError::DeviceLimit("output texture exceeds adapter limits".into()));}
        // Validate each rotating source once, before native graph execution.
        if !self.validated.contains(texture) {
            let scope=self.owner.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let view=texture.create_view(&Default::default());
            let validation=self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("validate resident source device"),layout:&self.preprocess.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&view)},entry(1,&self.input.buffer),entry(2,&self.parameters)]});
            if let Some(error)=pollster_validation(scope){return Err(GpuError::Validation(error));}drop(validation);
            if self.validated.len()==8{self.validated.remove(0);}
            self.validated.push(texture.clone());
        }
        self.resize_profile(width,height)?;
        if self.source_size!=(width,height){
            self.source_size=(width,height);self.bindings.clear();
            let raww=*self.model.output_shape.last().unwrap()as u32;let rawh=self.model.output_shape[self.model.output_shape.len()-2]as u32;
            let integers=[width,height,self.model.input_shape[3]as u32,self.model.input_shape[2]as u32,ow,oh,raww,rawh];
            let mut bytes=[0u8;48];for(i,v)in integers.into_iter().enumerate(){bytes[i*4..i*4+4].copy_from_slice(&v.to_ne_bytes());}
            for(i,v)in[self.options.score_threshold,self.options.iou_threshold,self.kind as f32,0.0].into_iter().enumerate(){bytes[32+i*4..36+i*4].copy_from_slice(&v.to_ne_bytes());}
            self.owner.queue.write_buffer(&self.parameters,0,&bytes);
        }
        // Cached bindings hold a slot for encoding only, not a consumer lease.
        self.slots.retain(|s|(s.width==ow&&s.height==oh)||Arc::strong_count(s)>1);
        let slot=if let Some(slot)=self.slots.iter().find(|s|s.width==ow&&s.height==oh&&Arc::strong_count(s)==1){slot.clone()}else{
            if self.slots.len()>=3{return Err(GpuError::Capacity);}
            let mut frame_lease=None;
            for pending in &mut self.prepared_frame_lease {
                if pending.as_ref().is_some_and(|(w,h,_)|*w==ow&&*h==oh) { frame_lease=pending.take().and_then(|(_,_,lease)|lease); break; }
            }
            if frame_lease.is_none()&&self.frame_allocator.is_some(){frame_lease=self.reserve_slot(ow,oh)?;}
            let raw=NativeBuffer::new(&self.owner.device,buffer_bytes(&self.model.output_shape)?)?;
            #[cfg(feature = "gpu-diagnostics")]
            let logits=self.model.diagnostic_logits_shape.as_ref().map(|shape|NativeBuffer::new(&self.owner.device,buffer_bytes(shape)?)).transpose()?;
            let texture=Arc::new(self.owner.device.create_texture(&wgpu::TextureDescriptor{label:Some("resident catalog leased output"),size:wgpu::Extent3d{width:ow,height:oh,depth_or_array_layers:1},mip_level_count:1,sample_count:1,dimension:wgpu::TextureDimension::D2,format:wgpu::TextureFormat::Rgba8Unorm,usage:wgpu::TextureUsages::STORAGE_BINDING|wgpu::TextureUsages::TEXTURE_BINDING|wgpu::TextureUsages::RENDER_ATTACHMENT|wgpu::TextureUsages::COPY_SRC,view_formats:&[]}));
            let view=Arc::new(texture.create_view(&Default::default()));
            let depth=self.owner.device.create_buffer(&wgpu::BufferDescriptor{label:Some("resident depth reduction partials"),size:2048,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let reduce_binding=self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("resident depth reduction"),layout:&self.reduce_layout,entries:&[entry(0,&raw.buffer),entry(1,&depth)]});
            let depth_range=self.owner.device.create_buffer(&wgpu::BufferDescriptor{label:Some("resident depth range"),size:8,usage:wgpu::BufferUsages::STORAGE|wgpu::BufferUsages::COPY_SRC,mapped_at_creation:false});
            let reduce_final_binding=self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("resident depth range final"),layout:&self.reduce_layout,entries:&[entry(0,&depth),entry(1,&depth_range)]});
            let detector=self.detector.as_ref().map(|d|d.prepare_slot(&self.owner.device,&raw.buffer,&self.parameters));
            let draw_dimensions=self.owner.device.create_buffer(&wgpu::BufferDescriptor{label:Some("YOLO box dimensions"),size:8,usage:wgpu::BufferUsages::UNIFORM|wgpu::BufferUsages::COPY_DST,mapped_at_creation:false});
            self.owner.queue.write_buffer(&draw_dimensions,0,&[ow.to_ne_bytes(),oh.to_ne_bytes()].concat());
            let draw_binding=if let(Some(pipeline),Some(detector))=(&self.draw_boxes,&detector){Some(self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("resident YOLO box binding"),layout:&pipeline.get_bind_group_layout(0),entries:&[entry(0,&detector.buffer),entry(1,&draw_dimensions)]}))}else{None};
            let slot=Arc::new(Slot{texture,view,raw,depth,depth_range,reduce_binding,reduce_final_binding,draw_binding,detector,width:ow,height:oh,_frame_lease:frame_lease,
                #[cfg(feature = "gpu-diagnostics")]
                logits});self.slots.push(slot.clone());slot
        };
        let binding_index=if let Some(index)=self.bindings.iter().position(|b|b.texture==*texture&&b.output==*slot.texture){index}else{
            let view=texture.create_view(&Default::default());
            let pre=self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("resident catalog ingress"),layout:&self.preprocess.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&view)},entry(1,&self.input.buffer),entry(2,&self.parameters)]});
            let auxiliary=slot.detector.as_ref().map(|d|d.buffer.as_ref()).unwrap_or(&slot.depth_range);
            let post=self.owner.device.create_bind_group(&wgpu::BindGroupDescriptor{label:Some("resident catalog presentation"),layout:&self.postprocess.get_bind_group_layout(0),entries:&[wgpu::BindGroupEntry{binding:0,resource:wgpu::BindingResource::TextureView(&view)},entry(1,&slot.raw.buffer),entry(2,&self.parameters),entry(3,auxiliary),wgpu::BindGroupEntry{binding:4,resource:wgpu::BindingResource::TextureView(&slot.view)}]});
            if self.bindings.len()==8{self.bindings.remove(0);}
            self.bindings.push(Binding{texture:texture.clone(),output:slot.texture.as_ref().clone(),pre,post});self.bindings.len()-1
        };
        let mut encoder=self.owner.device.create_command_encoder(&Default::default());
        {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&self.preprocess);pass.set_bind_group(0,&self.bindings[binding_index].pre,&[]);pass.dispatch_workgroups((self.model.input_shape[3]as u32).div_ceil(8),(self.model.input_shape[2]as u32).div_ceil(8),1);}
        self.owner.queue.submit([encoder.finish()]);
        #[cfg(feature = "gpu-diagnostics")]
        let diagnostic=slot.logits.as_ref();
        #[cfg(not(feature = "gpu-diagnostics"))]
        let diagnostic=None;
        self.native.encode(&self.owner.queue,&self.input,&slot.raw,diagnostic)?;
        let mut encoder=self.owner.device.create_command_encoder(&Default::default());
        if self.kind==2 {
            {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&self.reduce);pass.set_bind_group(0,&slot.reduce_binding,&[]);pass.dispatch_workgroups(256,1,1);}
            {let mut pass=encoder.begin_compute_pass(&Default::default());pass.set_pipeline(&self.reduce_final);pass.set_bind_group(0,&slot.reduce_final_binding,&[]);pass.dispatch_workgroups(1,1,1);}
        }
        if let(Some(detector),Some(buffers))=(&self.detector,&slot.detector){detector.encode(&mut encoder,buffers);}
        {let mut pass=encoder.begin_compute_pass(&wgpu::ComputePassDescriptor{label:Some("resident catalog output presentation"),timestamp_writes:None});pass.set_pipeline(&self.postprocess);pass.set_bind_group(0,&self.bindings[binding_index].post,&[]);pass.dispatch_workgroups(ow.div_ceil(8),oh.div_ceil(8),1);}
        if let Some(pipeline)=&self.draw_boxes {
            let mut pass=encoder.begin_render_pass(&wgpu::RenderPassDescriptor{label:Some("resident YOLO box overlay"),color_attachments:&[Some(wgpu::RenderPassColorAttachment{view:&slot.view,depth_slice:None,resolve_target:None,ops:wgpu::Operations{load:wgpu::LoadOp::Load,store:wgpu::StoreOp::Store}})],depth_stencil_attachment:None,timestamp_writes:None,occlusion_query_set:None,multiview_mask:None});
            pass.set_pipeline(pipeline);pass.set_bind_group(0,slot.draw_binding.as_ref().expect("detection box binding"),&[]);pass.draw(0..24,0..8400);
        }
        let submission=self.owner.queue.submit([encoder.finish()]);
        // Submitted native/wgpu work can outlive both the session and consumers.
        // Keep its physical accounting without treating same-queue work as a
        // consumer lease that would prevent safe ordered reuse of this slot.
        let leases=(self.persistent_frame_lease.clone(),slot._frame_lease.clone());
        if leases.0.is_some()||leases.1.is_some(){self.owner.queue.on_submitted_work_done(move||drop(leases));}
        self.counters.submissions+=1;self.counters.compute_dispatches+=if self.kind==2{3}else if self.kind==4{108}else{1};self.counters.last_wall_time=started.elapsed();self.counters.total_wall_time+=self.counters.last_wall_time;
        Ok(CatalogGpuOutput{slot:OutputLease::new(slot,self.output_waker.clone()),submission})
    }
}
fn pollster_validation(scope:wgpu::ErrorScopeGuard)->Option<String>{
    use std::{future::Future,task::{Context,Poll,Wake,Waker}};
    struct WakeThread(std::thread::Thread);impl Wake for WakeThread{fn wake(self:Arc<Self>){self.0.unpark();}fn wake_by_ref(self:&Arc<Self>){self.0.unpark();}}
    let waker=Waker::from(Arc::new(WakeThread(std::thread::current())));let mut context=Context::from_waker(&waker);let mut future=std::pin::pin!(scope.pop());
    loop{match future.as_mut().poll(&mut context){Poll::Ready(error)=>return error.map(|e|e.to_string()),Poll::Pending=>std::thread::park()}}
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resident_tensor_byte_sizes_are_checked() {
        assert_eq!(buffer_bytes(&[1, 3, 2, 4]).unwrap(), 96);
        assert!(buffer_bytes(&[1,0,3]).is_err());
        assert!(buffer_bytes(&[usize::MAX, 2]).is_err());
    }
}
