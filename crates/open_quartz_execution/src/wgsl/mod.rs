pub mod compiler;
pub mod parser;

pub use compiler::{
    compile_shader, validate_shader, CompilePort, CompileRequest, CompiledShader,
    WgslCompilationError,
};
pub use parser::{parse_shader, ParsedShader};
