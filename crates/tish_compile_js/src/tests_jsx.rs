#[cfg(test)]
mod tests {
    use tish_parser::parse;

    use crate::{compile_with_jsx, JsxMode};

    #[test]
    fn lattish_jsx_emits_h_with_children_array() {
        let src = r#"fn X() { return <div class="a">{"hi"}</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(js.contains("h(\"div\", { class: \"a\" }, [\"hi\"])"), "{}", js);
        assert!(!js.contains("function __h("));
    }

    #[test]
    fn fragment_lattish_uses_fragment_symbol() {
        let src = "fn X() { return <><b>{\"1\"}</b></> }";
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(js.contains("h(Fragment, null, ["));
    }

    #[test]
    fn vdom_emits_vdom_h() {
        let src = r#"fn X() { return <p/> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::Vdom).unwrap();
        assert!(js.contains("__vdom_h(\"p\", null, [])"), "{}", &js[..600.min(js.len())]);
        assert!(js.contains("__lattishVdomPatch"));
    }
}
