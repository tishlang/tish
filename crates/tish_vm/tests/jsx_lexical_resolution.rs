//! Bytecode JSX resolves `h`, `Fragment` and component tags lexically, falling back to globals.

use tishlang_bytecode::compile;
use tishlang_vm::run;

fn run_src(src: &str) {
    let program = tishlang_parser::parse(src).expect("parse");
    let chunk = compile(&program).expect("compile");
    run(&chunk).expect("run");
}

#[test]
fn component_tag_resolves_script_level_fn() {
    run_src(
        r#"
fn h(tag, props, children) {
  if (typeof tag === "function") { return tag(props) }
  return { tag: tag, props: props, children: children }
}
fn Row(props) { return <item title={props.title} /> }
let tree = <list><Row title="a" /></list>
let row = tree.children[0]
if (row.tag !== "item" || row.props.title !== "a") { throw "component tag did not resolve" }
"#,
    );
}

#[test]
fn h_and_fragment_resolve_from_enclosing_scope() {
    run_src(
        r#"
fn render() {
  let Fragment = "frag"
  let h = (tag, props, children) => typeof tag === "function" ? tag(props) : { tag: tag, children: children }
  fn Inner(p) { return <leaf /> }
  return <><Inner /></>
}
let t = render()
if (t.tag !== "frag" || t.children[0].tag !== "leaf") { throw "local h/Fragment/component not used" }
"#,
    );
}
