//! Type system for Tish static typing.
//!
//! Maps TypeAnnotation from the AST to concrete Rust types for code generation.

use std::collections::HashMap;
use std::sync::Arc;
use tishlang_ast::{BinOp, FunParam, TypeAnnotation, TypedParam};

/// Concrete Rust type representation for code generation.
#[derive(Debug, Clone, PartialEq)]
pub enum RustType {
    /// Dynamic Value type (untyped or complex types)
    Value,
    /// f64 (for number)
    F64,
    /// String (for string)
    String,
    /// bool (for boolean)
    Bool,
    /// () for void/null
    Unit,
    /// Vec<T> for arrays
    Vec(Box<RustType>),
    /// Option<T> for nullable types (T | null)
    Option(Box<RustType>),
    /// Inline object shape — used during inference / annotation lowering
    /// before a `Named` alias has been registered. Once a corresponding
    /// `type Foo = { ... }` declaration is found in the program, occurrences
    /// of this shape can be canonicalised into `RustType::Named("Foo")`.
    Object(Vec<(Arc<str>, RustType)>),
    /// User-defined named type (a struct emitted by the compiler).
    /// The field list is duplicated here so the codegen can emit struct
    /// literals, member access, and Value-conversion glue without going
    /// back to a global registry on every call site.
    Named {
        name: Arc<str>,
        fields: Vec<(Arc<str>, RustType)>,
    },
    /// Fn trait for typed functions
    Function {
        params: Vec<RustType>,
        returns: Box<RustType>,
    },
}

impl RustType {
    /// Convert a TypeAnnotation to a RustType (no alias resolution).
    /// Use [`Self::from_annotation_with_aliases`] when a registry is
    /// available so user-defined `type X = { ... }` aliases land as
    /// `RustType::Named` and can drive struct emission.
    pub fn from_annotation(ann: &TypeAnnotation) -> Self {
        Self::from_annotation_with_aliases(ann, &HashMap::new())
    }

    /// Like [`from_annotation`], but consults `aliases` so a `Simple(name)`
    /// reference to a user-declared `type X = { ... }` resolves to a
    /// `RustType::Named { name, fields }` carrying the struct shape.
    pub fn from_annotation_with_aliases(
        ann: &TypeAnnotation,
        aliases: &HashMap<String, RustType>,
    ) -> Self {
        match ann {
            TypeAnnotation::Simple(name) => match name.as_ref() {
                "number" => RustType::F64,
                "string" => RustType::String,
                "boolean" | "bool" => RustType::Bool,
                "void" | "undefined" => RustType::Unit,
                "null" => RustType::Unit,
                "any" => RustType::Value,
                other => {
                    // User-declared `type X = { ... }`: lift the inline
                    // object shape into a `Named` so the codegen can emit
                    // a Rust struct and direct field access for it.
                    if let Some(t) = aliases.get(other) {
                        if let RustType::Object(fields) = t {
                            return RustType::Named {
                                name: Arc::from(other),
                                fields: fields.clone(),
                            };
                        }
                        return t.clone();
                    }
                    RustType::Value
                }
            },
            TypeAnnotation::Array(elem) => RustType::Vec(Box::new(
                Self::from_annotation_with_aliases(elem, aliases),
            )),
            TypeAnnotation::Object(fields) => {
                let typed_fields: Vec<_> = fields
                    .iter()
                    .map(|(k, v)| (k.clone(), Self::from_annotation_with_aliases(v, aliases)))
                    .collect();
                RustType::Object(typed_fields)
            }
            TypeAnnotation::Function { params, returns } => {
                let typed_params: Vec<_> = params
                    .iter()
                    .map(|p| Self::from_annotation_with_aliases(p, aliases))
                    .collect();
                let typed_returns = Box::new(Self::from_annotation_with_aliases(returns, aliases));
                RustType::Function {
                    params: typed_params,
                    returns: typed_returns,
                }
            }
            TypeAnnotation::Union(types) => {
                // Check for T | null pattern -> Option<T>
                if types.len() == 2 {
                    let has_null = types
                        .iter()
                        .any(|t| matches!(t, TypeAnnotation::Simple(s) if s.as_ref() == "null"));
                    if has_null {
                        let non_null = types.iter().find(
                            |t| !matches!(t, TypeAnnotation::Simple(s) if s.as_ref() == "null"),
                        );
                        if let Some(inner) = non_null {
                            return RustType::Option(Box::new(
                                Self::from_annotation_with_aliases(inner, aliases),
                            ));
                        }
                    }
                }
                // Other unions fall back to Value
                RustType::Value
            }
        }
    }

    /// Check if this type is a native Rust type (not Value).
    pub fn is_native(&self) -> bool {
        !matches!(self, RustType::Value)
    }

    /// Check if this type is numeric (f64).
    pub fn is_numeric(&self) -> bool {
        matches!(self, RustType::F64)
    }

    /// Infer the result type of a binary operation given the operand types.
    /// Returns `None` if native code cannot be emitted (fall back to Value path).
    pub fn result_type_of_binop(op: BinOp, lhs: &RustType, rhs: &RustType) -> Option<RustType> {
        if lhs == &RustType::F64 && rhs == &RustType::F64 {
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                    Some(RustType::F64)
                }
                BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::StrictEq
                | BinOp::StrictNe => Some(RustType::Bool),
                _ => None,
            }
        } else if lhs == &RustType::Bool && rhs == &RustType::Bool {
            match op {
                BinOp::And | BinOp::Or => Some(RustType::Bool),
                BinOp::StrictEq | BinOp::StrictNe => Some(RustType::Bool),
                _ => None,
            }
        } else {
            None
        }
    }

    /// Get the Rust type string for code generation.
    pub fn to_rust_type_str(&self) -> String {
        match self {
            RustType::Value => "Value".to_string(),
            RustType::F64 => "f64".to_string(),
            RustType::String => "String".to_string(),
            RustType::Bool => "bool".to_string(),
            RustType::Unit => "()".to_string(),
            RustType::Vec(inner) => format!("Vec<{}>", inner.to_rust_type_str()),
            RustType::Option(inner) => format!("Option<{}>", inner.to_rust_type_str()),
            RustType::Object(_) => {
                // Anonymous inline shapes don't have a Rust struct; fall
                // back to the dynamic Value path.
                "Value".to_string()
            }
            RustType::Named { name, .. } => named_struct_ident(name),
            RustType::Function { params, returns } => {
                let params_str: Vec<_> = params.iter().map(|p| p.to_rust_type_str()).collect();
                format!(
                    "Rc<dyn Fn({}) -> {}>",
                    params_str.join(", "),
                    returns.to_rust_type_str()
                )
            }
        }
    }

    /// Get the default value for this type.
    pub fn default_value(&self) -> String {
        match self {
            RustType::Value => "Value::Null".to_string(),
            RustType::F64 => "0.0".to_string(),
            RustType::String => "String::new()".to_string(),
            RustType::Bool => "false".to_string(),
            RustType::Unit => "()".to_string(),
            RustType::Vec(_) => "Vec::new()".to_string(),
            RustType::Option(_) => "None".to_string(),
            RustType::Object(_) => "Value::Null".to_string(),
            RustType::Named { fields, .. } => {
                // Build a literal struct with each field at its own default,
                // so unannotated decls of a typed struct still compile.
                let init = fields
                    .iter()
                    .map(|(k, t)| format!("{}: {}", field_ident(k), t.default_value()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} {{ {} }}",
                    named_struct_ident(match self {
                        RustType::Named { name, .. } => name,
                        _ => unreachable!(),
                    }),
                    init
                )
            }
            RustType::Function { .. } => "Value::Null".to_string(),
        }
    }

    /// Generate code to convert from Value to this native type.
    pub fn from_value_expr(&self, value_expr: &str) -> String {
        match self {
            RustType::Value => value_expr.to_string(),
            RustType::F64 => format!(
                "match &{} {{ Value::Number(n) => *n, _ => panic!(\"expected number\") }}",
                value_expr
            ),
            RustType::String => format!(
                "match &{} {{ Value::String(s) => s.to_string(), _ => panic!(\"expected string\") }}",
                value_expr
            ),
            RustType::Bool => format!(
                "match &{} {{ Value::Bool(b) => *b, _ => panic!(\"expected boolean\") }}",
                value_expr
            ),
            RustType::Unit => "()".to_string(),
            RustType::Vec(inner) => {
                let inner_conversion = inner.from_value_expr("v");
                format!(
                    "match &{} {{ Value::Array(arr) => arr.borrow().iter().map(|v| {}).collect(), _ => panic!(\"expected array\") }}",
                    value_expr, inner_conversion
                )
            }
            RustType::Option(inner) => {
                let inner_conversion = inner.from_value_expr(value_expr);
                format!(
                    "match &{} {{ Value::Null => None, _ => Some({}) }}",
                    value_expr, inner_conversion
                )
            }
            RustType::Named { name, fields } => {
                // Each field is fetched out of the Value::Object via
                // `get_prop` and converted to its native type. Falls back
                // to the field's `default_value()` if the field is absent
                // (rare — usually these come from JSON or PG).
                let field_assigns = fields
                    .iter()
                    .map(|(k, ty)| {
                        let fetch =
                            format!("tishlang_runtime::get_prop(&{}, {:?})", value_expr, k.as_ref());
                        format!("{}: {}", field_ident(k), ty.from_value_expr(&fetch))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{} {{ {} }}", named_struct_ident(name), field_assigns)
            }
            _ => value_expr.to_string(), // Fallback
        }
    }

    /// Generate code to convert from this native type to Value.
    pub fn to_value_expr(&self, native_expr: &str) -> String {
        match self {
            RustType::Value => native_expr.to_string(),
            RustType::F64 => format!("Value::Number({})", native_expr),
            RustType::String => format!("Value::String({}.clone().into())", native_expr),
            RustType::Bool => format!("Value::Bool({})", native_expr),
            RustType::Unit => "Value::Null".to_string(),
            RustType::Vec(inner) => {
                // Use iter()/copied()/cloned() to avoid moving the vector.
                let (iter_suffix, val_expr) = match inner.as_ref() {
                    RustType::F64 => (".iter().copied()", "Value::Number(v)".to_string()),
                    RustType::Bool => (".iter().copied()", "Value::Bool(v)".to_string()),
                    _ => (".iter().cloned()", inner.to_value_expr("v")),
                };
                format!(
                    "Value::Array(VmRef::new({}{}.map(|v| {}).collect()))",
                    native_expr, iter_suffix, val_expr
                )
            }
            RustType::Option(inner) => {
                let inner_to_value = inner.to_value_expr("v");
                format!(
                    "match {} {{ Some(v) => {}, None => Value::Null }}",
                    native_expr, inner_to_value
                )
            }
            RustType::Named { fields, .. } => {
                // Walk fields, build an ObjectMap, wrap in Value::Object.
                // The boundary is paid only when crossing into untyped
                // Tish (JSON.stringify, calling a Value::Function, etc.);
                // direct Rust-to-Rust paths between two Named values stay
                // as plain struct moves.
                let inserts = fields
                    .iter()
                    .map(|(k, ty)| {
                        let access = format!("{}.{}", native_expr, field_ident(k));
                        let v_expr = ty.to_value_expr(&access);
                        format!(
                            "_om.insert(::std::sync::Arc::from({:?}), {});",
                            k.as_ref(),
                            v_expr
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "{{ let mut _om = ObjectMap::default(); {} Value::Object(VmRef::new(_om)) }}",
                    inserts
                )
            }
            _ => native_expr.to_string(), // Fallback
        }
    }
}

/// Map a Tish type-alias name to the Rust struct identifier we emit.
/// Prefixed so user names can never collide with runtime types like `Value`.
pub fn named_struct_ident(tish_name: &str) -> String {
    format!("TishStruct_{}", tish_name)
}

/// Map a Tish field name (`randomNumber`) to a valid Rust identifier
/// (kept identical here — non-snake-case is allowed via
/// `#[allow(non_snake_case)]` on the struct, so JS-style camelCase keys
/// stay readable in the generated source).
pub fn field_ident(tish_name: &str) -> String {
    // Reserve Rust keywords that would otherwise conflict.
    match tish_name {
        "type" | "ref" | "fn" | "match" | "move" | "mod" | "self" | "Self" | "super" | "use"
        | "where" | "loop" | "yield" | "async" | "await" | "dyn" | "impl" | "trait" | "in"
        | "as" | "box" | "crate" | "const" | "extern" | "let" | "mut" | "pub" | "static"
        | "unsafe" | "abstract" | "become" | "do" | "final" | "macro" | "override" | "priv"
        | "typeof" | "unsized" | "virtual" => format!("r#{}", tish_name),
        _ => tish_name.to_string(),
    }
}

/// Type context for tracking variable types during code generation.
#[derive(Debug, Clone, Default)]
pub struct TypeContext {
    /// Stack of scopes, each mapping variable names to their types
    scopes: Vec<HashMap<String, RustType>>,
}

impl TypeContext {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()], // Start with global scope
        }
    }

    /// Enter a new scope (e.g., function body, block).
    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    /// Exit the current scope.
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Push a scope for a function or arrow body and record formals as [`RustType::Value`].
    ///
    /// Native codegen always binds parameters from `args.get(i)` as `Value`; this prevents
    /// outer locals (e.g. a loop counter inferred as [`RustType::F64`]) from shadowing the
    /// wrong type for the same identifier.
    pub fn push_fun_param_scope(&mut self, params: &[FunParam], rest_param: Option<&TypedParam>) {
        self.push_scope();
        for p in params {
            for name in p.bound_names() {
                self.define(name.as_ref(), RustType::Value);
            }
        }
        if let Some(rp) = rest_param {
            self.define(rp.name.as_ref(), RustType::Value);
        }
    }

    /// Define a variable in the current scope.
    pub fn define(&mut self, name: &str, ty: RustType) {
        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name.to_string(), ty);
        }
    }

    /// Look up a variable's type (searches from innermost to outermost scope).
    pub fn lookup(&self, name: &str) -> Option<&RustType> {
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return Some(ty);
            }
        }
        None
    }

    /// Check if a variable is typed (has a non-Value type).
    pub fn is_typed(&self, name: &str) -> bool {
        self.lookup(name).map(|ty| ty.is_native()).unwrap_or(false)
    }

    /// Get the type of a variable, defaulting to Value if not found.
    pub fn get_type(&self, name: &str) -> RustType {
        self.lookup(name).cloned().unwrap_or(RustType::Value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_types() {
        assert_eq!(
            RustType::from_annotation(&TypeAnnotation::Simple("number".into())),
            RustType::F64
        );
        assert_eq!(
            RustType::from_annotation(&TypeAnnotation::Simple("string".into())),
            RustType::String
        );
        assert_eq!(
            RustType::from_annotation(&TypeAnnotation::Simple("boolean".into())),
            RustType::Bool
        );
    }

    #[test]
    fn test_array_type() {
        let arr_type = TypeAnnotation::Array(Box::new(TypeAnnotation::Simple("number".into())));
        assert_eq!(
            RustType::from_annotation(&arr_type),
            RustType::Vec(Box::new(RustType::F64))
        );
    }

    #[test]
    fn test_nullable_type() {
        let nullable = TypeAnnotation::Union(vec![
            TypeAnnotation::Simple("string".into()),
            TypeAnnotation::Simple("null".into()),
        ]);
        assert_eq!(
            RustType::from_annotation(&nullable),
            RustType::Option(Box::new(RustType::String))
        );
    }

    #[test]
    fn test_type_context() {
        let mut ctx = TypeContext::new();
        ctx.define("x", RustType::F64);
        assert_eq!(ctx.get_type("x"), RustType::F64);
        assert!(ctx.is_typed("x"));

        ctx.push_scope();
        ctx.define("y", RustType::String);
        assert_eq!(ctx.get_type("y"), RustType::String);
        assert_eq!(ctx.get_type("x"), RustType::F64); // Can still see outer scope

        ctx.pop_scope();
        assert_eq!(ctx.get_type("y"), RustType::Value); // y no longer visible
    }

    #[test]
    fn push_fun_param_scope_shadows_outer() {
        use tishlang_ast::{FunParam, Span, TypedParam};

        let mut ctx = TypeContext::new();
        ctx.define("n", RustType::F64);
        let params = vec![FunParam::Simple(TypedParam {
            name: "n".into(),
            name_span: Span {
                start: (0, 0),
                end: (0, 0),
            },
            type_ann: None,
            default: None,
        })];
        ctx.push_fun_param_scope(&params, None);
        assert_eq!(ctx.get_type("n"), RustType::Value);
        ctx.pop_scope();
        assert_eq!(ctx.get_type("n"), RustType::F64);
    }
}
