//! Tish recursive descent parser.

mod parser;

use parser::Parser;

use tish_ast::Program;
use tish_lexer::Lexer;

pub fn parse(source: &str) -> Result<Program, String> {
    let lexer = Lexer::new(source);
    let tokens: Result<Vec<_>, _> = lexer.collect();
    let tokens = tokens?;
    let mut parser = Parser::new(&tokens);
    parser.parse_program()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tish_ast::{Expr, ObjectProp, Statement};

    #[test]
    fn test_async_fn_parse() {
        let program = parse("async fn foo() { }").expect("parse async fn");
        assert_eq!(program.statements.len(), 1);
        if let tish_ast::Statement::FunDecl { async_, name, .. } = &program.statements[0] {
            assert!(async_, "expected async function");
            assert_eq!(name.as_ref(), "foo");
        } else {
            panic!("expected FunDecl");
        }
    }

    #[test]
    fn test_object_literal_shorthand_single() {
        let program = parse("const o = { port }").expect("parse object shorthand");
        assert_eq!(program.statements.len(), 1);
        let stmt = &program.statements[0];
        let init = match stmt {
            Statement::VarDecl { init: Some(ref i), .. } => i,
            _ => panic!("expected VarDecl with init"),
        };
        let props = match init {
            Expr::Object { ref props, .. } => props,
            _ => panic!("expected Object expr"),
        };
        assert_eq!(props.len(), 1);
        match &props[0] {
            ObjectProp::KeyValue(k, v) => {
                assert_eq!(k.as_ref(), "port");
                if let Expr::Ident { ref name, .. } = v {
                    assert_eq!(name.as_ref(), "port");
                } else {
                    panic!("expected Ident value for shorthand");
                }
            }
            _ => panic!("expected KeyValue prop"),
        }
    }

    #[test]
    fn test_object_literal_string_key() {
        let program = parse(r#"const o = { "ai-a": 0, human: 1 }"#).expect("parse object with string key");
        assert_eq!(program.statements.len(), 1);
        let stmt = &program.statements[0];
        let init = match stmt {
            Statement::VarDecl { init: Some(ref i), .. } => i,
            _ => panic!("expected VarDecl with init"),
        };
        let props = match init {
            Expr::Object { ref props, .. } => props,
            _ => panic!("expected Object expr"),
        };
        assert_eq!(props.len(), 2);
        match &props[0] {
            ObjectProp::KeyValue(k, _) => assert_eq!(k.as_ref(), "ai-a"),
            _ => panic!("expected KeyValue prop"),
        }
        match &props[1] {
            ObjectProp::KeyValue(k, _) => assert_eq!(k.as_ref(), "human"),
            _ => panic!("expected KeyValue prop"),
        }
    }

    #[test]
    fn test_object_literal_shorthand_mixed() {
        let program = parse("const o = { port, x: 1 }").expect("parse mixed object");
        assert_eq!(program.statements.len(), 1);
        let stmt = &program.statements[0];
        let init = match stmt {
            Statement::VarDecl { init: Some(ref i), .. } => i,
            _ => panic!("expected VarDecl with init"),
        };
        let props = match init {
            Expr::Object { ref props, .. } => props,
            _ => panic!("expected Object expr"),
        };
        assert_eq!(props.len(), 2);
        match &props[0] {
            ObjectProp::KeyValue(k, v) => {
                assert_eq!(k.as_ref(), "port");
                if let Expr::Ident { ref name, .. } = v {
                    assert_eq!(name.as_ref(), "port");
                } else {
                    panic!("expected Ident value for shorthand");
                }
            }
            _ => panic!("expected KeyValue prop"),
        }
        match &props[1] {
            ObjectProp::KeyValue(k, v) => {
                assert_eq!(k.as_ref(), "x");
                if let Expr::Literal { .. } = v {
                    // x: 1
                } else {
                    panic!("expected Literal for x");
                }
            }
            _ => panic!("expected KeyValue prop"),
        }
    }
}
