//! Classify a `pub fn` from syn for glue emission (driven by signature shape, not crate name).

use syn::{
    FnArg, GenericArgument, ItemFn, PathArguments, ReturnType, Type, TypeReference,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureClass {
    /// `fn foo(args: &[Value]) -> Value` (or `tishlang_runtime::Value`).
    TishValueAbi,
    /// First parameter `&T` (or `&mut T`), `T: Serialize` (or `?Sized + Serialize`), returns `Result<String, _>`-like.
    SerializeRefToResultString,
    /// First parameter `&str` (or `& 'a str`), returns `Result<_, _>`, and has `Deserialize` bound on a type param.
    DeserializeStrToResult,
}

pub fn classify_public_fn(item: &ItemFn) -> Option<SignatureClass> {
    if matches!(classify_tish_abi(item), Some(SignatureClass::TishValueAbi)) {
        return Some(SignatureClass::TishValueAbi);
    }
    if is_deserialize_str_result(item) {
        return Some(SignatureClass::DeserializeStrToResult);
    }
    if is_serialize_ref_to_result_string(item) {
        return Some(SignatureClass::SerializeRefToResultString);
    }
    None
}

fn classify_tish_abi(item: &ItemFn) -> Option<SignatureClass> {
    let sig = &item.sig;
    let mut value_args = 0;
    for arg in &sig.inputs {
        let FnArg::Typed(t) = arg else {
            continue;
        };
        if is_slice_value(&t.ty) {
            value_args += 1;
        }
    }
    if value_args != 1 || sig.inputs.len() != 1 {
        return None;
    }
    let Some(ret_ty) = return_type_inner(&sig.output) else {
        return None;
    };
    if !is_value_type(ret_ty) {
        return None;
    }
    Some(SignatureClass::TishValueAbi)
}

fn return_type_inner(ret: &ReturnType) -> Option<&Type> {
    match ret {
        ReturnType::Default => None,
        ReturnType::Type(_, ty) => Some(ty),
    }
}

fn is_slice_value(ty: &Type) -> bool {
    let Some(inner) = strip_reference(ty) else {
        return false;
    };
    let Type::Slice(s) = inner else {
        return false;
    };
    is_value_type(&s.elem)
}

fn strip_reference(ty: &Type) -> Option<&Type> {
    match ty {
        Type::Reference(TypeReference { elem, .. }) => Some(elem.as_ref()),
        _ => None,
    }
}

fn is_value_type(ty: &Type) -> bool {
    let Type::Path(p) = ty else {
        return false;
    };
    let seg = p.path.segments.last();
    let Some(seg) = seg else {
        return false;
    };
    if seg.ident != "Value" {
        return false;
    }
    // Accept `Value`, `tishlang_runtime::Value`, `tishlang_core::Value`
    if p.path.segments.len() == 1 {
        return true;
    }
    let prev = &p.path.segments[p.path.segments.len() - 2];
    prev.ident == "tishlang_runtime" || prev.ident == "tishlang_core"
}

fn is_str_ref(ty: &Type) -> bool {
    match ty {
        Type::Reference(TypeReference { elem, .. }) => matches!(
            elem.as_ref(),
            Type::Path(p) if p.path.is_ident("str")
        ),
        _ => false,
    }
}

fn is_deserialize_str_result(item: &ItemFn) -> bool {
    let sig = &item.sig;
    if sig.inputs.len() != 1 {
        return false;
    }
    let FnArg::Typed(arg) = sig.inputs.first().unwrap() else {
        return false;
    };
    if !is_str_ref(&arg.ty) {
        return false;
    }
    let Some(ret) = return_type_inner(&sig.output) else {
        return false;
    };
    if result_ok_type(ret).is_none() {
        return false;
    }
    has_deserialize_bound(item)
}

fn has_deserialize_bound(item: &ItemFn) -> bool {
    for p in &item.sig.generics.params {
        if let syn::GenericParam::Type(t) = p {
            for b in &t.bounds {
                if bound_name_is(b, "Deserialize") {
                    return true;
                }
            }
        }
    }
    if let Some(wc) = &item.sig.generics.where_clause {
        for pred in &wc.predicates {
            if let syn::WherePredicate::Type(t) = pred {
                for b in &t.bounds {
                    if bound_name_is(b, "Deserialize") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn bound_name_is(b: &syn::TypeParamBound, want: &str) -> bool {
    let syn::TypeParamBound::Trait(t) = b else {
        return false;
    };
    let path = &t.path;
    path.segments.last().is_some_and(|s| s.ident == want)
}

fn is_serialize_ref_to_result_string(item: &ItemFn) -> bool {
    let sig = &item.sig;
    if sig.inputs.len() != 1 {
        return false;
    }
    let FnArg::Typed(arg) = sig.inputs.first().unwrap() else {
        return false;
    };
    if strip_reference(&arg.ty).is_none() {
        return false;
    }
    let Some(ret) = return_type_inner(&sig.output) else {
        return false;
    };
    let Some(ok_ty) = result_ok_type(ret) else {
        return false;
    };
    if !type_is_string_or_str(ok_ty) {
        return false;
    }
    has_serialize_bound(item)
}

fn has_serialize_bound(item: &ItemFn) -> bool {
    for p in &item.sig.generics.params {
        if let syn::GenericParam::Type(t) = p {
            for b in &t.bounds {
                if bound_name_is(b, "Serialize") {
                    return true;
                }
            }
        }
    }
    if let Some(wc) = &item.sig.generics.where_clause {
        for pred in &wc.predicates {
            if let syn::WherePredicate::Type(t) = pred {
                for b in &t.bounds {
                    if bound_name_is(b, "Serialize") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn result_ok_type(ty: &Type) -> Option<&Type> {
    let Type::Path(p) = ty else {
        return None;
    };
    let seg = p.path.segments.last()?;
    if seg.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(ab) = &seg.arguments else {
        return None;
    };
    let first = ab.args.first()?;
    let GenericArgument::Type(t) = first else {
        return None;
    };
    Some(t)
}

fn type_is_string_or_str(ty: &Type) -> bool {
    match ty {
        Type::Path(p) => {
            if p.path.is_ident("String") {
                return true;
            }
            p.path.segments.len() == 1 && p.path.segments[0].ident == "str"
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syn::parse_quote;

    #[test]
    fn classify_serde_to_string_shape() {
        let item: ItemFn = parse_quote! {
            pub fn to_string<T: ?Sized + Serialize>(value: &T) -> Result<String, ()> {
                unimplemented!()
            }
        };
        assert_eq!(
            classify_public_fn(&item),
            Some(SignatureClass::SerializeRefToResultString)
        );
    }

    #[test]
    fn classify_from_str_shape() {
        let item: ItemFn = parse_quote! {
            pub fn from_str<'a, T: Deserialize<'a>>(s: &'a str) -> Result<T, ()> {
                unimplemented!()
            }
        };
        assert_eq!(
            classify_public_fn(&item),
            Some(SignatureClass::DeserializeStrToResult)
        );
    }
}
