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

    /// Docs: `--jsx lattish` → `h(...)`, no VDOM prelude; `--jsx vdom` → `__vdom_h` + prelude.
    /// This locks the contract so README / LATTISH.md stay honest.
    #[test]
    fn jsx_lattish_vs_vdom_compile_output_matches_documentation() {
        let src = r#"fn X() { return <div class="x">{"a"}</div> }"#;
        let program = parse(src).unwrap();

        let lattish = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            lattish.contains("h(\"div\", { class: \"x\" }"),
            "lattish mode should emit h(tag, props, …), got: {}",
            &lattish[..500.min(lattish.len())]
        );
        assert!(
            !lattish.contains("__vdom_h"),
            "lattish mode must not emit __vdom_h"
        );
        assert!(
            !lattish.contains("window.__LATTISH_JSX_VDOM"),
            "lattish mode must not inject VDOM prelude flag"
        );

        let vdom = compile_with_jsx(&program, false, JsxMode::Vdom).unwrap();
        assert!(
            vdom.contains("__vdom_h(\"div\", { class: \"x\" }"),
            "vdom mode should emit __vdom_h(…), got: {}",
            &vdom[..700.min(vdom.len())]
        );
        assert!(
            vdom.contains("window.__LATTISH_JSX_VDOM"),
            "vdom mode must set __LATTISH_JSX_VDOM so Lattish createRoot uses patch"
        );
        assert!(
            vdom.contains("__lattishVdomPatch"),
            "vdom prelude must define __lattishVdomPatch"
        );
        // `__vdom_h("div", …)` contains the substring `h("div", …)` — require the real callee prefix.
        let needle = "(\"div\", { class: \"x\" }";
        let pos = vdom.find(needle).expect("expected div+class in vdom output");
        let ctx_start = pos.saturating_sub(30);
        let ctx_end = (pos + needle.len() + 8).min(vdom.len());
        assert!(
            vdom[..pos].ends_with("__vdom_h"),
            "opening tag should be __vdom_h(\"div\", …), not h(\"div\", …); got {:?}",
            &vdom[ctx_start..ctx_end]
        );
    }

    /// Component calls like {Panel()} return DOM elements. Wrapping in String() produces [object HTMLDivElement].
    #[test]
    fn jsx_component_call_not_wrapped_in_string() {
        let src = r#"
fn Panel() { return <div class="p">content</div> }
fn App() { return <div>{Panel()}</div> }
"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            js.contains("Panel()"),
            "component call should appear as Panel(), got: {}",
            &js[..500.min(js.len())]
        );
        assert!(
            !js.contains("String(Panel()"),
            "component calls must NOT be wrapped in String() - causes [object HTMLDivElement]. got: {}",
            &js[..600.min(js.len())]
        );
    }

    /// Nested JSX elements must not be String()'d or they render as [object HTMLDivElement].
    #[test]
    fn jsx_nested_element_not_wrapped_in_string() {
        let src = r#"fn X() { return <div><span>inner</span></div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            !js.contains("String(h("),
            "nested JSX elements must NOT be wrapped in String(). got: {}",
            &js[..500.min(js.len())]
        );
    }

    /// Literal number/bool/null get String() for display. Idents (e.g. {items}) are NOT wrapped—they may hold elements.
    #[test]
    fn jsx_literal_number_wrapped_in_string() {
        let src = r#"fn X() { return <span>{42}</span> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            js.contains("String(42)"),
            "literal number in JSX should be wrapped in String(). got: {}",
            &js[..500.min(js.len())]
        );
    }

    /// Array/ident like {items} (array of buttons) must NOT be String()'d or we get [object HTMLButtonElement].
    #[test]
    fn jsx_array_of_elements_not_wrapped_in_string() {
        let src = r#"
fn FileList() {
  let items = []
  items.push(<button>a</button>)
  items.push(<button>b</button>)
  return <div>{items}</div>
}
"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).unwrap();
        assert!(
            !js.contains("String(items)"),
            "array/ident in JSX must NOT be wrapped in String() - causes [object HTMLButtonElement]. got: {}",
            &js[..600.min(js.len())]
        );
    }

    /// `>` inside `{ ... }` attribute values must be a comparison operator, not end of opening tag.
    #[test]
    fn jsx_gt_comparison_inside_attribute_expression() {
        let src = r#"fn X() {
  return <button
    type="button"
    onclick={() => {
      let nm = "a"
      if (nm && nm.length > 0) { print(nm) }
    }}
  >{"ok"}</button>
}"#;
        let program = parse(src).expect("parse multi-line JSX with > comparison in attr");
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).expect("compile");
        assert!(
            js.contains("length > 0") || js.contains("length>0"),
            "expected compiled JS to preserve greater-than comparison, got: {}",
            &js[..800.min(js.len())]
        );
    }

    /// Nested JSX inside an attribute callback must still close inner `<tag>` correctly.
    #[test]
    fn jsx_nested_element_inside_attribute_expression() {
        let src = r#"fn X() {
  return <button
    onclick={() => {
      let x = <span>{"inner"}</span>
      print(x)
    }}
  >{"outer"}</button>
}"#;
        let program = parse(src).expect("parse nested JSX inside onclick");
        let js = compile_with_jsx(&program, false, JsxMode::LattishH).expect("compile");
        assert!(
            js.contains("\"inner\""),
            "expected nested span text in output, got: {}",
            &js[..900.min(js.len())]
        );
        assert!(
            js.contains("\"outer\""),
            "expected button child text in output, got: {}",
            &js[..900.min(js.len())]
        );
    }
}
