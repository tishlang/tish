#[cfg(test)]
mod tests {
    use std::io::Write;

    use tishlang_parser::parse;

    use crate::{compile_project_with_jsx, compile_with_jsx};

    #[test]
    fn lattish_jsx_emits_h_with_children_array() {
        let src = r#"fn X() { return <div class="a">{"hi"}</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains("h(\"div\", { class: \"a\" }, [\"hi\"])"),
            "{}",
            js
        );
        assert!(!js.contains("function __h("));
    }

    #[test]
    fn fragment_lattish_uses_fragment_symbol() {
        let src = "fn X() { return <><b>{\"1\"}</b></> }";
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(js.contains("h(Fragment, null, ["));
    }

    #[test]
    fn jsx_text_whitespace_coalesced() {
        let src = r#"fn X() { return <p>First paragraph</p> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains(r#""work!""#),
            "expected 'work!', got: {}",
            &js[..400.min(js.len())]
        );
    }

    #[test]
    fn jsx_text_emojis() {
        let src = r#"fn X() { return <p>hello 😔</p> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains("😔"),
            "expected emoji, got: {}",
            &js[..400.min(js.len())]
        );
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
        let js = compile_project_with_jsx(&path, Some(&dir), false)
            .expect("compile_project_with_jsx failed");
        assert!(
            js.contains("\"First paragraph\""),
            "compile_project: expected \"First paragraph\", got: {}",
            &js[..500.min(js.len())]
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn jsx_never_emits_vdom_helpers_or_prelude_flags() {
        let src = r#"fn X() { return <div class="x">{"a"}</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains("h(\"div\", { class: \"x\" }"),
            "{}",
            &js[..500.min(js.len())]
        );
        assert!(!js.contains("__vdom_h"), "{}", &js[..600.min(js.len())]);
        assert!(
            !js.contains("window.__LATTISH_JSX_VDOM"),
            "{}",
            &js[..600.min(js.len())]
        );
        assert!(
            !js.contains("__lattishVdomPatch"),
            "{}",
            &js[..600.min(js.len())]
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
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).unwrap();
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
        let js = compile_with_jsx(&program, false).expect("compile");
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
        let js = compile_with_jsx(&program, false).expect("compile");
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

    #[test]
    fn new_date_global_emits_valid_js_with_and_without_optimize() {
        let src = "let epoch = new Date(0)\nconsole.log(epoch.getTime())";
        let program = parse(src).expect("parse");
        for optimize in [false, true] {
            let js = compile_with_jsx(&program, optimize).expect("compile");
            assert!(
                js.contains("new Date(0)"),
                "optimize={optimize}: expected `new Date(0)` in JS output:\n{js}"
            );
            assert!(
                !js.contains("let epoch = new;"),
                "optimize={optimize}: broken `new` emission (missing constructor):\n{js}"
            );
        }
    }

    #[test]
    fn new_uint8array_emits_direct_new_no_preamble() {
        let src = "fn f(n) { return new Uint8Array(n) }";
        let program = parse(src).expect("parse");
        let js = compile_with_jsx(&program, false).expect("compile");
        assert!(
            js.contains("new Uint8Array("),
            "expected direct new Uint8Array, got: {}",
            &js[..500.min(js.len())]
        );
        assert!(
            !js.contains("__tishUint8Array"),
            "should not emit legacy intrinsic helper"
        );
    }

    #[test]
    fn new_audio_context_emits_direct_new_no_preamble() {
        let src = "fn f() { return new AudioContext() }";
        let program = parse(src).expect("parse");
        let js = compile_with_jsx(&program, false).expect("compile");
        assert!(
            js.contains("new AudioContext("),
            "expected new AudioContext, got: {}",
            &js[..500.min(js.len())]
        );
        assert!(
            !js.contains("__tishWebAudioCreateContext"),
            "should not emit legacy intrinsic helper"
        );
    }

    #[test]
    fn new_class_name_emits_direct_new_js() {
        let src = r#"
fn ClassName(x) {
    return x
}
fn factory() {
    return new ClassName(42)
}
"#;
        let program = parse(src).expect("parse");
        let js = compile_with_jsx(&program, false).expect("compile");
        assert!(
            js.contains("new ClassName("),
            "expected new ClassName( in JS output, got: {}",
            &js[..800.min(js.len())]
        );
    }
}
