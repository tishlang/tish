//! Type system for Tish static typing.
//!
//! Maps TypeAnnotation from the AST to concrete Rust types for code generation.

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use tishlang_ast::{BinOp, FunParam, TypeAnnotation, TypedParam};

thread_local! {
    /// Whether the extended GBA numeric vocabulary — the narrow integer widths (`i8/u8/i16/u16/u32`)
    /// and `fixed` — lowers to native scalar types. **Off by default** (standard targets): those
    /// annotations must fall back to the boxed `Value` path off-GBA, because `fixed` has no host
    /// runtime type (a hard compile error) and the narrow widths truncate on store (`u8 = 300` → 255),
    /// diverging from the interpreter and breaking the typed == boxed == interpreter guarantee that
    /// non-GBA targets rely on. A GBA build turns this on (see `set_gba_numerics`), where those
    /// types ARE the point. `i32`/`f64` are NOT gated — `f64` is just `number`, and `i32` is the
    /// pre-existing JS-ToInt32 register lowering.
    static GBA_NUMERICS: Cell<bool> = const { Cell::new(false) };
}

/// Enable (GBA) or disable (every other target) the extended narrow-int/`fixed` numeric vocabulary
/// for the current compile. Set once at compile start from the emit mode; see [`GBA_NUMERICS`].
pub fn set_gba_numerics(on: bool) {
    GBA_NUMERICS.with(|c| c.set(on));
}

/// Whether `from_annotation` should lower `i8/u8/i16/u16/u32/fixed` to native scalars.
fn gba_numerics() -> bool {
    GBA_NUMERICS.with(|c| c.get())
}

/// Concrete Rust type representation for code generation.
#[derive(Debug, Clone, PartialEq)]
pub enum RustType {
    /// Dynamic Value type (untyped or complex types)
    Value,
    /// f64 (for number)
    F64,
    /// i32 — a `number` local PROVEN to always hold an integer reinterpretable as a JS ToInt32
    /// bit-pattern, kept in an integer register across a bitwise/hash hot loop (bun/JSC-style)
    /// instead of round-tripping `f64`↔`i32` on every op. The value is the signed int32
    /// (= JS `ToInt32`) view; `>>> 0` results are uint32 reinterpreted into this i32. Off-GBA only
    /// the codegen's i32-loop-var lowering produces this type (a proven-in-range value) — never
    /// `from_annotation`; a `--target gba` build additionally lowers a `: i32` annotation to it
    /// (part of the GBA typed-scalar vocabulary, gated by `GBA_NUMERICS`).
    I32,
    /// Fixed-point `tishlang_runtime::Fixed` (= agb `Num<i32, 8>`), for the `fixed`
    /// annotation — fast, FPU-free math for positions/velocities on the GBA. Only
    /// `from_annotation` (and fixed-typed inference) produces this. Boxes to/from
    /// `Value::Number` losslessly at boundaries.
    Fixed,
    /// Narrow integer STORAGE types (`i8`/`u8`/`i16`/`u16`/`u32` annotations). Their whole
    /// point is compact struct fields — a `u8` HP or an `i16` tile coordinate costs 1–2
    /// bytes in scarce EWRAM instead of 8. Arithmetic PROMOTES to `f64` (JS Number
    /// semantics, exactly like `I32` at [`Self::result_type_of_binop`]); a store back into
    /// the narrow field truncates with a saturating `as` cast. Only `from_annotation`
    /// produces these. Box to/from `Value::Number` (every value in-range is exact in f64).
    I8,
    U8,
    I16,
    U16,
    U32,
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
    /// Box<T> — heap indirection required to make recursive structs finite-sized.
    /// Only the recursive-struct native pass (#178) produces this, for child fields
    /// like `Option<Box<TishRec_Node>>`; never from `from_annotation`.
    Boxed(Box<RustType>),
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
    /// Tuple `(T0, T1, …)` for `[T0, T1]` tuple types — a native Rust tuple.
    Tuple(Vec<RustType>),
    /// #179 Stage B: a closed set of 2..=8 distinct primitive-record shapes read at one site — the
    /// "megamorphic" array-of-heterogeneous-objects pattern. Emitted as a generated Rust enum
    /// `TishUnion_<name> { V0(TishStruct_<v0>), … }` (one variant per shape); a `.field` read present
    /// in EVERY variant lowers to a `match` with a direct field load per arm (no hash, no IC). `name`
    /// is the union alias; each `variants` entry is `(per-variant struct alias, its fields)`.
    ShapeUnion {
        name: Arc<str>,
        variants: Vec<(Arc<str>, Vec<(Arc<str>, RustType)>)>,
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
            TypeAnnotation::Simple(name, _) => match name.as_ref() {
                "number" => RustType::F64,
                // Concrete Rust scalar names in typed tish (user requirement): they
                // lower to the corresponding native Rust type instead of a boxed
                // `Value`. `f64` is an alias for `number`; `i32` reuses the existing
                // integer-register lowering (its value is the JS ToInt32 view, which
                // is exactly an annotated `i32`). The narrow widths are compact struct
                // storage; `fixed` is agb `Num<i32,8>`.
                "f64" => RustType::F64,
                // GBA-only numeric vocabulary (see `GBA_NUMERICS`): off-GBA these fall through to
                // the `other` arm → boxed `Value`, so `fixed` compiles (no host `Fixed` type),
                // narrow widths keep interpreter number semantics instead of truncating, and a
                // `: i32` annotation doesn't saturate/ToInt32 an out-of-range value — restoring the
                // invariant that `I32` comes ONLY from the proven-in-range i32-loop-var lowering,
                // never a raw annotation. (`f64` is ungated — it is just `number`.)
                "i32" if gba_numerics() => RustType::I32,
                "i8" if gba_numerics() => RustType::I8,
                "u8" if gba_numerics() => RustType::U8,
                "i16" if gba_numerics() => RustType::I16,
                "u16" if gba_numerics() => RustType::U16,
                "u32" if gba_numerics() => RustType::U32,
                "fixed" if gba_numerics() => RustType::Fixed,
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
            TypeAnnotation::Array(elem) => {
                RustType::Vec(Box::new(Self::from_annotation_with_aliases(elem, aliases)))
            }
            TypeAnnotation::Object(fields) => {
                // Security #379: a field key that is not a valid Rust identifier must not drive a
                // native struct (it would be interpolated into generated Rust). Keep the whole object
                // on the boxed `Value` path — always correct, just unspecialized.
                if fields.iter().any(|(k, _)| !is_struct_field_safe(k)) {
                    return RustType::Value;
                }
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
                        .any(|t| matches!(t, TypeAnnotation::Simple(s, _) if s.as_ref() == "null"));
                    if has_null {
                        let non_null = types.iter().find(
                            |t| !matches!(t, TypeAnnotation::Simple(s, _) if s.as_ref() == "null"),
                        );
                        if let Some(inner) = non_null {
                            return RustType::Option(Box::new(Self::from_annotation_with_aliases(
                                inner, aliases,
                            )));
                        }
                    }
                }
                // Other unions fall back to Value
                RustType::Value
            }
            // `[T0, T1]` -> a native Rust tuple `(T0, T1)`.
            TypeAnnotation::Tuple(elems) => RustType::Tuple(
                elems
                    .iter()
                    .map(|e| Self::from_annotation_with_aliases(e, aliases))
                    .collect(),
            ),
            // A literal type lowers to its base primitive.
            TypeAnnotation::Literal(lit) => match lit {
                tishlang_ast::TypeLiteral::Str(_) => RustType::String,
                tishlang_ast::TypeLiteral::Num(_) => RustType::F64,
                tishlang_ast::TypeLiteral::Bool(_) => RustType::Bool,
            },
            // Intersection of object shapes (e.g. `interface X extends Y { … }` → `Y & { … }`):
            // merge the fields into one shape. Registered as a `type` alias, this becomes a native
            // struct. Any non-object member → can't merge → fall back to boxed `Value`.
            TypeAnnotation::Intersection(parts) => {
                let mut fields: Vec<(Arc<str>, RustType)> = Vec::new();
                for p in parts {
                    match Self::from_annotation_with_aliases(p, aliases) {
                        RustType::Object(fs) | RustType::Named { fields: fs, .. } => {
                            for (k, v) in fs {
                                if !fields.iter().any(|(ek, _)| *ek == k) {
                                    fields.push((k, v));
                                }
                            }
                        }
                        _ => return RustType::Value,
                    }
                }
                RustType::Object(fields)
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

    /// A native integer scalar (`i32` or one of the narrow storage widths). These share one
    /// arithmetic model: read as `f64` (JS Number semantics), store with a truncating `as`
    /// cast. `Fixed` is deliberately excluded — it is fixed-point, not an integer.
    pub fn is_integer_scalar(&self) -> bool {
        matches!(
            self,
            RustType::I32
                | RustType::I8
                | RustType::U8
                | RustType::I16
                | RustType::U16
                | RustType::U32
        )
    }

    /// A NARROW integer storage width (`i8`/`u8`/`i16`/`u16`/`u32`) — an integer scalar other
    /// than the `I32` register type, which has its own JS-ToInt32 lowering. These promote to
    /// `f64` for arithmetic and truncate-cast on store.
    pub fn is_narrow_int(&self) -> bool {
        matches!(
            self,
            RustType::I8 | RustType::U8 | RustType::I16 | RustType::U16 | RustType::U32
        )
    }

    /// Infer the result type of a binary operation given the operand types.
    /// Returns `None` if native code cannot be emitted (fall back to Value path).
    pub fn result_type_of_binop(op: BinOp, lhs: &RustType, rhs: &RustType) -> Option<RustType> {
        if lhs == &RustType::F64 && rhs == &RustType::F64 {
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Mod | BinOp::Pow => {
                    Some(RustType::F64)
                }
                // Bitwise / shift ops: JS coerces both sides to int32, computes, and
                // returns a Number — so the native result is still F64. Big win for
                // crypto/hashing loops that would otherwise box every `^`/`>>>`.
                BinOp::BitAnd
                | BinOp::BitOr
                | BinOp::BitXor
                | BinOp::Shl
                | BinOp::Shr
                | BinOp::UShr => Some(RustType::F64),
                BinOp::Lt
                | BinOp::Le
                | BinOp::Gt
                | BinOp::Ge
                | BinOp::StrictEq
                | BinOp::StrictNe => Some(RustType::Bool),
                _ => None,
            }
        } else if lhs == &RustType::Fixed && rhs == &RustType::Fixed {
            // agb `Num<i32,8>` overloads `+ - * /` (Mul/Div apply the Q24.8 shift
            // correction) and `PartialOrd`/`PartialEq`, so these lower to native
            // fixed-point ops — no f64 round-trip, no FPU. `%`/`**`/bitwise fall to
            // the boxed path (rare on positions/velocities).
            match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => Some(RustType::Fixed),
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
        } else if lhs == &RustType::String && rhs == &RustType::String {
            // M2: native string concat + value equality. `+` concatenates; `===`/`!==` compare by
            // value (byte-identical to JS and to the boxed `Value::String` path). Relational
            // `< <= > >=` deliberately stay on the boxed path: JS orders strings by UTF-16 code
            // units while Rust `String` orders by UTF-8 bytes — they diverge outside the BMP.
            match op {
                BinOp::Add => Some(RustType::String),
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
            RustType::I32 => "i32".to_string(),
            RustType::I8 => "i8".to_string(),
            RustType::U8 => "u8".to_string(),
            RustType::I16 => "i16".to_string(),
            RustType::U16 => "u16".to_string(),
            RustType::U32 => "u32".to_string(),
            RustType::Fixed => "tishlang_runtime::Fixed".to_string(),
            RustType::String => "String".to_string(),
            RustType::Bool => "bool".to_string(),
            RustType::Unit => "()".to_string(),
            RustType::Vec(inner) => format!("Vec<{}>", inner.to_rust_type_str()),
            RustType::Option(inner) => format!("Option<{}>", inner.to_rust_type_str()),
            RustType::Boxed(inner) => format!("Box<{}>", inner.to_rust_type_str()),
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
            RustType::Tuple(elems) => tuple_text(&elems.iter().map(|e| e.to_rust_type_str()).collect::<Vec<_>>()),
            RustType::ShapeUnion { name, .. } => shape_union_enum_ident(name),
        }
    }

    /// Get the default value for this type.
    pub fn default_value(&self) -> String {
        match self {
            RustType::Value => "Value::Null".to_string(),
            RustType::F64 => "0.0".to_string(),
            RustType::I32 => "0i32".to_string(),
            RustType::I8 => "0i8".to_string(),
            RustType::U8 => "0u8".to_string(),
            RustType::I16 => "0i16".to_string(),
            RustType::U16 => "0u16".to_string(),
            RustType::U32 => "0u32".to_string(),
            RustType::Fixed => "tishlang_runtime::Fixed::from_raw(0)".to_string(),
            RustType::String => "String::new()".to_string(),
            RustType::Bool => "false".to_string(),
            RustType::Unit => "()".to_string(),
            RustType::Vec(_) => "Vec::new()".to_string(),
            RustType::Option(_) => "None".to_string(),
            RustType::Boxed(inner) => format!("Box::new({})", inner.default_value()),
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
            RustType::Tuple(elems) => {
                tuple_text(&elems.iter().map(|e| e.default_value()).collect::<Vec<_>>())
            }
            RustType::ShapeUnion { name, variants } => {
                // Default = the first variant with each of its fields defaulted.
                let (v0_alias, v0_fields) = &variants[0];
                let init = v0_fields
                    .iter()
                    .map(|(k, t)| format!("{}: {}", field_ident(k), t.default_value()))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}::{}({} {{ {} }})",
                    shape_union_enum_ident(name),
                    shape_union_variant_ident(0),
                    named_struct_ident(v0_alias),
                    init
                )
            }
        }
    }

    /// Generate code to convert from Value to this native type.
    pub fn from_value_expr(&self, value_expr: &str) -> String {
        match self {
            RustType::Tuple(elems) => {
                // `Value::Array([..])` -> `(e0, e1, …)`, converting each slot from its `Value`.
                let parts: Vec<String> = elems
                    .iter()
                    .enumerate()
                    .map(|(i, e)| {
                        e.from_value_expr(&format!(
                            "_t.get({}).cloned().unwrap_or(Value::Null)",
                            i
                        ))
                    })
                    .collect();
                format!(
                    "match &{} {{ Value::Array(_a) => {{ let _t = _a.borrow(); {} }}, _ => panic!(\"expected tuple\") }}",
                    value_expr,
                    tuple_text(&parts)
                )
            }
            RustType::Value => value_expr.to_string(),
            RustType::F64 => format!(
                "match &{} {{ Value::Number(n) => *n, _ => panic!(\"expected number\") }}",
                value_expr
            ),
            // A `Value::Number` narrowed to its JS ToInt32 bit-pattern (NaN/±Inf → 0, exactly as
            // the bitwise/shift path coerces). Only reached for an `I32`-typed binding boundary.
            RustType::I32 => format!(
                "match &{} {{ Value::Number(n) => tishlang_runtime::to_int32(*n), _ => panic!(\"expected number\") }}",
                value_expr
            ),
            // Narrow int storage: extract the JS Number and truncate-cast to the field width
            // (Rust `f64 as iN` saturates out-of-range and maps NaN→0 — safe for a storage slot).
            RustType::I8 | RustType::U8 | RustType::I16 | RustType::U16 | RustType::U32 => {
                format!(
                    "match &{} {{ Value::Number(n) => (*n as {}), _ => panic!(\"expected number\") }}",
                    value_expr,
                    self.to_rust_type_str()
                )
            }
            // f64 → Q24.8: scale by 256 and TRUNCATE toward zero (`as i32`), keeping only 8
            // fractional bits (`0.1` → raw 25 ≈ 0.0977, not exact). Must stay bit-identical with
            // the compile-time literal fold in codegen (`fixed_literal_of`) so a `fixed` value is
            // the same whether it came from a folded literal or this runtime conversion.
            RustType::Fixed => format!(
                "match &{} {{ Value::Number(n) => tishlang_runtime::Fixed::from_raw((*n * 256.0) as i32), _ => panic!(\"expected number\") }}",
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
                // Each field is fetched out of the Value::Object via `get_prop` and converted to its
                // native type. The source Value is bound ONCE to `_src` so a non-trivial `value_expr`
                // (an object literal, a call, …) is evaluated a single time instead of being textually
                // re-inlined per field (which would re-allocate the whole object N times). Missing
                // fields fall back to `default_value()` (rare — usually these come from JSON or PG).
                let field_assigns = fields
                    .iter()
                    .map(|(k, ty)| {
                        let fetch = format!("tishlang_runtime::get_prop(_src, {:?})", k.as_ref());
                        format!("{}: {}", field_ident(k), ty.from_value_expr(&fetch))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                // Bind `_src` as a BORROW (not a move) of the source Value: field fetches only need
                // `&Value` (get_prop borrows), and moving would use-after-move a caller-owned temp —
                // e.g. an assignment-as-expression `{ let _v = obj; lhs = <coerce _v>; _v }` reuses
                // `_v` for its value (tishlang/tish#486). `&(expr)` evaluates the source once and
                // (for a temporary) lifetime-extends it to this block, so it's still single-eval.
                format!(
                    "{{ let _src = &({}); {} {{ {} }} }}",
                    value_expr,
                    named_struct_ident(name),
                    field_assigns
                )
            }
            RustType::ShapeUnion { .. } => {
                // #179 Stage B: converting a boxed Value INTO a ShapeUnion is a boundary the safety
                // walk forbids (a typed union element never crosses to/from boxed), so codegen must
                // never emit this. Present for match-completeness; fails loud if gating is violated.
                format!(
                    "{{ let _ = {}; unreachable!(\"ShapeUnion from Value is a forbidden boundary\") }}",
                    value_expr
                )
            }
            _ => value_expr.to_string(), // Fallback
        }
    }

    /// Generate code to convert from this native type to Value.
    pub fn to_value_expr(&self, native_expr: &str) -> String {
        match self {
            RustType::Tuple(elems) => {
                // `(e0, e1, …)` -> `Value::Array([e0.into_value(), …])`.
                let parts: Vec<String> = elems
                    .iter()
                    .enumerate()
                    .map(|(i, e)| e.to_value_expr(&format!("{}.{}", native_expr, i)))
                    .collect();
                format!("Value::Array(VmRef::new(vec![{}]))", parts.join(", "))
            }
            RustType::Value => native_expr.to_string(),
            RustType::F64 => format!("Value::Number({})", native_expr),
            // The signed int32 view boxes as a JS Number (every i32 is exactly representable in
            // f64). The uint32 `>>> 0` reinterpretation is applied at the boxing site, not here.
            RustType::I32 => format!("Value::Number(({}) as f64)", native_expr),
            // Narrow int → JS Number: every in-range narrow int is exact in f64.
            RustType::I8 | RustType::U8 | RustType::I16 | RustType::U16 | RustType::U32 => {
                format!("Value::Number(({}) as f64)", native_expr)
            }
            // Q24.8 → f64 is exact (32 significant bits into a 52-bit mantissa).
            RustType::Fixed => format!("Value::Number(({}).to_raw() as f64 / 256.0)", native_expr),
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
                // Box by REFERENCE so the source Option is NOT moved — it may be a local captured by
                // an FnMut closure (a `serve()` handler, called per request) or used again after this
                // conversion. Mirrors the Vec arm's `.iter()` (which likewise avoids moving). `(*v)`
                // derefs the `&inner` the ref-match binds, so the inner's own clone/copy still applies.
                let inner_to_value = inner.to_value_expr("(*v)");
                format!(
                    "match &({}) {{ Some(v) => {}, None => Value::Null }}",
                    native_expr, inner_to_value
                )
            }
            RustType::Named { fields, .. } => {
                // Build the boxed Value with an ORDERED PropMap via `object_from_pairs` (no
                // intermediate `AHashMap`), so key order == field-declaration order == JS
                // insertion order. The old `ObjectMap::default()` (`AHashMap`) + insert path
                // scrambled key order NON-DETERMINISTICALLY per run (ahash seed) — a shipped
                // `JSON.stringify` / `Object.keys` / `for..in` divergence from node whenever a
                // native struct crosses back into untyped Tish. This boundary is paid only on
                // that crossing (JSON.stringify, calling a Value::Function, etc.); direct
                // Rust-to-Rust paths between two Named values stay as plain struct moves.
                let pairs = fields
                    .iter()
                    .map(|(k, ty)| {
                        let access = format!("{}.{}", native_expr, field_ident(k));
                        // A `Value`-typed field (e.g. from a generic struct `Box<T>`) accessed
                        // behind `&self` must be cloned — it isn't `Copy` and `to_value_expr(Value)`
                        // is identity. Native field types clone/copy inside their own `to_value_expr`.
                        let v_expr = if matches!(ty, RustType::Value) {
                            format!("{}.clone()", access)
                        } else {
                            ty.to_value_expr(&access)
                        };
                        format!("(::std::sync::Arc::from({:?}), {})", k.as_ref(), v_expr)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                // `object_from_pairs::<N>` builds the `PropMap` in one ordered pass (N = field
                // count, a codegen-time constant) — no AHashMap, so key order is preserved.
                format!("Value::object_from_pairs([{}])", pairs)
            }
            RustType::ShapeUnion { name, variants } => {
                // union → boxed Value::Object: one match arm per variant, delegating to that
                // variant's Named glue (ordered `object_from_pairs`, preserving JS key order).
                let arms = variants
                    .iter()
                    .enumerate()
                    .map(|(i, (alias, fields))| {
                        let named = RustType::Named {
                            name: alias.clone(),
                            fields: fields.clone(),
                        };
                        format!(
                            "{}::{}(__u) => {}",
                            shape_union_enum_ident(name),
                            shape_union_variant_ident(i),
                            named.to_value_expr("__u")
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("(match &{} {{ {} }})", native_expr, arms)
            }
            _ => native_expr.to_string(), // Fallback
        }
    }
}

/// Map a Tish type-alias name to the Rust struct identifier we emit.
/// Prefixed so user names can never collide with runtime types like `Value`.
/// Render a Rust tuple type/value from its parts, using the `(T,)` form for 1-tuples.
fn tuple_text(parts: &[String]) -> String {
    if parts.len() == 1 {
        format!("({},)", parts[0])
    } else {
        format!("({})", parts.join(", "))
    }
}

pub fn named_struct_ident(tish_name: &str) -> String {
    format!("TishStruct_{}", tish_name)
}

/// #179 Stage B: map a shape-union alias to the generated Rust enum identifier.
pub fn shape_union_enum_ident(tish_name: &str) -> String {
    format!("TishUnion_{}", tish_name)
}

/// #179 Stage B: the Rust variant identifier for the `idx`-th shape of a union (`V0`, `V1`, …).
pub fn shape_union_variant_ident(idx: usize) -> String {
    format!("V{}", idx)
}

/// Keywords that cannot be written as a raw identifier (`r#self` etc. are a hard Rust error), so an
/// object key equal to one of these must NOT drive struct/shape inference — it stays boxed.
const NON_RAWABLE_KEYWORDS: &[&str] = &["self", "Self", "super", "crate"];

/// True iff `key` can be safely emitted as a generated Rust struct field name via [`field_ident`]:
/// a plain ASCII identifier (`^[A-Za-z_][A-Za-z0-9_]*$`), excluding the bare wildcard `_` and the
/// four non-rawable keywords. Any other key — one containing spaces/punctuation, empty, non-ASCII,
/// etc. — is REJECTED so the object it belongs to falls back to the boxed `Value` path instead of
/// being lowered to a native struct.
///
/// This is the front-line guard for the native-codegen key-injection class (security #379): an
/// object literal key is arbitrary tish/JS text, and interpolating it verbatim into generated Rust
/// (`pub {key}: {ty},`) is arbitrary-code injection. Inference/annotation lowering call this to keep
/// unsafe-keyed objects on the always-correct boxed path; [`field_ident`] additionally sanitizes as a
/// last-resort choke point so no path can ever emit a non-identifier verbatim.
pub fn is_struct_field_safe(key: &str) -> bool {
    if key == "_" || NON_RAWABLE_KEYWORDS.contains(&key) {
        return false;
    }
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c == '_' || c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

/// FNV-1a 64-bit — a tiny, dependency-free hash used only to synthesize a deterministic safe field
/// identifier for an (unexpected) non-identifier key. Deterministic so a struct's field definition,
/// its constructor, its accessors, and its serializer all agree on the same name.
fn fnv1a_64(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Map a Tish field name (`randomNumber`) to a valid Rust identifier
/// (kept identical here — non-snake-case is allowed via
/// `#[allow(non_snake_case)]` on the struct, so JS-style camelCase keys
/// stay readable in the generated source).
///
/// A key that is not a valid Rust identifier is NEVER emitted verbatim (that would be code injection,
/// security #379): inference/annotation lowering should already have kept such an object boxed via
/// [`is_struct_field_safe`], so this is a defense-in-depth choke point — an unsafe key is replaced by
/// a deterministic `__tish_field_<hash>` name. The worst case if a producer is missed is thus a wrong
/// field name (a compile error / correctness bug), never injected Rust.
pub fn field_ident(tish_name: &str) -> String {
    if !is_struct_field_safe(tish_name) {
        return format!("__tish_field_{:016x}", fnv1a_64(tish_name));
    }
    // Reserve Rust keywords that would otherwise conflict. (The non-rawable keywords are already
    // excluded by `is_struct_field_safe` above, so every keyword reaching here has a valid `r#` form.)
    match tish_name {
        "type" | "ref" | "fn" | "match" | "move" | "mod" | "use"
        | "where" | "loop" | "yield" | "async" | "await" | "dyn" | "impl" | "trait" | "in"
        | "as" | "box" | "const" | "extern" | "let" | "mut" | "pub" | "static"
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
            RustType::from_annotation(&TypeAnnotation::Simple("number".into(), tishlang_ast::Span::default())),
            RustType::F64
        );
        assert_eq!(
            RustType::from_annotation(&TypeAnnotation::Simple("string".into(), tishlang_ast::Span::default())),
            RustType::String
        );
        assert_eq!(
            RustType::from_annotation(&TypeAnnotation::Simple("boolean".into(), tishlang_ast::Span::default())),
            RustType::Bool
        );
    }

    #[test]
    fn test_array_type() {
        let arr_type = TypeAnnotation::Array(Box::new(TypeAnnotation::Simple("number".into(), tishlang_ast::Span::default())));
        assert_eq!(
            RustType::from_annotation(&arr_type),
            RustType::Vec(Box::new(RustType::F64))
        );
    }

    // ---- security #379: object-literal key injection into generated Rust ----

    #[test]
    fn is_struct_field_safe_accepts_plain_and_camel_and_keywords() {
        for k in ["x", "y", "randomNumber", "_priv", "a1", "TishAnon_0", "type", "fn", "match"] {
            assert!(is_struct_field_safe(k), "{k:?} should be a safe field");
        }
    }

    #[test]
    fn is_struct_field_safe_rejects_injection_and_nonrawable() {
        for k in [
            // the #379 injection shapes: anything with punctuation/space/braces
            "x: i32 } pub fn pwned() {} struct Z { pub y",
            "a, b",
            "0abc",
            "has space",
            "unicodé",
            "",
            // non-rawable keywords would become invalid `r#self` etc.
            "self", "Self", "super", "crate",
            // bare wildcard is not a legal field name
            "_",
        ] {
            assert!(!is_struct_field_safe(k), "{k:?} must NOT be a safe field");
        }
    }

    #[test]
    fn field_ident_never_emits_non_identifier_for_unsafe_key() {
        // The core RCE guarantee: whatever key comes in, field_ident() out is ALWAYS a legal Rust
        // identifier (ASCII ident chars, or a leading `r#`) — never verbatim injectable text.
        let malicious = "x: i32 } pub fn PWNED() -> i32 { 1337 } struct S { pub y";
        let out = field_ident(malicious);
        assert!(out.starts_with("__tish_field_"), "unsafe key must be sanitized, got {out:?}");
        let body = out.trim_start_matches("r#");
        assert!(
            body.chars().enumerate().all(|(i, c)| {
                if i == 0 { c == '_' || c.is_ascii_alphabetic() } else { c == '_' || c.is_ascii_alphanumeric() }
            }),
            "field_ident output {out:?} is not a legal Rust identifier"
        );
        // deterministic: the def, the accessor, and the serializer must agree.
        assert_eq!(field_ident(malicious), field_ident(malicious));
    }

    #[test]
    fn field_ident_preserves_legit_and_raws_keywords() {
        assert_eq!(field_ident("randomNumber"), "randomNumber");
        assert_eq!(field_ident("x"), "x");
        assert_eq!(field_ident("type"), "r#type");
        // a non-rawable keyword is sanitized, NOT emitted as the invalid `r#self`.
        assert!(field_ident("self").starts_with("__tish_field_"));
    }

    #[test]
    fn annotation_object_with_unsafe_key_stays_boxed() {
        // A `type` alias whose object shape has a non-identifier key must fall back to boxed Value,
        // never a native struct with an injected field name.
        let obj = TypeAnnotation::Object(vec![
            ("safe".into(), TypeAnnotation::Simple("number".into(), tishlang_ast::Span::default())),
            (
                "evil } fn pwned() {".into(),
                TypeAnnotation::Simple("number".into(), tishlang_ast::Span::default()),
            ),
        ]);
        assert_eq!(RustType::from_annotation(&obj), RustType::Value);
    }

    #[test]
    fn test_nullable_type() {
        let nullable = TypeAnnotation::Union(vec![
            TypeAnnotation::Simple("string".into(), tishlang_ast::Span::default()),
            TypeAnnotation::Simple("null".into(), tishlang_ast::Span::default()),
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
