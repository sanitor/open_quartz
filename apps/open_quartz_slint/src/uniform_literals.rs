//! Shared parameter-literal rules used by both native graph hosts.
use open_quartz::{DataType, Port, ProjectNode};

pub(crate) fn uniform_shape(data_type: DataType) -> Option<usize> {
    Some(match data_type {
        DataType::Float | DataType::Int | DataType::Uint | DataType::Bool => 1,
        DataType::Vec2 | DataType::Ivec2 | DataType::Uvec2 => 2,
        DataType::Vec3 | DataType::Ivec3 | DataType::Uvec3 => 3,
        DataType::Vec4 | DataType::Ivec4 | DataType::Uvec4 | DataType::Mat2 => 4,
        DataType::Mat3 => 9,
        DataType::Mat4 => 16,
        _ => return None,
    })
}

pub(crate) fn default_uniform_value(data_type: DataType) -> serde_json::Value {
    let zero = match data_type {
        DataType::Bool => serde_json::Value::Bool(false),
        DataType::Int | DataType::Ivec2 | DataType::Ivec3 | DataType::Ivec4 => serde_json::json!(0_i32),
        DataType::Uint | DataType::Uvec2 | DataType::Uvec3 | DataType::Uvec4 => serde_json::json!(0_u32),
        _ => serde_json::json!(0.0),
    };
    match uniform_shape(data_type) {
        Some(arity) if arity > 1 => serde_json::Value::Array(vec![zero; arity]),
        _ => zero,
    }
}

pub(crate) fn format_uniform_literal(value: &serde_json::Value) -> String {
    fn scalar(value: &serde_json::Value) -> String {
        value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())
    }
    match value.as_array() {
        Some(values) => values.iter().map(scalar).collect::<Vec<_>>().join(", "),
        None => scalar(value),
    }
}

pub(crate) fn effective_uniform_value(node: &ProjectNode, port: &Port) -> serde_json::Value {
    let value = node.data.uniforms.get(&port.label).cloned()
        .unwrap_or_else(|| default_uniform_value(port.data_type));
    parse_uniform_literal(port.data_type, &format_uniform_literal(&value)).unwrap_or(value)
}

pub(crate) fn parse_uniform_literal(data_type: DataType, literal: &str) -> Result<serde_json::Value, String> {
    let arity = uniform_shape(data_type).ok_or_else(|| "Unsupported uniform type".to_owned())?;
    let parts = literal.split(',').map(str::trim).collect::<Vec<_>>();
    if parts.len() != arity {
        return Err(format!("Expected {arity} component(s), got {}", parts.len()));
    }
    let values = parts.into_iter().map(|part| match data_type {
        DataType::Bool => match part {
            "true" | "1" => Ok(serde_json::Value::Bool(true)),
            "false" | "0" => Ok(serde_json::Value::Bool(false)),
            _ => Err("Expected true, false, 1, or 0".to_owned()),
        },
        DataType::Int | DataType::Ivec2 | DataType::Ivec3 | DataType::Ivec4 => part.parse::<i32>()
            .map(|value| serde_json::json!(value)).map_err(|_| "Expected a signed 32-bit integer".to_owned()),
        DataType::Uint | DataType::Uvec2 | DataType::Uvec3 | DataType::Uvec4 => {
            if part.starts_with('-') { return Err("Expected an unsigned 32-bit integer".to_owned()); }
            part.parse::<u32>().map(|value| serde_json::json!(value))
                .map_err(|_| "Expected an unsigned 32-bit integer".to_owned())
        },
        _ => part.parse::<f64>().ok()
            .filter(|value| value.is_finite() && (*value as f32).is_finite())
            .map(|value| serde_json::json!(value))
            .ok_or_else(|| "Expected a finite 32-bit float".to_owned()),
    }).collect::<Result<Vec<_>,_>>()?;
    if arity == 1 { Ok(values.into_iter().next().expect("scalar has one component")) }
    else { Ok(serde_json::Value::Array(values)) }
}
