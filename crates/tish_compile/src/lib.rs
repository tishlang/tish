//! Native compiler backend for Tish.
//!
//! Emits Rust source that links to tish_runtime.

mod codegen;
mod types;

pub use codegen::{compile, compile_with_features, compile_with_project_root};
pub use codegen::CompileError;
pub use types::{RustType, TypeContext};

#[cfg(test)]
mod tests {
    use super::*;
    use tish_parser::parse;

    #[test]
    fn typed_assign_conversion() {
        let src = r#"
fn sum(...args: number[]): number {
    let total: number = 0
    for (let n of args) { total = total + n }
    return total
}
"#;
        let program = parse(src).unwrap();
        let rust = compile(&program).unwrap();
        assert!(rust.contains("match &_v { Value::Number(n) => *n"), "expected typed assign conversion");
    }
}
