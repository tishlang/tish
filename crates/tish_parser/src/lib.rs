//! Tish recursive descent parser.

mod parser;

use parser::Parser;

use tishlang_ast::Program;
use tishlang_lexer::Lexer;

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
    use tishlang_ast::{CallArg, Expr, ObjectProp, Statement};

    #[test]
    fn test_async_fn_parse() {
        let program = parse("async fn foo() { }").expect("parse async fn");
        assert_eq!(program.statements.len(), 1);
        if let tishlang_ast::Statement::FunDecl { async_, name, .. } = &program.statements[0] {
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

    fn unwrap_expr_stmt(program: &tishlang_ast::Program) -> &Expr {
        match program.statements.first() {
            Some(Statement::ExprStmt { expr, .. }) => expr,
            _ => panic!("expected expression statement"),
        }
    }

    #[test]
    fn new_expression_simple_call() {
        let program = parse("new Foo()").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::New { callee, args, .. } => {
                assert!(matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Foo"));
                assert!(args.is_empty());
            }
            _ => panic!("expected New, got {:?}", e),
        }
    }

    #[test]
    fn new_expression_with_args() {
        let program = parse("new Uint8Array(16)").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::New { callee, args, .. } => {
                assert!(matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Uint8Array"));
                assert_eq!(args.len(), 1);
                assert!(matches!(&args[0], CallArg::Expr(Expr::Literal { .. })));
            }
            _ => panic!("expected New"),
        }
    }

    #[test]
    fn new_expression_member_callee() {
        let program = parse("new ns.AudioContext()").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::New { callee, args, .. } => {
                assert!(matches!(
                    callee.as_ref(),
                    Expr::Member { prop: tishlang_ast::MemberProp::Name(p), .. } if p.as_ref() == "AudioContext"
                ));
                assert!(args.is_empty());
            }
            _ => panic!("expected New"),
        }
    }

    #[test]
    fn new_expression_chained_new() {
        let program = parse("new new Date()").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::New { callee, args, .. } => {
                assert!(args.is_empty());
                match callee.as_ref() {
                    Expr::New { callee: inner, args: inner_args, .. } => {
                        assert!(matches!(inner.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Date"));
                        assert!(inner_args.is_empty());
                    }
                    _ => panic!("expected nested New"),
                }
            }
            _ => panic!("expected New"),
        }
    }

    #[test]
    fn new_then_member_access() {
        let program = parse("new Foo().bar").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::Member { object, prop: tishlang_ast::MemberProp::Name(p), .. } => {
                assert_eq!(p.as_ref(), "bar");
                match object.as_ref() {
                    Expr::New { callee, args, .. } => {
                        assert!(matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Foo"));
                        assert!(args.is_empty());
                    }
                    _ => panic!("expected New object"),
                }
            }
            _ => panic!("expected Member"),
        }
    }

    #[test]
    fn new_with_spread_arg() {
        let program = parse("new Foo(...xs)").expect("parse");
        let e = unwrap_expr_stmt(&program);
        match e {
            Expr::New { args, .. } => {
                assert!(matches!(&args[0], CallArg::Spread(Expr::Ident { name, .. }) if name.as_ref() == "xs"));
            }
            _ => panic!("expected New"),
        }
    }
}
