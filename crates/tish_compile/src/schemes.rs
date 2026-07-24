//! Import-scheme registry — the extension seam for custom import prefixes.
//!
//! An *import scheme* is a prefix like `asset:` that means "don't resolve this as a `.tish` module
//! or npm package; instead resolve it my way and inject my code." tish core ships **no** built-in
//! schemes — it is target-agnostic (see [`SchemeRegistry::builtin`]). Schemes are contributed by
//! cargo path-dependencies that ship a `tish.schemes.json` (this is how `tish-agb` provides
//! `asset:`/`sheet:`/`background:`/`wav:`/`map:` for free to any game that depends on it) and by a
//! project's own `package.json` → `tish.schemes`, with no changes to the compiler. This is the same
//! philosophy as `cargo:` imports (config-declared, zero core edits per crate), generalized.
//!
//! A scheme is described declaratively:
//! * `prefix` — the scheme name (`asset` → matches `asset:...`).
//! * `resolve_file` — resolve the rest as a project-relative file and validate it exists.
//! * `targets` — per emit-target ("gba", …) code templates: the `use`s + body of a generated
//!   `mod`, an accessor, and a registration call. Templates substitute `{path}` (the resolved
//!   absolute path, as a Rust string literal) and `{mod}` (the generated module's identifier).
//!
//! Only *file-baking* schemes are expressible this way today — the family `asset:` belongs to
//! (resolve a file → emit a macro + a registration + bind an i32 handle). Schemes needing
//! compile-time logic (e.g. parsing aseprite tags) will want a code-plugin surface later.
//!
//! The active registry is compile-scoped: [`set_active`] installs it at the start of a build and
//! the resolve/codegen predicates read it via [`with_active`]. It defaults to [`builtin`] so any
//! path that doesn't install one still sees the built-in schemes (matching prior behaviour).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

/// What an imported name binds to. Only `Handle` (an `i32` index into a runtime arena) exists
/// today; kept as an enum so richer bindings (typed structs, const data) can be added later.
#[derive(Clone, Debug, PartialEq)]
pub enum BindKind {
    /// The imported name is an `i32` handle — its index among the scheme's registered files.
    Handle,
}

/// One emit target's code templates for a scheme (e.g. the `gba` recipe for `asset:`).
#[derive(Clone, Debug)]
pub struct TargetEmit {
    /// `use` lines placed at the top of the generated `mod`.
    pub uses: Vec<String>,
    /// The module body — typically the include macro. `{path}` → the file as a Rust string literal.
    pub module_body: String,
    /// A public accessor emitted inside the same `mod` (so it can see the macro's private statics).
    pub accessor: String,
    /// Registration call emitted in the entry before `run()`. `{mod}` → the generated module ident.
    pub register: String,
    /// What each imported name binds to.
    pub bind: BindKind,
}

/// A declaratively-described import scheme.
#[derive(Clone, Debug)]
pub struct SchemeDef {
    /// The prefix WITHOUT the trailing colon (`asset`); matches specs starting `asset:`.
    pub name: String,
    /// Resolve the part after the colon as a project-relative file (canonicalize + validate exists).
    pub resolve_file: bool,
    /// Per emit-target code templates.
    pub targets: HashMap<String, TargetEmit>,
}

impl SchemeDef {
    /// The spec prefix including the colon (`asset:`).
    pub fn prefix(&self) -> String {
        format!("{}:", self.name)
    }
}

/// The set of active import schemes for a build.
#[derive(Clone, Debug, Default)]
pub struct SchemeRegistry {
    pub schemes: Vec<SchemeDef>,
}

impl SchemeRegistry {
    /// The schemes tish core ships with — **none**. tish core is target-agnostic and knows nothing
    /// about agb/GBA; concrete schemes (like `asset:`) are contributed by the crates that implement
    /// their runtime (see [`from_project`](Self::from_project)). Kept as the seam for any future
    /// genuinely-generic built-in scheme.
    pub fn builtin() -> Self {
        SchemeRegistry {
            schemes: Vec::new(),
        }
    }

    /// The active registry for a build: schemes contributed by cargo dependencies, then the
    /// project's own, merged in that order (project overrides a dependency of the same name).
    ///
    /// * **Dependency-contributed** — each `tish.rustDependencies` entry with a `path` may ship a
    ///   `tish.schemes.json` (`{ "schemes": { … } }`) in its crate dir. This is how tish-agb
    ///   provides `asset:` without any tish-core edits: a game that depends on tish-agb (as every
    ///   sprite game already does) gets the scheme for free, no per-game config.
    /// * **Project-local** — `package.json` → `tish.schemes`, for one-off or overriding schemes.
    ///
    /// A malformed entry is skipped, not fatal.
    pub fn from_project(project_root: &Path) -> Self {
        let mut reg = Self::builtin();
        let tish = crate::resolve::read_project_tish_config(project_root);
        // 1. Schemes contributed by cargo path-dependencies.
        if let Some(deps) = tish.get("rustDependencies").and_then(|v| v.as_object()) {
            for val in deps.values() {
                let Some(path) = val
                    .as_object()
                    .and_then(|o| o.get("path"))
                    .and_then(|p| p.as_str())
                else {
                    continue;
                };
                let p = Path::new(path);
                let dep_dir = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    project_root.join(p)
                };
                let manifest = dep_dir.join("tish.schemes.json");
                if let Ok(content) = std::fs::read_to_string(&manifest) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        reg.merge_schemes(json.get("schemes"));
                    }
                }
            }
        }
        // 2. Project-local schemes (override dependency-contributed ones of the same name).
        reg.merge_schemes(tish.get("schemes"));
        reg
    }

    /// Merge a JSON `{ name: def, … }` schemes object into the registry; a same-named scheme
    /// replaces the existing one.
    fn merge_schemes(&mut self, schemes: Option<&serde_json::Value>) {
        let Some(obj) = schemes.and_then(|v| v.as_object()) else {
            return;
        };
        for (name, def) in obj {
            if let Some(parsed) = parse_scheme_def(name, def) {
                self.schemes.retain(|s| s.name != parsed.name);
                self.schemes.push(parsed);
            }
        }
    }

    /// The scheme whose prefix `spec` starts with, if any.
    pub fn matches(&self, spec: &str) -> Option<&SchemeDef> {
        self.schemes.iter().find(|s| spec.starts_with(&s.prefix()))
    }
}

/// Parse one `tish.schemes["name"]` JSON entry into a [`SchemeDef`]. Returns `None` (skip) on a
/// shape that doesn't describe a valid scheme.
fn parse_scheme_def(name: &str, def: &serde_json::Value) -> Option<SchemeDef> {
    // The name is interpolated into a generated module identifier (`mod __scheme_<name>_<j>`) and
    // used as the `<name>:` import prefix, so it must be a valid Rust identifier. A name like
    // `sprite-sheet` would emit un-compilable Rust (a raw rustc error far from its cause); skip it
    // instead, consistent with "a malformed entry is skipped, not fatal".
    if !is_valid_scheme_name(name) {
        return None;
    }
    let obj = def.as_object()?;
    let resolve_file = obj.get("file").and_then(|v| v.as_bool()).unwrap_or(true);
    let mut targets = HashMap::new();
    if let Some(target_map) = obj.get("targets").and_then(|v| v.as_object()) {
        for (target, spec) in target_map {
            let s = spec.as_object()?;
            let uses = s
                .get("uses")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let module_body = s.get("module").and_then(|v| v.as_str())?.to_string();
            let accessor = s
                .get("accessor")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let register = s
                .get("register")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            targets.insert(
                target.clone(),
                TargetEmit {
                    uses,
                    module_body,
                    accessor,
                    register,
                    bind: BindKind::Handle,
                },
            );
        }
    }
    Some(SchemeDef {
        name: name.to_string(),
        resolve_file,
        targets,
    })
}

/// A scheme name must be a valid Rust identifier: it becomes part of a generated module ident
/// (`__scheme_<name>_<j>`) and the `<name>:` import prefix. ASCII letter/underscore, then
/// letters/digits/underscores. (The same shape `types::is_struct_field_safe` enforces for native
/// struct field keys, kept local so `schemes` stays self-contained.)
fn is_valid_scheme_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

thread_local! {
    static ACTIVE: RefCell<SchemeRegistry> = RefCell::new(SchemeRegistry::builtin());
}

/// Install the registry for the current build. Call once at the start of a compile, before
/// resolution — every scheme predicate reads whatever is installed here.
pub fn set_active(registry: SchemeRegistry) {
    ACTIVE.with(|c| *c.borrow_mut() = registry);
}

/// Read from the active registry.
pub fn with_active<R>(f: impl FnOnce(&SchemeRegistry) -> R) -> R {
    ACTIVE.with(|c| f(&c.borrow()))
}

/// True if `spec` is a registered import scheme (built-in or project-declared).
pub fn is_scheme_import(spec: &str) -> bool {
    with_active(|r| r.matches(spec).is_some())
}

/// Substitute `{path}` (the file as a Rust string literal) and `{mod}` (the generated module ident)
/// in a template.
pub fn render_template(template: &str, abspath: &str, module_ident: &str) -> String {
    template
        .replace("{path}", &format!("{:?}", abspath))
        .replace("{mod}", module_ident)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheme_name_must_be_a_rust_identifier() {
        // Valid identifiers pass.
        assert!(is_valid_scheme_name("asset"));
        assert!(is_valid_scheme_name("sprite_sheet"));
        assert!(is_valid_scheme_name("_x"));
        assert!(is_valid_scheme_name("bg2"));
        // Anything that would break the generated `mod __scheme_<name>_<j>` is rejected.
        assert!(!is_valid_scheme_name("sprite-sheet")); // hyphen
        assert!(!is_valid_scheme_name("")); // empty
        assert!(!is_valid_scheme_name("2d")); // leading digit
        assert!(!is_valid_scheme_name("a:b")); // colon
        assert!(!is_valid_scheme_name("a b")); // space
    }

    #[test]
    fn parse_scheme_def_skips_invalid_name() {
        let def = serde_json::json!({
            "file": true,
            "targets": { "gba": { "module": "// x", "register": "" } }
        });
        // A hyphenated name is skipped (returns None) rather than emitting un-compilable Rust.
        assert!(parse_scheme_def("sprite-sheet", &def).is_none());
        // The same def under a valid name parses.
        assert!(parse_scheme_def("sheet", &def).is_some());
    }
}
