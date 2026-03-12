//! Convert OXC AST (with semantic info) to Tish AST.

mod expr;
mod stmt;

use oxc::allocator::Allocator;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use tish_ast::Program;

use crate::error::{ConvertError, ConvertErrorKind};
use crate::transform::stmt::convert_statements;

/// Convert JavaScript source to Tish AST.
///
/// Performs parse, semantic analysis, normalization (var/function hoisting),
/// and transformation to Tish AST.
pub fn convert(js_source: &str) -> Result<Program, ConvertError> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path("script.js").unwrap();
    let parser_ret = Parser::new(&allocator, js_source, source_type).parse();

    if parser_ret.panicked {
        let msg = parser_ret
            .errors
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ConvertError::new(ConvertErrorKind::Parse(msg)));
    }
    if !parser_ret.errors.is_empty() {
        let msg = parser_ret
            .errors
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ConvertError::new(ConvertErrorKind::Parse(msg)));
    }

    let program = parser_ret.program;
    let semantic_ret = SemanticBuilder::new()
        .with_check_syntax_error(true)
        .build(&program);

    if !semantic_ret.errors.is_empty() {
        let msg = semantic_ret
            .errors
            .into_iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(ConvertError::new(ConvertErrorKind::Semantic(msg)));
    }

    let statements = convert_statements(&program.body, &semantic_ret.semantic, js_source)?;
    Ok(Program { statements })
}
