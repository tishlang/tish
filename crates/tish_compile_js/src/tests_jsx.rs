#[cfg(test)]
mod tests {
    use tish_parser::parse;

    use crate::{compile_with_jsx, JsxMode};

    #[test]
    fn tishact_jsx_emits_h_with_children_array() {
        let src = r#"fn X() { return <div class="a">{"hi"}</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::TishactH).unwrap();
        assert!(js.contains("h(\"div\", { class: \"a\" }, [\"hi\"])"), "{}", js);
        assert!(!js.contains("function __h("));
    }

    #[test]
    fn legacy_jsx_emits_preamble_h() {
        let src = r#"fn X() { return <span/> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LegacyDom).unwrap();
        assert!(js.contains("function __h("), "{}", &js[..400.min(js.len())]);
        assert!(js.contains("__h(\"span\", null)"));
    }

    #[test]
    fn fragment_tishact_uses_fragment_symbol() {
        let src = "fn X() { return <><b>{\"1\"}</b></> }";
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::TishactH).unwrap();
        assert!(js.contains("h(Fragment, null, ["));
    }

    #[test]
    fn vdom_emits_vdom_h() {
        let src = r#"fn X() { return <p/> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::Vdom).unwrap();
        assert!(js.contains("__vdom_h(\"p\", null, [])"), "{}", &js[..600.min(js.len())]);
        assert!(js.contains("__tishactVdomPatch"));
    }
}
