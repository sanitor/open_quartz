//! DirectML execution over renderer-owned D3D12 tensor buffers.
use crate::gpu::GpuError;
use ort::{
    error::Error,
    memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType},
    session::Session,
    sys,
    value::{Shape, TensorRefMut},
    AsPointer,
};
use std::{
    ffi::{c_void, CString},
    ptr::{self, NonNull},
};
use windows::{
    core::Interface,
    Win32::AI::MachineLearning::DirectML::{
        DMLCreateDevice, IDMLDevice, DML_CREATE_DEVICE_FLAG_NONE,
    },
};

pub(crate) struct Graph {
    session: Session,
    api: &'static sys::OrtDmlApi,
}

impl Graph {
    pub(crate) fn new(device: &wgpu::Device, model: &[u8]) -> Result<Self, GpuError> {
        if device.adapter_info().device_type == wgpu::DeviceType::Cpu {
            return Err(GpuError::UnsupportedBackend(
                "DirectML needs a hardware adapter".into(),
            ));
        }
        crate::runtime::initialize().map_err(validation)?;
        let mut api = ptr::null();
        unsafe {
            Error::result_from_status((ort::api().GetExecutionProviderApi)(
                c"DML".as_ptr(),
                21,
                &mut api,
            ))
        }
        .map_err(validation)?;
        let api = unsafe { api.cast::<sys::OrtDmlApi>().as_ref() }.ok_or_else(|| {
            GpuError::UnsupportedBackend("runtime has no DirectML provider".into())
        })?;
        let mut builder = Session::builder()
            .map_err(validation)?
            .with_no_environment_execution_providers()
            .map_err(validation)?
            .with_parallel_execution(false)
            .map_err(validation)?
            .with_memory_pattern(false)
            .map_err(validation)?
            .with_config_entry("session.disable_cpu_ep_fallback", "1")
            .map_err(validation)?;
        {
            let dx =
                unsafe { device.as_hal::<wgpu::hal::api::Dx12>() }.ok_or(GpuError::WrongDevice)?;
            let mut ml: Option<IDMLDevice> = None;
            unsafe { DMLCreateDevice(dx.raw_device(), DML_CREATE_DEVICE_FLAG_NONE, &mut ml) }
                .map_err(validation)?;
            let ml = ml.ok_or_else(|| validation("DirectML returned no device"))?;
            unsafe {
                Error::result_from_status((api.SessionOptionsAppendExecutionProvider_DML1)(
                    builder.ptr_mut(),
                    ml.as_raw(),
                    dx.raw_queue().as_raw(),
                ))
            }
            .map_err(validation)?;
        }
        let session = builder.commit_from_memory(model).map_err(validation)?;
        Ok(Self { session, api })
    }

    pub(crate) fn encode(
        &mut self,
        input: (&str, &[usize], &wgpu::Buffer),
        outputs: &[(&str, &[usize], &wgpu::Buffer)],
    ) -> Result<(), GpuError> {
        let read = Allocation::new(self.api, input.2)?;
        let read_tensor = read.tensor(input.1, input.2.size())?;
        let in_name = CString::new(input.0).map_err(validation)?;
        let writes = outputs
            .iter()
            .map(|&(_, _, buffer)| Allocation::new(self.api, buffer))
            .collect::<Result<Vec<_>, _>>()?;
        let write_tensors = writes
            .iter()
            .zip(outputs)
            .map(|(allocation, &(_, dims, buffer))| allocation.tensor(dims, buffer.size()))
            .collect::<Result<Vec<_>, _>>()?;
        let binding = self.session.create_binding().map_err(validation)?;
        let raw = binding.ptr().cast_mut();
        unsafe {
            Error::result_from_status((ort::api().BindInput)(
                raw,
                in_name.as_ptr(),
                read_tensor.ptr(),
            ))
        }
        .map_err(validation)?;
        for (tensor, &(name, _, _)) in write_tensors.iter().zip(outputs) {
            let name = CString::new(name).map_err(validation)?;
            unsafe {
                Error::result_from_status((ort::api().BindOutput)(raw, name.as_ptr(), tensor.ptr()))
            }
            .map_err(validation)?;
        }
        unsafe {
            Error::result_from_status((ort::api().RunWithBinding)(
                self.session.ptr().cast_mut(),
                ptr::null(),
                raw,
            ))
        }
        .map_err(validation)
    }
}

/// Compile the selected output with concrete input dimensions so shape-only
/// expressions can fold before the provider places the graph.
pub(crate) fn specialize(
    model: &crate::catalog_model::CatalogModel,
    input: &[usize],
    output: &[usize],
    diagnostic: Option<(&str, &[usize])>,
) -> Result<(Vec<u8>, String), GpuError> {
    use crate::gpu_model::proto::{Attribute, Node};
    use prost::Message;
    #[derive(Clone, PartialEq, Message)]
    struct Model {
        #[prost(int64, tag = "1")]
        ir: i64,
        #[prost(message, optional, tag = "7")]
        graph: Option<GraphProto>,
        #[prost(bytes = "vec", repeated, tag = "8")]
        opsets: Vec<Vec<u8>>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct GraphProto {
        #[prost(message, repeated, tag = "1")]
        nodes: Vec<Node>,
        #[prost(string, tag = "2")]
        name: String,
        #[prost(bytes = "vec", repeated, tag = "5")]
        initializers: Vec<Vec<u8>>,
        #[prost(message, repeated, tag = "11")]
        inputs: Vec<Value>,
        #[prost(message, repeated, tag = "12")]
        outputs: Vec<Value>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct Value {
        #[prost(string, tag = "1")]
        name: String,
        #[prost(message, optional, tag = "2")]
        ty: Option<Type>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct Type {
        #[prost(message, optional, tag = "1")]
        tensor: Option<Tensor>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct Tensor {
        #[prost(int32, tag = "1")]
        dtype: i32,
        #[prost(message, optional, tag = "2")]
        shape: Option<TensorShape>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct TensorShape {
        #[prost(message, repeated, tag = "1")]
        dims: Vec<Dimension>,
    }
    #[derive(Clone, PartialEq, Message)]
    struct Dimension {
        #[prost(int64, tag = "1")]
        size: i64,
    }
    fn value(name: String, dims: &[usize]) -> Value {
        Value {
            name,
            ty: Some(Type {
                tensor: Some(Tensor {
                    dtype: 1,
                    shape: Some(TensorShape {
                        dims: dims
                            .iter()
                            .map(|&size| Dimension { size: size as i64 })
                            .collect(),
                    }),
                }),
            }),
        }
    }
    let mut executable = Model::decode(model.onnx.as_ref()).map_err(validation)?;
    let graph = executable
        .graph
        .as_mut()
        .ok_or_else(|| validation("ONNX graph missing"))?;
    graph.inputs = vec![value(model.plan.input_name.clone(), input)];
    let mut name = model.output_name.clone();
    if model.output_dtype != 1 {
        // The presentation shader takes float class IDs on both platforms.
        let mut presentation = format!("{name}_oq_float");
        while graph
            .nodes
            .iter()
            .any(|node| node.output.contains(&presentation))
        {
            presentation.push('_');
        }
        graph.nodes.push(Node {
            input: vec![name],
            output: vec![presentation.clone()],
            op_type: "Cast".into(),
            attribute: vec![Attribute {
                name: "to".into(),
                i: Some(1),
                r#type: Some(2),
                ..Default::default()
            }],
            ..Default::default()
        });
        name = presentation;
    }
    graph.outputs = vec![value(name.clone(), output)];
    let mut needed = std::collections::HashSet::from([name.clone()]);
    if let Some((name, dims)) = diagnostic {
        graph.outputs.push(value(name.to_owned(), dims));
        needed.insert(name.to_owned());
    }
    let mut selected = Vec::new();
    for node in std::mem::take(&mut graph.nodes).into_iter().rev() {
        if node.output.iter().any(|edge| needed.contains(edge)) {
            needed.extend(node.input.iter().cloned());
            selected.push(node);
        }
    }
    graph.nodes = selected.into_iter().rev().collect();
    Ok((executable.encode_to_vec(), name))
}

struct Allocation {
    raw: NonNull<c_void>,
    api: &'static sys::OrtDmlApi,
}

impl Allocation {
    fn new(api: &'static sys::OrtDmlApi, buffer: &wgpu::Buffer) -> Result<Self, GpuError> {
        let raw_buffer =
            unsafe { buffer.as_hal::<wgpu::hal::api::Dx12>() }.ok_or(GpuError::WrongDevice)?;
        let resource = unsafe { raw_buffer.raw_resource() };
        let mut raw = ptr::null_mut();
        unsafe {
            Error::result_from_status((api.CreateGPUAllocationFromD3DResource)(
                resource.as_raw(),
                &mut raw,
            ))
        }
        .map_err(validation)?;
        Ok(Self {
            raw: NonNull::new(raw).ok_or_else(|| validation("DirectML returned no allocation"))?,
            api,
        })
    }

    fn tensor(&self, dims: &[usize], bytes: u64) -> Result<TensorRefMut<'_, f32>, GpuError> {
        if dims
            .iter()
            .try_fold(4u64, |size, &dim| size.checked_mul(dim as u64))
            != Some(bytes)
        {
            return Err(validation(
                "DirectML tensor dimensions do not match the GPU buffer",
            ));
        }
        let info = MemoryInfo::new(
            AllocationDevice::DIRECTML,
            0,
            AllocatorType::Device,
            MemoryType::Default,
        )
        .map_err(validation)?;
        let shape = Shape::new(dims.iter().map(|&dim| dim as i64));
        unsafe { TensorRefMut::from_raw(info, self.raw.as_ptr(), shape) }.map_err(validation)
    }
}

impl Drop for Allocation {
    fn drop(&mut self) {
        let _ =
            unsafe { Error::result_from_status((self.api.FreeGPUAllocation)(self.raw.as_ptr())) };
    }
}

fn validation(error: impl std::fmt::Display) -> GpuError {
    GpuError::Validation(format!("DirectML resident execution: {error}"))
}
