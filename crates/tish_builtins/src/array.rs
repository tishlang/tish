//! Array builtin methods.

use crate::helpers::normalize_index;
use tishlang_core::Value;
use tishlang_core::VmRef;

/// Create a new array Value from a Vec of Values.
pub fn from_vec(v: Vec<Value>) -> Value {
    Value::Array(VmRef::new(v))
}

/// Get the length of an array.
pub fn len(arr: &Value) -> Option<usize> {
    match arr {
        Value::Array(a) => Some(a.borrow().len()),
        Value::NumberArray(a) => Some(a.borrow().len()),
        _ => None,
    }
}

/// Normalise `NumberArray → Array` so callers that don't have a packed fast path
/// can use this deopt helper rather than changing every `if let Value::Array` branch.
/// Returns the original value unchanged for anything that isn't a `NumberArray`.
#[inline]
fn as_boxed_array(arr: &Value) -> std::borrow::Cow<'_, Value> {
    match arr {
        Value::NumberArray(na) => std::borrow::Cow::Owned(Value::materialize_number_array(na)),
        other => std::borrow::Cow::Borrowed(other),
    }
}

pub fn push(arr: &Value, args: &[Value]) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        for v in args {
            arr_mut.push(v.clone());
        }
        Value::Number(arr_mut.len() as f64)
    } else {
        Value::Null
    }
}

pub fn pop(arr: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        arr.borrow_mut().pop().unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

pub fn shift(arr: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        if arr_mut.is_empty() {
            Value::Null
        } else {
            arr_mut.remove(0)
        }
    } else {
        Value::Null
    }
}

pub fn unshift(arr: &Value, args: &[Value]) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        for (i, v) in args.iter().enumerate() {
            arr_mut.insert(i, v.clone());
        }
        Value::Number(arr_mut.len() as f64)
    } else {
        Value::Null
    }
}

pub fn index_of(arr: &Value, search: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if v.strict_eq(search) {
                return Value::Number(i as f64);
            }
        }
    }
    Value::Number(-1.0)
}

pub fn includes(arr: &Value, search: &Value, from: Option<&Value>) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len() as i64;
        let start = match from {
            Some(Value::Number(n)) if *n >= 0.0 => (*n as i64).min(len).max(0) as usize,
            Some(Value::Number(n)) if *n < 0.0 => ((len + *n as i64).max(0)) as usize,
            _ => 0,
        };
        for v in arr_borrow.iter().skip(start) {
            if v.strict_eq(search) {
                return Value::Bool(true);
            }
        }
    }
    Value::Bool(false)
}

pub fn join(arr: &Value, sep: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let separator = match sep {
            Value::String(s) => s.to_string(),
            _ => ",".to_string(),
        };
        let arr_borrow = arr.borrow();
        // JS `Array.prototype.join`: null/undefined → "", everything else via JS ToString
        // (nested arrays recurse to a comma-join, objects → "[object Object]").
        let parts: Vec<String> = arr_borrow
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                other => other.to_js_string(),
            })
            .collect();
        Value::String(parts.join(&separator).into())
    } else {
        Value::Null
    }
}

pub fn reverse(arr: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        arr.borrow_mut().reverse();
        Value::Array(arr.clone())
    } else {
        Value::Null
    }
}

/// Fisher-Yates shuffle. Returns a new shuffled array (does not mutate).
pub fn shuffle(arr: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut v = arr.borrow().clone();
        use rand::seq::SliceRandom;
        v.shuffle(&mut rand::rng());
        Value::Array(VmRef::new(v))
    } else {
        Value::Null
    }
}

pub fn splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        let len = arr_mut.len() as i64;
        let start_idx = normalize_index(start, len, 0);
        let del_count = match delete_count {
            Some(Value::Number(n)) => (*n as i64).max(0) as usize,
            _ => (len as usize).saturating_sub(start_idx),
        };
        let actual_delete = del_count.min(arr_mut.len().saturating_sub(start_idx));
        let removed: Vec<Value> = arr_mut
            .splice(start_idx..start_idx + actual_delete, items.iter().cloned())
            .collect();
        Value::Array(VmRef::new(removed))
    } else {
        Value::Null
    }
}

pub fn slice(arr: &Value, start: &Value, end: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len() as i64;
        let start_idx = normalize_index(start, len, 0);
        let end_idx = normalize_index(end, len, len as usize);
        let sliced = if start_idx < end_idx {
            arr_borrow[start_idx..end_idx].to_vec()
        } else {
            vec![]
        };
        Value::Array(VmRef::new(sliced))
    } else {
        Value::Null
    }
}

pub fn concat(arr: &Value, args: &[Value]) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut result = arr.borrow().clone();
        for v in args {
            if let Value::Array(other) = v {
                result.extend(other.borrow().iter().cloned());
            } else {
                result.push(v.clone());
            }
        }
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

pub fn flat(arr: &Value, depth: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    fn flatten(arr: &[Value], depth: i32, result: &mut Vec<Value>) {
        for v in arr {
            if depth > 0 {
                if let Value::Array(inner) = v {
                    flatten(&inner.borrow(), depth - 1, result);
                    continue;
                }
            }
            result.push(v.clone());
        }
    }

    if let Value::Array(arr) = arr {
        let d = match depth {
            Value::Number(n) => *n as i32,
            _ => 1,
        };
        let mut result = Vec::new();
        flatten(&arr.borrow(), d, &mut result);
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

// Higher-order array methods require a callback function.
// These take NativeFn from tishlang_core::Value::Function

pub fn map(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let result: Vec<Value> = arr_borrow
            .iter()
            .enumerate()
            .map(|(i, v)| cb(&[v.clone(), Value::Number(i as f64)]))
            .collect();
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

pub fn filter(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let result: Vec<Value> = arr_borrow
            .iter()
            .enumerate()
            .filter_map(|(i, v)| {
                let keep = cb(&[v.clone(), Value::Number(i as f64)]);
                if keep.is_truthy() {
                    Some(v.clone())
                } else {
                    None
                }
            })
            .collect();
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

pub fn reduce(arr: &Value, callback: &Value, initial: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len();
        let (start_idx, mut acc) = if matches!(initial, Value::Null) && !arr_borrow.is_empty() {
            // No initial value: use first element as acc, start from index 1
            (1, arr_borrow[0].clone())
        } else {
            (0, initial.clone())
        };
        for i in start_idx..len {
            let v = arr_borrow[i].clone();
            acc = cb(&[acc, v.clone(), Value::Number(i as f64)]);
        }
        acc
    } else {
        Value::Null
    }
}

pub fn for_each(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            cb(&[v.clone(), Value::Number(i as f64)]);
        }
    }
    Value::Null
}

pub fn find(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            let result = cb(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return v.clone();
            }
        }
    }
    Value::Null
}

pub fn find_index(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            let result = cb(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return Value::Number(i as f64);
            }
        }
    }
    Value::Number(-1.0)
}

pub fn some(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            let result = cb(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return Value::Bool(true);
            }
        }
    }
    Value::Bool(false)
}

pub fn every(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            let result = cb(&[v.clone(), Value::Number(i as f64)]);
            if !result.is_truthy() {
                return Value::Bool(false);
            }
        }
        Value::Bool(true)
    } else {
        Value::Bool(false)
    }
}

pub fn flat_map(arr: &Value, callback: &Value) -> Value {
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let mut result: Vec<Value> = Vec::new();
        for (i, v) in arr_borrow.iter().enumerate() {
            let mapped = cb(&[v.clone(), Value::Number(i as f64)]);
            if let Value::Array(inner) = mapped {
                result.extend(inner.borrow().iter().cloned());
            } else {
                result.push(mapped);
            }
        }
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

fn sort_by_impl<F>(arr: &Value, cmp: F) -> Value
where
    F: FnMut(&Value, &Value) -> std::cmp::Ordering,
{
    if let Value::Array(arr) = arr {
        arr.borrow_mut().sort_by(cmp);
        Value::Array(arr.clone())
    } else {
        Value::Null
    }
}

pub fn sort_default(arr: &Value) -> Value {
    sort_by_impl(arr, |a, b| {
        a.to_display_string().cmp(&b.to_display_string())
    })
}

pub fn sort_with_comparator(arr: &Value, comparator: &Value) -> Value {
    if let (Value::Array(arr), Value::Function(cmp_fn)) = (arr, comparator) {
        let mut arr_mut = arr.borrow_mut();
        let len = arr_mut.len();
        let mut indices: Vec<usize> = (0..len).collect();
        let mut elements: Vec<Value> = std::mem::take(&mut *arr_mut);
        let mut args_buf: [Value; 2] = [Value::Null, Value::Null];

        indices.sort_by(|&a, &b| {
            args_buf[0] = elements[a].clone();
            args_buf[1] = elements[b].clone();
            match cmp_fn(&args_buf) {
                Value::Number(n) if n < 0.0 => std::cmp::Ordering::Less,
                Value::Number(n) if n > 0.0 => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        });

        *arr_mut = indices
            .into_iter()
            .map(|i| std::mem::replace(&mut elements[i], Value::Null))
            .collect();
        drop(arr_mut);
        Value::Array(arr.clone())
    } else {
        Value::Null
    }
}

fn num_cmp(a: &Value, b: &Value, asc: bool) -> std::cmp::Ordering {
    let (na, nb) = match (a, b) {
        (Value::Number(a), Value::Number(b)) => (*a, *b),
        _ => (f64::NAN, f64::NAN),
    };
    let cmp = na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
    if asc {
        cmp
    } else {
        cmp.reverse()
    }
}

pub fn sort_numeric_asc(arr: &Value) -> Value {
    sort_numeric_impl(arr, true)
}

pub fn sort_numeric_desc(arr: &Value) -> Value {
    sort_numeric_impl(arr, false)
}

/// Numeric sort. When every element is a number, extract to unboxed `f64`,
/// `sort_unstable` (faster than the stable comparator sort, and stability is
/// irrelevant for equal numbers), then write back — no per-comparison `Value`
/// match. Mixed arrays fall back to the comparator path.
fn sort_numeric_impl(arr: &Value, asc: bool) -> Value {
    // NumberArray fast path: sort the Vec<f64> directly — no unbox pass, no rebox.
    if let Value::NumberArray(a) = arr {
        let mut g = a.borrow_mut();
        if asc {
            g.sort_unstable_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
        } else {
            g.sort_unstable_by(|x, y| y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal));
        }
        return Value::NumberArray(a.clone());
    }
    if let Value::Array(a) = arr {
        {
            let mut g = a.borrow_mut();
            if g.iter().all(|v| matches!(v, Value::Number(_))) {
                let mut nums: Vec<f64> = g
                    .iter()
                    .map(|v| match v {
                        Value::Number(n) => *n,
                        _ => f64::NAN,
                    })
                    .collect();
                if asc {
                    nums.sort_unstable_by(|x, y| {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    });
                } else {
                    nums.sort_unstable_by(|x, y| {
                        y.partial_cmp(x).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                for (slot, n) in g.iter_mut().zip(nums) {
                    *slot = Value::Number(n);
                }
                return Value::Array(a.clone());
            }
            g.sort_by(|x, y| num_cmp(x, y, asc));
        }
        Value::Array(a.clone())
    } else {
        Value::Null
    }
}

/// Sort array of objects by numeric property: arr.sort((a,b)=>a.prop-b.prop)
pub fn sort_by_property_numeric(arr: &Value, prop: &str, asc: bool) -> Value {
    let prop_arc = std::sync::Arc::from(prop);
    sort_by_impl(arr, move |a, b| {
        let na = get_prop_number(a, &prop_arc);
        let nb = get_prop_number(b, &prop_arc);
        let cmp = na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
        if asc {
            cmp
        } else {
            cmp.reverse()
        }
    })
}

fn get_prop_number(v: &Value, prop: &std::sync::Arc<str>) -> f64 {
    match v {
        Value::Object(o) => o
            .borrow()
            .strings
            .get(prop.as_ref())
            .map(|v| v.as_number().unwrap_or(f64::NAN))
            .unwrap_or(f64::NAN),
        // `.length` is a *computed* property (not a stored map entry) for strings and arrays.
        // The fused `(a,b)=>a.length-b.length` sort path must compute it the same way
        // `get_member` does, otherwise it returns NaN, every comparison collapses to Equal,
        // and the array is left unsorted. Mirror get_member's length semantics here.
        Value::String(s) if prop.as_ref() == "length" => s.chars().count() as f64,
        Value::Array(a) if prop.as_ref() == "length" => a.borrow().len() as f64,
        _ => f64::NAN,
    }
}
