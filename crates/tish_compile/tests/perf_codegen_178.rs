//! #178 Bun-style fusion targets — AST shape checks + codegen assertions when fusion lands.

use std::fs;
use std::path::PathBuf;

use tishlang_ast::{BinOp, Expr, Statement};
use tishlang_compile::compile_project_full;
use tishlang_opt::optimize;
use tishlang_parser::parse;

fn enable_typed_flags() {
    for k in [
        "TISH_PARAM_NATIVE",
        "TISH_PARAM_INFER",
        "TISH_NATIVE_FN",
        "TISH_STRUCT_INFER",
        "TISH_FUSED_HOF",
        "TISH_NATIVE_HOF",
        "TISH_AGGREGATE_INFER",
    ] {
        std::env::set_var(k, "1");
    }
}

fn compile_fixture_typed(rel: &str) -> String {
    enable_typed_flags();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest.join("../..").join(rel).canonicalize().unwrap();
    let (rust, _, _, _) = compile_project_full(&path, path.parent(), &[], true).unwrap();
    rust
}

fn stmt_slice_unwrapped(body: &Statement) -> Vec<&Statement> {
    let mut cur = body;
    loop {
        match cur {
            Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
                if statements.len() == 1 {
                    cur = &statements[0];
                } else {
                    return statements.iter().collect();
                }
            }
            other => return vec![other],
        }
    }
}

fn find_flip_for_body(s: &Statement) -> Option<&Statement> {
    match s {
        Statement::For { cond, body, .. } => {
            if let Some(cond) = cond.as_ref() {
                if let Expr::Binary {
                    op: BinOp::Lt,
                    right,
                    ..
                } = cond
                {
                    if matches!(right.as_ref(), Expr::Ident { name, .. } if name.as_ref() == "k2") {
                        return Some(body);
                    }
                }
            }
            None
        }
        Statement::Block { statements, .. } | Statement::Multi { statements, .. } => {
            for st in statements {
                if let Some(b) = find_flip_for_body(st) {
                    return Some(b);
                }
            }
            None
        }
        Statement::While { body, .. } => find_flip_for_body(body),
        Statement::If { then_branch, .. } => find_flip_for_body(then_branch),
        _ => None,
    }
}

#[test]
fn fannkuch_flip_for_body_is_three_index_assigns() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let path = manifest
        .join("../../tests/perf/fannkuch.tish")
        .canonicalize()
        .unwrap();
    let program = optimize(&parse(&fs::read_to_string(&path).unwrap()).unwrap());
    let body = program
        .statements
        .iter()
        .find_map(|s| {
            if let Statement::FunDecl { name, body, .. } = s {
                if name.as_ref() == "fannkuch" {
                    return find_flip_for_body(body);
                }
            }
            None
        })
        .expect("flip for body");
    let stmts = stmt_slice_unwrapped(body);
    assert_eq!(stmts.len(), 3);
    for st in &stmts[1..] {
        let Statement::ExprStmt { expr, .. } = st else {
            panic!("expected expr stmt");
        };
        assert!(matches!(expr, Expr::IndexAssign { .. }));
    }
}

#[test]
fn fannkuch_nv_uses_usize_sub_index_in_flip() {
    let rust = compile_fixture_typed("tests/perf/fannkuch.tish");
    if rust.contains("fn fannkuch_nv(") {
        let nv = rust.split("fn fannkuch_nv(").nth(1).unwrap();
        let nv = nv.split("fn run()").next().unwrap_or(nv);
        assert!(
            nv.contains("let ku =") && nv.contains("for _usize_flip_"),
            "flip loop should fuse to ku half-loop with usize swaps"
        );
        assert!(
            !nv.contains("let mut k2:") && !nv.contains("let mut temp:"),
            "fused flip loop should not emit k2/temp temps"
        );
    }
}
