//! `DeclareVar` + block scopes: function `let` shadows script-level names (bytecode VM).

use tishlang_bytecode::compile;
use tishlang_vm::run;

#[test]
fn declare_var_shadows_script_let_inside_fn() {
    let src = r#"
let x = 1
fn f() {
  let x = 2
  return x
}
let r = f()
console.log("script", x, "fn", r)
"#;
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    run(&chunk).expect("run");
}

#[test]
fn block_let_restores_outer_binding() {
    let src = r#"
let x = 1
{
  let x = 2
}
console.log(x)
"#;
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    run(&chunk).expect("run");
}
