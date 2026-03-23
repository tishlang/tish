#[cfg(test)]
mod tests {
    use std::io::Write;

    use tishlang_parser::parse;

    use crate::{compile_project_with_jsx, compile_with_jsx, JsxMode};

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
    fn jsx_text_whitespace_coalesced() {
        let src = r#"fn X() { return <p>First paragraph</p> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            js.contains("\"First paragraph\""),
            "expected \"First paragraph\" in output, got: {}",
            &js[..400.min(js.len())]
        );
        assert!(
            !js.contains("\"First\", \"paragraph\""),
            "text should be coalesced, not split"
        );
    }

    #[test]
    fn jsx_text_whitespace_coalesced_multiline() {
        let src = "fn App() {\n  return <p>First paragraph</p>\n}";
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            js.contains("\"First paragraph\""),
            "multiline: expected \"First paragraph\", got: {}",
            &js[..400.min(js.len())]
        );
    }

    #[test]
    fn jsx_text_punctuation_no_space() {
        // Punctuation (e.g. !) concatenates without space: "work!" not "work !"
        let src = r#"fn X() { return <p>work!</p> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(js.contains(r#""work!""#), "expected 'work!', got: {}", &js[..400.min(js.len())]);
    }

    #[test]
    fn jsx_text_emojis() {
        let src = r#"fn X() { return <p>hello 😔</p> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(js.contains("😔"), "expected emoji, got: {}", &js[..400.min(js.len())]);
    }

    #[test]
    fn jsx_text_whitespace_via_compile_project() {
        let dir = std::env::temp_dir().join("tishlang_compile_project_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.tish");
        let src = "fn App() {\n  return <p>First paragraph</p>\n}";
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(src.as_bytes()).unwrap();
        f.sync_all().unwrap();
        drop(f);
        let js = compile_project_with_jsx(&path, Some(&dir), false, JsxMode::LattishH)
            .expect("compile_project_with_jsx failed");
        assert!(
            js.contains("\"First paragraph\""),
            "compile_project: expected \"First paragraph\", got: {}",
            &js[..500.min(js.len())]
        );
        let _ = std::fs::remove_file(&path);
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
