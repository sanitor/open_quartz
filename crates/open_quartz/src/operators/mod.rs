//! Typed, device-independent structured operators and their pure lowering contracts.
mod color_corrector;
mod hdr;
mod lowering;

pub use color_corrector::*;
pub use hdr::*;
pub use lowering::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperatorError(pub String);

impl std::fmt::Display for OperatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(&self.0) }
}
impl std::error::Error for OperatorError {}
