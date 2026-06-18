#[cfg(test)]
mod tests {
    use std::io::Write;

    use tishlang_parser::parse;

    use crate::{compile_project_esm, compile_project_with_jsx, compile_with_jsx, EmittedJsModule};

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
    fn jsx_keyword_text_after_child_element() {
        // #108: a text run *after* a nested child element must be lexed as JSX text, so a bare
        // reserved keyword (`as`, `in`, `if`, `return`, `let`) in that run is plain text, not a
        // keyword token. Text-only children already worked; the bug was failing to re-enter
        // JSX-text mode once a child element had closed.
        for kw in ["as", "in", "if", "return", "let"] {
            let src = format!("fn V() {{ return <div><span>x</span> {kw} JSON</div> }}");
            let program =
                parse(&src).unwrap_or_else(|e| panic!("parse failed for trailing `{kw}`: {e}"));
            let js = compile_with_jsx(&program, false).unwrap();
            let expected = format!("[h(\"span\", null, [\"x\"]), \" {kw} JSON\"]");
            assert!(
                js.contains(&expected),
                "trailing `{kw}` after child element: expected {expected:?} in output, got: {js}"
            );
        }
    }

    #[test]
    fn jsx_text_between_and_after_multiple_children() {
        // Text both between and after child elements stays text (incl. a keyword run between).
        let src = r#"fn V() { return <div><span>x</span> as <b>y</b> in z</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains(
                "[h(\"span\", null, [\"x\"]), \" as \", h(\"b\", null, [\"y\"]), \" in z\"]"
            ),
            "{js}"
        );
    }

    #[test]
    fn jsx_self_closing_child_then_keyword_text() {
        // A self-closing child (`<br/>`) followed by keyword text must also re-enter text mode.
        let src = r#"fn V() { return <div><br/> as JSON</div> }"#;
        let program = parse(src).unwrap();
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(
            js.contains("[h(\"br\", null, []), \" as JSON\"]"),
            "{js}"
        );
    }

    #[test]
    fn jsx_child_element_inside_expr_container_not_text_mode() {
        // Re-entering JSX text mode after a child closes must NOT happen when that child lived
        // inside a `{…}` expression container — otherwise the `)`/`,` that follows is swallowed as
        // JsxText ("Expected RParen, got JsxText"). Regression for the #108 fix itself, caught by
        // the downstream suite (tish-audio / tish-midi use `{items.map(x => <li>{x}</li>)}`).
        let src = r#"fn V(items) { return <ul>{items.map(x => <li>{x}</li>)}</ul> }"#;
        let program = parse(src).expect("map-in-container must parse");
        let js = compile_with_jsx(&program, false).unwrap();
        assert!(js.contains("h(\"ul\""), "{js}");
        assert!(js.contains(".map("), "{js}");

        // And the combined case: a `{…}` container followed by trailing keyword text.
        let src2 = r#"fn V(items) { return <div>{items.map(x => <span>{x}</span>)} as JSON</div> }"#;
        let program2 = parse(src2).expect("container-then-text must parse");
        let js2 = compile_with_jsx(&program2, false).unwrap();
        assert!(js2.contains("\" as JSON\""), "{js2}");
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

    #[test]
    fn fn_body_two_lets_not_split_by_closing_brace() {
        let src = "fn h() {\n  let a = 1\n  let b = 2\n}\n";
        let program = parse(src).expect("parse");
        let js = compile_with_jsx(&program, false).expect("compile");
        let i = js.find("let a = 1").expect("let a");
        let j = js.find("let b = 2").expect("let b");
        assert!(
            !js[i..j].contains('}'),
            "first let must not end in an inner block before second let (regression #43): {:?}",
            &js[i..j]
        );
    }

    #[test]
    fn control_flow_wraps_lexical_decl_body_in_block_for_valid_js() {
        let src = r#"fn f() {
  if (true)
    const x = 1
  while (false)
    let y = 2
  for (;;)
    const z = 3
  for (const v of [])
    let w = 4
}"#;
        let program = parse(src).expect("parse");
        let js = compile_with_jsx(&program, false).expect("compile");
        for (label, key, decl) in [
            ("if", "if (true)", "const x = 1"),
            ("while", "while (false)", "let y = 2"),
            ("for", "for (; ; )", "const z = 3"),
            ("for-of", "for (const v of [])", "let w = 4"),
        ] {
            let i = js.find(key).expect(label);
            let j = js.find(decl).expect(label);
            assert!(
                i < j && js[i..j].contains('{'),
                "{label}: expected '{{' between {key:?} and {decl:?}, got {:?}",
                &js[i..j]
            );
        }
    }

    // tish `=== null` / `!== null` lower to JS `== null` / `!= null` so the nullish check catches the
    // JS-runtime `undefined` (missing props / holes) too — matching interp/vm/native, which read a
    // missing property back as null. Strict equality between non-null operands stays strict.
    #[test]
    fn strict_eq_null_lowers_to_loose_null() {
        let program = parse("let x = 1\nconsole.log(x === null)\nconsole.log(x !== null)\n").unwrap();
        let js = crate::compile(&program, false).unwrap();
        assert!(!js.contains("=== null"), "`=== null` must lower to `== null`:\n{js}");
        assert!(!js.contains("!== null"), "`!== null` must lower to `!= null`:\n{js}");
        assert!(
            js.contains("== null") && js.contains("!= null"),
            "expected loose null checks:\n{js}"
        );
    }

    #[test]
    fn strict_eq_between_non_null_operands_stays_strict() {
        let program = parse("let a = 1\nlet b = 2\nconsole.log(a === b)\nconsole.log(a !== b)\n").unwrap();
        let js = crate::compile(&program, false).unwrap();
        assert!(js.contains("==="), "non-null `===` must stay strict:\n{js}");
        assert!(js.contains("!=="), "non-null `!==` must stay strict:\n{js}");
    }

    // `typeof null` is "null" in tish (interp/vm/native agree — null is a first-class type), not JS's
    // `typeof null === "object"` wart. The JS backend must map a nullish operand to "null".
    #[test]
    fn typeof_null_emits_null_not_object() {
        let program = parse("console.log(typeof null)\n").unwrap();
        let js = crate::compile(&program, false).unwrap();
        assert!(!js.contains("(typeof null)"), "must not emit raw `typeof null`:\n{js}");
        assert!(js.contains("\"null\""), "typeof of a nullish value must yield \"null\":\n{js}");
    }

    // 1/0 and -1/0 fold to Infinity / -Infinity; emit the JS spellings, not Rust's `inf` / `-inf`
    // (which would be undefined identifiers in the output).
    #[test]
    fn non_finite_number_literals_use_js_spellings() {
        let pos = crate::compile(&parse("console.log(1 / 0)\n").unwrap(), true).unwrap();
        assert!(pos.contains("Infinity"), "1/0 must emit Infinity:\n{pos}");
        assert!(!pos.contains("inf"), "must not emit Rust's lowercase `inf`:\n{pos}");
        let neg = crate::compile(&parse("console.log(-1 / 0)\n").unwrap(), true).unwrap();
        assert!(neg.contains("-Infinity"), "-1/0 must emit -Infinity:\n{neg}");
    }

    // ── #282: ESM module output (one file per module, real import/export) ──────────────────────

    /// Write a set of `(relative_path, source)` modules into a fresh temp dir and compile the entry
    /// in ESM mode. Returns the emitted modules keyed for easy lookup by their output relative path.
    fn build_esm(entry: &str, modules: &[(&str, &str)]) -> Vec<EmittedJsModule> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        for (rel, src) in modules {
            let p = dir.join(rel);
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut f = std::fs::File::create(&p).unwrap();
            f.write_all(src.as_bytes()).unwrap();
            f.sync_all().unwrap();
        }
        compile_project_esm(&dir.join(entry), Some(dir), false).expect("compile_project_esm failed")
    }

    fn module_js<'a>(mods: &'a [EmittedJsModule], rel: &str) -> &'a str {
        mods.iter()
            .find(|m| m.relative_path.to_string_lossy() == rel)
            .map(|m| m.js.as_str())
            .unwrap_or_else(|| panic!("module {rel} not emitted; got {:?}", mods.iter().map(|m| m.relative_path.display().to_string()).collect::<Vec<_>>()))
    }

    #[test]
    fn esm_emits_named_and_default_exports() {
        let mods = build_esm(
            "main.tish",
            &[(
                "main.tish",
                "export const VERSION = \"1.0\"\nexport fn greet(n) { return \"hi \" + n }\nexport default greet\n",
            )],
        );
        let js = module_js(&mods, "main.js");
        assert!(js.contains("export const VERSION = \"1.0\";"), "named const export:\n{js}");
        assert!(js.contains("export function greet "), "named fn export:\n{js}");
        assert!(js.contains("export default greet;"), "default export:\n{js}");
    }

    #[test]
    fn esm_rewrites_import_specifier_to_js_with_alias() {
        let mods = build_esm(
            "main.tish",
            &[
                ("dep.tish", "export const ssrH = 42\nexport fn greet(n) { return n }\n"),
                (
                    "main.tish",
                    "import { ssrH as h, greet } from \"./dep.tish\"\nimport * as M from \"./dep.tish\"\nconsole.log(h)\nconsole.log(greet(M.ssrH))\n",
                ),
            ],
        );
        let js = module_js(&mods, "main.js");
        assert!(
            js.contains("import { ssrH as h, greet } from \"./dep.js\";"),
            "named import with alias, .tish->.js:\n{js}"
        );
        assert!(js.contains("import * as M from \"./dep.js\";"), "namespace import:\n{js}");
    }

    #[test]
    fn esm_one_file_per_module_preserves_tree() {
        let mods = build_esm(
            "main.tish",
            &[
                ("lib/util.tish", "export fn id(x) { return x }\n"),
                ("main.tish", "import { id } from \"./lib/util.tish\"\nconsole.log(id(1))\n"),
            ],
        );
        // Nested module keeps its relative path; importer points at the nested `.js`.
        let _ = module_js(&mods, "lib/util.js");
        let main = module_js(&mods, "main.js");
        assert!(
            main.contains("from \"./lib/util.js\";"),
            "nested relative import preserved:\n{main}"
        );
    }

    #[test]
    fn esm_module_outside_project_root_is_emitted() {
        // #282 follow-up: a dependency in a *sibling* package (outside the entry's project root)
        // must still be emitted — the output tree is rooted at the directory common to all modules.
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();
        std::fs::create_dir_all(base.join("app/src")).unwrap();
        std::fs::create_dir_all(base.join("lib")).unwrap();
        std::fs::write(base.join("lib/util.tish"), "export fn id(x) { return x }\n").unwrap();
        std::fs::write(
            base.join("app/src/main.tish"),
            "import { id } from \"../../lib/util.tish\"\nconsole.log(id(1))\n",
        )
        .unwrap();
        // Project root is the entry's package (`app`); `lib/util.tish` lives outside it.
        let mods = compile_project_esm(
            &base.join("app/src/main.tish"),
            Some(&base.join("app")),
            false,
        )
        .expect("compile_project_esm failed for sibling dep");
        let rels: Vec<String> = mods
            .iter()
            .map(|m| m.relative_path.to_string_lossy().replace('\\', "/"))
            .collect();
        let main_js = mods
            .iter()
            .find(|m| m.relative_path.to_string_lossy().replace('\\', "/").ends_with("app/src/main.js"))
            .map(|m| m.js.clone());
        assert!(
            rels.iter().any(|r| r.ends_with("lib/util.js")),
            "sibling dep emitted under common base: {:?}",
            rels
        );
        assert!(
            rels.iter().any(|r| r.ends_with("app/src/main.js")),
            "entry emitted under its own subtree: {:?}",
            rels
        );
        let main_js = main_js.expect("entry module present");
        assert!(
            main_js.contains("from \"../../lib/util.js\";"),
            "relative import to sibling rewritten to .js:\n{main_js}"
        );
    }

    #[test]
    fn esm_bare_node_modules_dep_is_emitted() {
        // #282 follow-up: a bare specifier resolved from `node_modules` (like `lattish`) is emitted
        // into the output tree and the importer points at it with a relative `.js` specifier.
        let tmp = tempfile::tempdir().expect("tempdir");
        let base = tmp.path();
        std::fs::create_dir_all(base.join("src")).unwrap();
        std::fs::create_dir_all(base.join("node_modules/pkg")).unwrap();
        std::fs::write(
            base.join("node_modules/pkg/package.json"),
            "{\"name\":\"pkg\",\"main\":\"index.tish\"}\n",
        )
        .unwrap();
        std::fs::write(
            base.join("node_modules/pkg/index.tish"),
            "export fn ping() { return \"pong\" }\n",
        )
        .unwrap();
        std::fs::write(
            base.join("src/main.tish"),
            "import { ping } from \"pkg\"\nconsole.log(ping())\n",
        )
        .unwrap();
        let mods = compile_project_esm(&base.join("src/main.tish"), Some(base), false)
            .expect("compile_project_esm failed for node_modules dep");
        let rels: Vec<String> = mods
            .iter()
            .map(|m| m.relative_path.to_string_lossy().replace('\\', "/"))
            .collect();
        let main_js = mods
            .iter()
            .find(|m| m.relative_path.to_string_lossy().replace('\\', "/").ends_with("src/main.js"))
            .map(|m| m.js.clone());
        assert!(
            rels.iter().any(|r| r.ends_with("node_modules/pkg/index.js")),
            "node_modules dep emitted: {:?}",
            rels
        );
        let main_js = main_js.expect("entry module present");
        assert!(
            main_js.contains("from \"../node_modules/pkg/index.js\";"),
            "bare specifier rewritten to relative .js path:\n{main_js}"
        );
    }

    #[test]
    fn esm_rejects_native_imports() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        let p = dir.join("main.tish");
        std::fs::write(&p, "import { readFile } from \"fs\"\nconsole.log(1)\n").unwrap();
        let err = compile_project_esm(&p, Some(dir), false).unwrap_err();
        assert!(
            err.message.contains("Native module import") && err.message.contains("esm"),
            "expected a native-import rejection for ESM, got: {}",
            err.message
        );
    }
}
