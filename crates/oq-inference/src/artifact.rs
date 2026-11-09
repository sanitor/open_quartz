//! Source-level ONNX interface inspection, independent of runtime support.
//!
//! This reads the declared top-level interface, not inferred operator semantics.
//! A missing tensor shape is unknown rank; a present empty shape is a scalar.
//! Element types and data locations retain their numerical ONNX codes, including
//! codes unknown to this implementation. Initializer payloads, graph nodes, and
//! training/function bodies are skipped by protobuf decoding, never copied or
//! loaded from external files. This is not a full ONNX semantic validator.

use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInterface {
    pub ir_version: Option<i64>,
    pub producer_name: Option<String>,
    pub producer_version: Option<String>,
    pub domain: Option<String>,
    pub model_version: Option<i64>,
    pub doc_string: Option<String>,
    pub metadata: BTreeMap<String, String>,
    pub opset_imports: Vec<ArtifactOpsetImport>,
    pub graph_name: Option<String>,
    pub graph_doc_string: Option<String>,
    pub graph_metadata: BTreeMap<String, String>,
    pub inputs: Vec<ArtifactInput>,
    pub outputs: Vec<ArtifactValue>,
    pub initializers: Vec<ArtifactInitializer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactOpsetImport {
    /// Missing and empty domains both designate the standard ONNX domain.
    pub domain: Option<String>,
    pub version: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactInput {
    pub value: ArtifactValue,
    pub requirement: ArtifactInputRequirement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactInputRequirement {
    /// A graph input with neither an initializer default nor an Optional type.
    Required,
    Optional,
    OverridableInitializer,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactValue {
    pub name: String,
    pub value_type: Option<ArtifactType>,
    pub doc_string: Option<String>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactType {
    pub denotation: Option<String>,
    pub kind: ArtifactValueKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactValueKind {
    Tensor {
        element_type: Option<i32>,
        shape: ArtifactShape,
    },
    SparseTensor {
        element_type: Option<i32>,
        shape: ArtifactShape,
    },
    Sequence {
        element: Option<Box<ArtifactType>>,
    },
    Map {
        key_type: Option<i32>,
        value: Option<Box<ArtifactType>>,
    },
    Optional {
        element: Option<Box<ArtifactType>>,
    },
    Opaque {
        domain: Option<String>,
        name: Option<String>,
    },
    /// No recognized type discriminator was present; no type is inferred.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactShape {
    UnknownRank,
    /// An empty vector explicitly represents a rank-zero scalar.
    KnownRank(Vec<ArtifactDimension>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactDimension {
    pub value: ArtifactDimensionValue,
    pub denotation: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactDimensionValue {
    /// Retained verbatim, including an invalid negative source dimension.
    Static(i64),
    Symbol(String),
    Dynamic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTensor {
    pub name: Option<String>,
    pub element_type: Option<i32>,
    pub dimensions: Vec<i64>,
    pub doc_string: Option<String>,
    pub metadata: BTreeMap<String, String>,
    /// ONNX TensorProto.DataLocation code; None defaults to embedded data.
    pub data_location: Option<i32>,
    /// References only: paths, offsets, lengths and unknown keys remain strings.
    pub external_data: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactInitializer {
    Dense(ArtifactTensor),
    Sparse {
        dimensions: Vec<i64>,
        /// The sparse initializer's name and element type belong to this tensor.
        values: Option<ArtifactTensor>,
        indices: Option<ArtifactTensor>,
    },
}

impl ArtifactInitializer {
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Dense(tensor) => tensor.name.as_deref(),
            Self::Sparse { values, .. } => values.as_ref()?.name.as_deref(),
        }
    }
}

#[derive(Debug)]
pub enum ArtifactError {
    InvalidProtobuf(prost::DecodeError),
    MissingGraph,
    DuplicateName { scope: &'static str, name: String },
    DuplicateMetadata { scope: String, key: String },
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidProtobuf(error) => write!(f, "invalid ONNX protobuf: {error}"),
            Self::MissingGraph => f.write_str("ONNX model has no graph"),
            Self::DuplicateName { scope, name } => {
                write!(f, "duplicate ONNX {scope} name {name:?}")
            }
            Self::DuplicateMetadata { scope, key } => {
                write!(f, "duplicate ONNX metadata key {key:?} in {scope}")
            }
        }
    }
}

impl std::error::Error for ArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidProtobuf(error) => Some(error),
            _ => None,
        }
    }
}

/// Inspect the source model without constructing a runtime session or reading weights.
///
/// Protobuf wire errors, a missing graph, duplicate port/initializer names and
/// duplicate metadata keys are errors. Input and output namespaces are checked
/// separately: an identity graph may legally expose the same name in both.
/// Unrecognized protobuf fields are skipped for forward compatibility.
pub fn inspect_onnx(model: &[u8]) -> Result<ArtifactInterface, ArtifactError> {
    let model = proto::ModelProto::decode(model).map_err(ArtifactError::InvalidProtobuf)?;
    let graph = model.graph.ok_or(ArtifactError::MissingGraph)?;
    let metadata = collect_metadata(model.metadata_props, || "model".into())?;
    let graph_metadata = collect_metadata(graph.metadata_props, || "graph".into())?;
    let mut initializers =
        Vec::with_capacity(graph.initializer.len() + graph.sparse_initializer.len());
    for tensor in graph.initializer {
        initializers.push(ArtifactInitializer::Dense(convert_tensor(tensor)?));
    }
    for tensor in graph.sparse_initializer {
        initializers.push(ArtifactInitializer::Sparse {
            dimensions: tensor.dims,
            values: tensor.values.map(convert_tensor).transpose()?,
            indices: tensor.indices.map(convert_tensor).transpose()?,
        });
    }
    let mut initializer_names = BTreeSet::new();
    for initializer in &initializers {
        if let Some(name) = initializer.name() {
            if !initializer_names.insert(name) {
                return Err(ArtifactError::DuplicateName {
                    scope: "initializer",
                    name: name.to_owned(),
                });
            }
        }
    }
    let inputs = convert_values(graph.input, "input")?
        .into_iter()
        .map(|value| {
            let requirement = if initializer_names.contains(value.name.as_str()) {
                ArtifactInputRequirement::OverridableInitializer
            } else if matches!(
                value.value_type.as_ref().map(|ty| &ty.kind),
                Some(ArtifactValueKind::Optional { .. })
            ) {
                ArtifactInputRequirement::Optional
            } else {
                ArtifactInputRequirement::Required
            };
            ArtifactInput { value, requirement }
        })
        .collect();
    let outputs = convert_values(graph.output, "output")?;
    Ok(ArtifactInterface {
        ir_version: model.ir_version,
        producer_name: model.producer_name,
        producer_version: model.producer_version,
        domain: model.domain,
        model_version: model.model_version,
        doc_string: model.doc_string,
        metadata,
        opset_imports: model
            .opset_import
            .into_iter()
            .map(|opset| ArtifactOpsetImport {
                domain: opset.domain,
                version: opset.version,
            })
            .collect(),
        graph_name: graph.name,
        graph_doc_string: graph.doc_string,
        graph_metadata,
        inputs,
        outputs,
        initializers,
    })
}

fn collect_metadata(
    entries: Vec<proto::StringStringEntryProto>,
    scope: impl FnOnce() -> String,
) -> Result<BTreeMap<String, String>, ArtifactError> {
    let mut result = BTreeMap::new();
    for entry in entries {
        let key = entry.key.unwrap_or_default();
        match result.entry(key) {
            std::collections::btree_map::Entry::Vacant(slot) => {
                slot.insert(entry.value.unwrap_or_default());
            }
            std::collections::btree_map::Entry::Occupied(entry) => {
                return Err(ArtifactError::DuplicateMetadata {
                    scope: scope(),
                    key: entry.key().clone(),
                });
            }
        }
    }
    Ok(result)
}

fn convert_values(
    values: Vec<proto::ValueInfoProto>,
    scope: &'static str,
) -> Result<Vec<ArtifactValue>, ArtifactError> {
    // Keep source ordering while checking duplicates without cloning names.
    let mut names = BTreeSet::new();
    for value in &values {
        let name = value.name.as_deref().unwrap_or_default();
        if !names.insert(name) {
            return Err(ArtifactError::DuplicateName {
                scope,
                name: name.to_owned(),
            });
        }
    }
    values
        .into_iter()
        .map(|value| {
            let name = value.name.unwrap_or_default();
            let metadata = collect_metadata(value.metadata_props, || format!("{scope} {name:?}"))?;
            Ok(ArtifactValue {
                name,
                value_type: value.r#type.map(convert_type),
                doc_string: value.doc_string,
                metadata,
            })
        })
        .collect()
}

fn convert_tensor(tensor: proto::TensorProto) -> Result<ArtifactTensor, ArtifactError> {
    let scope = || {
        format!(
            "initializer {:?}",
            tensor.name.as_deref().unwrap_or_default()
        )
    };
    let metadata = collect_metadata(tensor.metadata_props, scope)?;
    let external_data = collect_metadata(tensor.external_data, scope)?;
    Ok(ArtifactTensor {
        name: tensor.name,
        element_type: tensor.data_type,
        dimensions: tensor.dims,
        doc_string: tensor.doc_string,
        metadata,
        data_location: tensor.data_location,
        external_data,
    })
}

fn convert_type(value: proto::TypeProto) -> ArtifactType {
    use proto::type_proto::Value;
    let kind = match value.value {
        Some(Value::Tensor(tensor)) => ArtifactValueKind::Tensor {
            element_type: tensor.elem_type,
            shape: convert_shape(tensor.shape),
        },
        Some(Value::SparseTensor(tensor)) => ArtifactValueKind::SparseTensor {
            element_type: tensor.elem_type,
            shape: convert_shape(tensor.shape),
        },
        Some(Value::Sequence(sequence)) => ArtifactValueKind::Sequence {
            element: sequence
                .elem_type
                .map(|value| Box::new(convert_type(*value))),
        },
        Some(Value::Map(map)) => ArtifactValueKind::Map {
            key_type: map.key_type,
            value: map.value_type.map(|value| Box::new(convert_type(*value))),
        },
        Some(Value::Optional(optional)) => ArtifactValueKind::Optional {
            element: optional
                .elem_type
                .map(|value| Box::new(convert_type(*value))),
        },
        Some(Value::Opaque(opaque)) => ArtifactValueKind::Opaque {
            domain: opaque.domain,
            name: opaque.name,
        },
        None => ArtifactValueKind::Unknown,
    };
    ArtifactType {
        denotation: value.denotation,
        kind,
    }
}

fn convert_shape(shape: Option<proto::TensorShapeProto>) -> ArtifactShape {
    match shape {
        None => ArtifactShape::UnknownRank,
        Some(shape) => ArtifactShape::KnownRank(
            shape
                .dim
                .into_iter()
                .map(|dimension| {
                    let value = match dimension.value {
                        Some(proto::dimension::Value::DimValue(value)) => {
                            ArtifactDimensionValue::Static(value)
                        }
                        Some(proto::dimension::Value::DimParam(value)) => {
                            ArtifactDimensionValue::Symbol(value)
                        }
                        None => ArtifactDimensionValue::Dynamic,
                    };
                    ArtifactDimension {
                        value,
                        denotation: dimension.denotation,
                    }
                })
                .collect(),
        ),
    }
}

// Selected fields from the official ONNX proto2 schema:
// https://github.com/onnx/onnx/blob/main/onnx/onnx.proto
// Field numbers and scalar presence match that schema. In particular, TensorProto
// has NO payload fields here: prost skips them directly in the borrowed input.
// Recursive types retain prost's default recursion limit for untrusted artifacts.
mod proto {
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ModelProto {
        #[prost(int64, optional, tag = "1")]
        pub ir_version: Option<i64>,
        #[prost(string, optional, tag = "2")]
        pub producer_name: Option<String>,
        #[prost(string, optional, tag = "3")]
        pub producer_version: Option<String>,
        #[prost(string, optional, tag = "4")]
        pub domain: Option<String>,
        #[prost(int64, optional, tag = "5")]
        pub model_version: Option<i64>,
        #[prost(string, optional, tag = "6")]
        pub doc_string: Option<String>,
        #[prost(message, optional, tag = "7")]
        pub graph: Option<GraphProto>,
        #[prost(message, repeated, tag = "8")]
        pub opset_import: Vec<OperatorSetIdProto>,
        #[prost(message, repeated, tag = "14")]
        pub metadata_props: Vec<StringStringEntryProto>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct GraphProto {
        #[prost(string, optional, tag = "2")]
        pub name: Option<String>,
        #[prost(message, repeated, tag = "5")]
        pub initializer: Vec<TensorProto>,
        #[prost(string, optional, tag = "10")]
        pub doc_string: Option<String>,
        #[prost(message, repeated, tag = "11")]
        pub input: Vec<ValueInfoProto>,
        #[prost(message, repeated, tag = "12")]
        pub output: Vec<ValueInfoProto>,
        #[prost(message, repeated, tag = "15")]
        pub sparse_initializer: Vec<SparseTensorProto>,
        #[prost(message, repeated, tag = "16")]
        pub metadata_props: Vec<StringStringEntryProto>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ValueInfoProto {
        #[prost(string, optional, tag = "1")]
        pub name: Option<String>,
        #[prost(message, optional, tag = "2")]
        pub r#type: Option<TypeProto>,
        #[prost(string, optional, tag = "3")]
        pub doc_string: Option<String>,
        #[prost(message, repeated, tag = "4")]
        pub metadata_props: Vec<StringStringEntryProto>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StringStringEntryProto {
        #[prost(string, optional, tag = "1")]
        pub key: Option<String>,
        #[prost(string, optional, tag = "2")]
        pub value: Option<String>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct OperatorSetIdProto {
        #[prost(string, optional, tag = "1")]
        pub domain: Option<String>,
        #[prost(int64, optional, tag = "2")]
        pub version: Option<i64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TensorProto {
        #[prost(int64, repeated, packed = "false", tag = "1")]
        pub dims: Vec<i64>,
        #[prost(int32, optional, tag = "2")]
        pub data_type: Option<i32>,
        #[prost(string, optional, tag = "8")]
        pub name: Option<String>,
        #[prost(string, optional, tag = "12")]
        pub doc_string: Option<String>,
        #[prost(message, repeated, tag = "13")]
        pub external_data: Vec<StringStringEntryProto>,
        #[prost(int32, optional, tag = "14")]
        pub data_location: Option<i32>,
        #[prost(message, repeated, tag = "16")]
        pub metadata_props: Vec<StringStringEntryProto>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SparseTensorProto {
        #[prost(message, optional, tag = "1")]
        pub values: Option<TensorProto>,
        #[prost(message, optional, tag = "2")]
        pub indices: Option<TensorProto>,
        #[prost(int64, repeated, packed = "false", tag = "3")]
        pub dims: Vec<i64>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TensorShapeProto {
        #[prost(message, repeated, tag = "1")]
        pub dim: Vec<Dimension>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Dimension {
        #[prost(oneof = "dimension::Value", tags = "1, 2")]
        pub value: Option<dimension::Value>,
        #[prost(string, optional, tag = "3")]
        pub denotation: Option<String>,
    }

    pub mod dimension {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Value {
            #[prost(int64, tag = "1")]
            DimValue(i64),
            #[prost(string, tag = "2")]
            DimParam(String),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TypeProto {
        #[prost(oneof = "type_proto::Value", tags = "1, 4, 5, 7, 8, 9")]
        pub value: Option<type_proto::Value>,
        #[prost(string, optional, tag = "6")]
        pub denotation: Option<String>,
    }

    pub mod type_proto {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum Value {
            #[prost(message, tag = "1")]
            Tensor(super::TensorTypeProto),
            #[prost(message, tag = "4")]
            Sequence(super::SequenceTypeProto),
            #[prost(message, tag = "5")]
            Map(super::MapTypeProto),
            #[prost(message, tag = "7")]
            Opaque(super::OpaqueTypeProto),
            #[prost(message, tag = "8")]
            SparseTensor(super::TensorTypeProto),
            #[prost(message, tag = "9")]
            Optional(super::SequenceTypeProto),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TensorTypeProto {
        #[prost(int32, optional, tag = "1")]
        pub elem_type: Option<i32>,
        #[prost(message, optional, tag = "2")]
        pub shape: Option<TensorShapeProto>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct SequenceTypeProto {
        #[prost(message, optional, boxed, tag = "1")]
        pub elem_type: Option<Box<TypeProto>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct MapTypeProto {
        #[prost(int32, optional, tag = "1")]
        pub key_type: Option<i32>,
        #[prost(message, optional, boxed, tag = "2")]
        pub value_type: Option<Box<TypeProto>>,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct OpaqueTypeProto {
        #[prost(string, optional, tag = "1")]
        pub domain: Option<String>,
        #[prost(string, optional, tag = "2")]
        pub name: Option<String>,
    }
}

#[cfg(test)]
#[path = "artifact/tests.rs"]
mod tests;
