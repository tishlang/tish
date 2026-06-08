//! Tish recursive descent parser.

mod parser;

use parser::Parser;

use tishlang_ast::Program;
use tishlang_lexer::Lexer;
pub use tishlang_lexer::LexerOptions;

/// Parse `source`, reading lexer options from the environment (e.g. `TISH_IGNORE_INDENT=1`
/// to ignore indentation syntax). Every backend funnels through here, so the env toggle
/// reaches run/build/dump-ast/fmt/lint/lsp uniformly.
pub fn parse(source: &str) -> Result<Program, String> {
    parse_with_options(source, LexerOptions::from_env())
}

/// Parse with explicit lexer options, bypassing the environment.
///
/// With `LexerOptions { ignore_indent: true }`, indentation is treated as ordinary
/// whitespace and blocks must be brace-delimited — useful for debugging how nested
/// blocks transpile, since fully brace-delimited code parses identically either way.
pub fn parse_with_options(source: &str, options: LexerOptions) -> Result<Program, String> {
    let lexer = Lexer::with_options(source, options);
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
            Statement::VarDecl {
                init: Some(ref i), ..
            } => i,
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
        let program =
            parse(r#"const o = { "ai-a": 0, human: 1 }"#).expect("parse object with string key");
        assert_eq!(program.statements.len(), 1);
        let stmt = &program.statements[0];
        let init = match stmt {
            Statement::VarDecl {
                init: Some(ref i), ..
            } => i,
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
            Statement::VarDecl {
                init: Some(ref i), ..
            } => i,
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
                assert!(
                    matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Foo")
                );
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
                assert!(
                    matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Uint8Array")
                );
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
                    Expr::Member { prop: tishlang_ast::MemberProp::Name { name, .. }, .. } if name.as_ref() == "AudioContext"
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
                    Expr::New {
                        callee: inner,
                        args: inner_args,
                        ..
                    } => {
                        assert!(
                            matches!(inner.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Date")
                        );
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
            Expr::Member {
                object,
                prop: tishlang_ast::MemberProp::Name { name, .. },
                ..
            } => {
                assert_eq!(name.as_ref(), "bar");
                match object.as_ref() {
                    Expr::New { callee, args, .. } => {
                        assert!(
                            matches!(callee.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "Foo")
                        );
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
                assert!(
                    matches!(&args[0], CallArg::Spread(Expr::Ident { name, .. }) if name.as_ref() == "xs")
                );
            }
            _ => panic!("expected New"),
        }
    }

    #[test]
    fn stdlib_builtins_d_tish_parses() {
        const SRC: &str = include_str!("../../../stdlib/builtins.d.tish");
        parse(SRC).expect("stdlib/builtins.d.tish should parse");
    }

    #[test]
    fn for_empty_head_parses() {
        let src = r#"fn f() {
  for (;;)
    const x = 1
}"#;
        let program = parse(src).expect("for (;;)");
        let body = match &program.statements[0] {
            Statement::FunDecl { body, .. } => body,
            _ => panic!("expected fn"),
        };
        let stmts = match body.as_ref() {
            Statement::Block { statements, .. } => statements,
            _ => panic!("expected block body"),
        };
        assert!(
            matches!(
                stmts.iter().find(|s| matches!(s, Statement::For { .. })),
                Some(Statement::For {
                    init: None,
                    cond: None,
                    update: None,
                    ..
                })
            ),
            "expected for (;;)"
        );
    }

    #[test]
    fn brace_function_body_does_not_nest_block_around_first_let() {
        let src = "fn h() {\n  let a = 1\n  let b = 2\n}\n";
        let program = parse(src).expect("parse");
        let body = match &program.statements[0] {
            Statement::FunDecl { body, .. } => body,
            _ => panic!("expected fn"),
        };
        let stmts = match body.as_ref() {
            Statement::Block { statements, .. } => statements,
            _ => panic!("expected block body"),
        };
        assert_eq!(
            stmts.len(),
            2,
            "expected two top-level lets in fn body, not Block(let) + let — got {stmts:?}"
        );
        assert!(matches!(stmts[0], Statement::VarDecl { .. }));
        assert!(matches!(stmts[1], Statement::VarDecl { .. }));
    }

    #[test]
    fn member_access_allows_type_property_name() {
        let src = "fn f() {\n  const label = 0\n  label.type = \"button\"\n}\n";
        parse(src).expect("label.type should parse: `type` is a keyword but valid after `.`");
    }

    #[test]
    fn brace_block_stmt_then_const_then_if_are_siblings() {
        let src = "fn g() {\n  f()\n  const x = 1\n  if (x) {\n    f()\n  }\n}\n";
        let program = parse(src).expect("parse");
        let body = match &program.statements[0] {
            Statement::FunDecl { body, .. } => body,
            _ => panic!("expected fn"),
        };
        let stmts = match body.as_ref() {
            Statement::Block { statements, .. } => statements,
            _ => panic!("expected block body"),
        };
        assert_eq!(
            stmts.len(),
            3,
            "expected expr; const; if as siblings — got {stmts:?}"
        );
        assert!(matches!(stmts[0], Statement::ExprStmt { .. }));
        assert!(matches!(stmts[1], Statement::VarDecl { .. }));
        assert!(matches!(stmts[2], Statement::If { .. }));
    }

    #[test]
    fn ignore_indent_parses_brace_blocks_identically() {
        // Fully brace-delimited code: braces are authoritative, indentation is decoration.
        // Ignoring indentation must therefore produce an identical AST.
        let src = "fn f() {\n  let a = 1\n  if (a) {\n    let b = 2\n    g(b)\n  }\n}\n";
        let normal = parse(src).expect("parse (indentation significant)");
        let ignored = parse_with_options(src, LexerOptions { ignore_indent: true })
            .expect("parse (indentation ignored)");
        assert_eq!(
            format!("{normal:#?}"),
            format!("{ignored:#?}"),
            "brace-delimited code must parse identically with indentation ignored"
        );
    }

    #[test]
    fn ignore_indent_drops_indentation_induced_block() {
        // A leading-indented line makes the lexer open an indent level, so the parser wraps
        // `a()` in a `Block` — the kind of stray, indentation-driven nesting that can give
        // transpiled JS the wrong lexical scope. Ignoring indentation removes that wrapper.
        let src = "  a()\nb()\n";

        let normal = parse(src).expect("parse normal");
        assert!(
            matches!(normal.statements.first(), Some(Statement::Block { .. })),
            "indentation should wrap a() in a Block, got: {:?}",
            normal.statements
        );

        let ignored = parse_with_options(src, LexerOptions { ignore_indent: true })
            .expect("parse ignored");
        assert!(
            ignored
                .statements
                .iter()
                .all(|s| matches!(s, Statement::ExprStmt { .. })),
            "with indentation ignored, both calls are flat expression statements, got: {:?}",
            ignored.statements
        );
    }
}
