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

/// Packed-HOF fast-path gate: when `arr` is a packed [`Value::NumberArray`] and `callback` is
/// callable, snapshot the `Vec<f64>` so a higher-order method can fold/scan it WITHOUT first
/// materialising a boxed `Vec<Value>` (the `as_boxed_array` deopt that otherwise allocates a
/// 24-byte-per-element copy on every call). Snapshotting — rather than holding the `VmRef` borrow
/// across the callback — matches the boxed path's copy semantics (mutations to the array from inside
/// the callback aren't observed mid-scan) and can't deadlock if the callback re-enters the same
/// array. Returns `None` to fall through to the generic boxed path (regular arrays, or a non-callable
/// second argument).
#[inline]
fn packed_snapshot<'c>(
    arr: &Value,
    callback: &'c Value,
) -> Option<(Vec<f64>, &'c tishlang_core::NativeFn)> {
    match (arr, callback) {
        (Value::NumberArray(na), Value::Function(cb)) => Some((na.borrow().clone(), cb)),
        _ => None,
    }
}

/// A `Vec<f64>` HOF result → packed [`Value::NumberArray`] so a packed input keeps producing packed
/// output (memory stays 3× denser and downstream packed fast paths keep firing). Empty results stay
/// a boxed `Value::Array`, matching the convention that empty arrays are general-purpose containers
/// whose element type can't be inferred. Only reached from a `NumberArray` input, which already
/// implies packed arrays are enabled, so no extra flag check is needed.
#[inline]
fn packed_or_empty(nums: Vec<f64>) -> Value {
    if nums.is_empty() {
        Value::Array(VmRef::new(Vec::new()))
    } else {
        Value::number_array(nums)
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
            // SameValueZero: like `===` but NaN matches NaN (JS `Array.prototype.includes`, unlike
            // `indexOf` which stays strict). #247
            if v.strict_eq(search)
                || matches!((v, search), (Value::Number(a), Value::Number(b)) if a.is_nan() && b.is_nan())
            {
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

/// `Array.prototype.fill(value, start?, end?)` — overwrites elements in `[start, end)` with
/// `value`, mutating in place and returning the same array. start/end use JS index
/// normalization (negatives count from the end; defaults 0 and length). Issue #76.
pub fn fill(arr: &Value, value: &Value, start: &Value, end: &Value) -> Value {
    let arr = as_boxed_array(arr);
    let arr = &*arr;
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        let len = arr_mut.len() as i64;
        let start_idx = normalize_index(start, len, 0);
        let end_idx = normalize_index(end, len, len as usize);
        let mut i = start_idx;
        while i < end_idx && i < arr_mut.len() {
            arr_mut[i] = value.clone();
            i += 1;
        }
        Value::Array(arr.clone())
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
    // Packed fast path: scan the `Vec<f64>` snapshot directly. Speculatively build a packed
    // `Vec<f64>` so a numeric map (the common `x => x * k` case) keeps its result packed with NO
    // boxed `Vec<Value>` intermediate — downstream packed fast paths then keep firing. Deopt to a
    // boxed array on the FIRST non-numeric callback result; every element's callback still runs
    // exactly once, in index order (the deopt resumes at `i + 1`).
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        let mut nums: Vec<f64> = Vec::with_capacity(data.len());
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { return packed_or_empty(nums); } // #303
            match cb.call(&[Value::Number(n), Value::Number(i as f64)]) {
                Value::Number(r) => nums.push(r),
                other => {
                    let mut boxed: Vec<Value> = nums.into_iter().map(Value::Number).collect();
                    boxed.push(other);
                    for (j, &m) in data.iter().enumerate().skip(i + 1) {
                        if tishlang_core::has_pending_throw() { break; } // #303
                        boxed.push(cb.call(&[Value::Number(m), Value::Number(j as f64)]));
                    }
                    return Value::Array(VmRef::new(boxed));
                }
            }
        }
        return packed_or_empty(nums);
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        // #303: stop on a pending throw from the callback (don't map the rest of the array).
        let mut result: Vec<Value> = Vec::with_capacity(arr_borrow.len());
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; }
            result.push(cb.call(&[v.clone(), Value::Number(i as f64)]));
        }
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

pub fn filter(arr: &Value, callback: &Value) -> Value {
    // Packed fast path: `filter` keeps a SUBSET of the input f64s, so the result is always numeric —
    // build the packed `Vec<f64>` directly, no boxed intermediate, and hand back a `NumberArray`.
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        let mut out: Vec<f64> = Vec::new();
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb.call(&[Value::Number(n), Value::Number(i as f64)]).is_truthy() {
                out.push(n);
            }
        }
        return packed_or_empty(out);
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        // #303: stop on a pending throw from the predicate (don't test the rest of the array).
        let mut result: Vec<Value> = Vec::new();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; }
            if cb.call(&[v.clone(), Value::Number(i as f64)]).is_truthy() {
                result.push(v.clone());
            }
        }
        Value::Array(VmRef::new(result))
    } else {
        Value::Null
    }
}

pub fn reduce(arr: &Value, callback: &Value, initial: &Value) -> Value {
    // Packed fast path: fold the `Vec<f64>` snapshot directly. Same no-initial rule as the boxed
    // path (absent init → first element as the seed, scan from index 1).
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        let (start_idx, mut acc) = if matches!(initial, Value::Null) && !data.is_empty() {
            (1usize, Value::Number(data[0]))
        } else {
            (0usize, initial.clone())
        };
        // `skip(start_idx)` preserves the true element index for the callback's 3rd arg.
        for (i, &x) in data.iter().enumerate().skip(start_idx) {
            if tishlang_core::has_pending_throw() { return acc; } // #303
            acc = cb.call(&[acc, Value::Number(x), Value::Number(i as f64)]);
        }
        return acc;
    }
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
            if tishlang_core::has_pending_throw() { return acc; } // #303
            let v = arr_borrow[i].clone();
            acc = cb.call(&[acc, v.clone(), Value::Number(i as f64)]);
        }
        acc
    } else {
        Value::Null
    }
}

pub fn for_each(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { return Value::Null; } // #303
            cb.call(&[Value::Number(n), Value::Number(i as f64)]);
        }
        return Value::Null;
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            cb.call(&[v.clone(), Value::Number(i as f64)]);
        }
    }
    Value::Null
}

pub fn find(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb.call(&[Value::Number(n), Value::Number(i as f64)]).is_truthy() {
                return Value::Number(n);
            }
        }
        return Value::Null;
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            let result = cb.call(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return v.clone();
            }
        }
    }
    Value::Null
}

pub fn find_index(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb.call(&[Value::Number(n), Value::Number(i as f64)]).is_truthy() {
                return Value::Number(i as f64);
            }
        }
        return Value::Number(-1.0);
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            let result = cb.call(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return Value::Number(i as f64);
            }
        }
    }
    Value::Number(-1.0)
}

/// `Array.prototype.findLast` — like [`find`] but iterates from the end; the callback still receives
/// the original index. Returns `null` (JS `undefined`) when nothing matches. #247
pub fn find_last(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for i in (0..data.len()).rev() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb
                .call(&[Value::Number(data[i]), Value::Number(i as f64)])
                .is_truthy()
            {
                return Value::Number(data[i]);
            }
        }
        return Value::Null;
    }
    let arr = as_boxed_array(arr);
    let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for i in (0..arr_borrow.len()).rev() {
            if tishlang_core::has_pending_throw() { break; } // #303
            let v = &arr_borrow[i];
            if cb.call(&[v.clone(), Value::Number(i as f64)]).is_truthy() {
                return v.clone();
            }
        }
    }
    Value::Null
}

/// `Array.prototype.findLastIndex` — like [`find_index`] but from the end; `-1` if no match. #247
pub fn find_last_index(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for i in (0..data.len()).rev() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb
                .call(&[Value::Number(data[i]), Value::Number(i as f64)])
                .is_truthy()
            {
                return Value::Number(i as f64);
            }
        }
        return Value::Number(-1.0);
    }
    let arr = as_boxed_array(arr);
    let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for i in (0..arr_borrow.len()).rev() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb
                .call(&[arr_borrow[i].clone(), Value::Number(i as f64)])
                .is_truthy()
            {
                return Value::Number(i as f64);
            }
        }
    }
    Value::Number(-1.0)
}

/// `Array.prototype.at(index)` — negative `index` counts from the end; out of range → `null`
/// (JS `undefined`). A non-numeric index is `ToInteger`'d to 0, like JS. #247
pub fn at(arr: &Value, index: &Value) -> Value {
    let i = match index {
        Value::Number(n) => *n as i64,
        _ => 0,
    };
    let arr = as_boxed_array(arr);
    let arr = &*arr;
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len() as i64;
        let idx = if i < 0 { len + i } else { i };
        if idx >= 0 && idx < len {
            return arr_borrow[idx as usize].clone();
        }
    }
    Value::Null
}

pub fn some(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if cb.call(&[Value::Number(n), Value::Number(i as f64)]).is_truthy() {
                return Value::Bool(true);
            }
        }
        return Value::Bool(false);
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            let result = cb.call(&[v.clone(), Value::Number(i as f64)]);
            if result.is_truthy() {
                return Value::Bool(true);
            }
        }
    }
    Value::Bool(false)
}

pub fn every(arr: &Value, callback: &Value) -> Value {
    if let Some((data, cb)) = packed_snapshot(arr, callback) {
        for (i, &n) in data.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            if !cb.call(&[Value::Number(n), Value::Number(i as f64)]).is_truthy() {
                return Value::Bool(false);
            }
        }
        return Value::Bool(true);
    }
    let arr = as_boxed_array(arr); let arr = &*arr;
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            if tishlang_core::has_pending_throw() { break; } // #303
            let result = cb.call(&[v.clone(), Value::Number(i as f64)]);
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
            if tishlang_core::has_pending_throw() { break; } // #303
            let mapped = cb.call(&[v.clone(), Value::Number(i as f64)]);
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
            // #303: once the comparator has thrown, stop invoking it — return Equal so the sort can
            // unwind. Avoids extra comparator calls (and their side effects) after the throw.
            if tishlang_core::has_pending_throw() {
                return std::cmp::Ordering::Equal;
            }
            args_buf[0] = elements[a].clone();
            args_buf[1] = elements[b].clone();
            match cmp_fn.call(&args_buf) {
                Value::Number(n) if n < 0.0 => std::cmp::Ordering::Less,
                Value::Number(n) if n > 0.0 => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            }
        });

        if tishlang_core::has_pending_throw() {
            // #303: the comparator threw — do NOT write the partial/bogus reordering back. Restore the
            // original element order (leave the array unmodified) and let the caller re-raise the throw.
            *arr_mut = elements;
        } else {
            *arr_mut = indices
                .into_iter()
                .map(|i| std::mem::replace(&mut elements[i], Value::Null))
                .collect();
        }
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

#[cfg(test)]
mod packed_hof_tests {
    //! The packed (`NumberArray`) HOF fast paths must be observably IDENTICAL to the boxed path —
    //! same element + index callback args, same result shape — since cross-backend parity depends
    //! on it. These run with packing semantics directly (the helpers don't read the env flag; a
    //! `NumberArray` value is enough to take the fast path).
    use super::*;
    use tishlang_core::Value;

    fn na(xs: &[f64]) -> Value {
        Value::NumberArray(VmRef::new(xs.to_vec()))
    }
    fn nums(v: &Value) -> Vec<f64> {
        match v {
            Value::Array(a) => a.borrow().iter().map(|e| e.as_number().unwrap_or(f64::NAN)).collect(),
            Value::NumberArray(a) => a.borrow().clone(),
            _ => vec![],
        }
    }
    fn cb_num(f: fn(f64, f64) -> f64) -> Value {
        Value::native(move |a: &[Value]| {
            Value::Number(f(a[0].as_number().unwrap_or(0.0), a[1].as_number().unwrap_or(0.0)))
        })
    }
    fn cb_pred(f: fn(f64, f64) -> bool) -> Value {
        Value::native(move |a: &[Value]| {
            Value::Bool(f(a[0].as_number().unwrap_or(0.0), a[1].as_number().unwrap_or(0.0)))
        })
    }

    #[test]
    fn reduce_packed() {
        let n = na(&[3.0, 1.0, 4.0, 1.0, 5.0]);
        let add = cb_num(|acc, x| acc + x);
        // With init.
        assert_eq!(reduce(&n, &add, &Value::Number(0.0)).as_number(), Some(14.0));
        // No init → first element seeds, scan from index 1 (same total here).
        assert_eq!(reduce(&n, &add, &Value::Null).as_number(), Some(14.0));
        // Index arg: callback (acc, _elem, index) — sum the indices 0..5 = 10.
        let sum_idx = Value::native(|a: &[Value]| {
            Value::Number(a[0].as_number().unwrap() + a[2].as_number().unwrap())
        });
        assert_eq!(reduce(&n, &sum_idx, &Value::Number(0.0)).as_number(), Some(10.0));
    }

    #[test]
    fn map_filter_stay_packed() {
        let n = na(&[3.0, 1.0, 4.0, 1.0, 5.0]);
        // Numeric map → packed NumberArray result (chains stay packed), with correct values.
        let m = map(&n, &cb_num(|x, _i| x * 2.0));
        assert!(matches!(m, Value::NumberArray(_)), "numeric map should stay packed");
        assert_eq!(nums(&m), vec![6.0, 2.0, 8.0, 2.0, 10.0]);
        // filter keeps a subset of the input f64s → always packed.
        let f = filter(&n, &cb_pred(|x, _i| x > 2.0));
        assert!(matches!(f, Value::NumberArray(_)), "filter should stay packed");
        assert_eq!(nums(&f), vec![3.0, 4.0, 5.0]);
    }

    #[test]
    fn map_deopts_to_boxed_on_non_numeric() {
        let n = na(&[1.0, 2.0, 3.0]);
        // Callback returns a string for the middle element → deopt to a boxed array, preserving order
        // (callback runs once per element).
        let cb = Value::native(|a: &[Value]| {
            let x = a[0].as_number().unwrap();
            if x == 2.0 { Value::String("two".into()) } else { Value::Number(x * 10.0) }
        });
        match &map(&n, &cb) {
            Value::Array(a) => {
                let b = a.borrow();
                assert_eq!(b.len(), 3);
                assert_eq!(b[0].as_number(), Some(10.0));
                assert!(matches!(&b[1], Value::String(s) if s.as_str() == "two"));
                assert_eq!(b[2].as_number(), Some(30.0));
            }
            _ => panic!("mixed-result map must be a boxed array"),
        }
    }

    #[test]
    fn map_filter_empty_stays_boxed() {
        let n = na(&[1.0, 2.0, 3.0]);
        // All rejected → empty boxed array (empty arrays stay general-purpose containers).
        assert!(matches!(filter(&n, &cb_pred(|_x, _i| false)), Value::Array(_)));
        // Empty input → empty boxed array.
        assert!(matches!(map(&na(&[]), &cb_num(|x, _i| x)), Value::Array(_)));
    }

    #[test]
    fn scan_packed() {
        let n = na(&[3.0, 1.0, 4.0, 1.0, 5.0]);
        assert!(matches!(some(&n, &cb_pred(|x, _i| x > 4.0)), Value::Bool(true)));
        assert!(matches!(some(&n, &cb_pred(|x, _i| x > 9.0)), Value::Bool(false)));
        assert!(matches!(every(&n, &cb_pred(|x, _i| x > 0.0)), Value::Bool(true)));
        assert!(matches!(every(&n, &cb_pred(|x, _i| x > 2.0)), Value::Bool(false)));
        // first element > 3 is 4.0 at index 2.
        assert_eq!(find(&n, &cb_pred(|x, _i| x > 3.0)).as_number(), Some(4.0));
        assert_eq!(find_index(&n, &cb_pred(|x, _i| x > 3.0)).as_number(), Some(2.0));
        assert_eq!(find_index(&n, &cb_pred(|x, _i| x > 99.0)).as_number(), Some(-1.0));
    }

    #[test]
    fn for_each_packed_passes_element_and_index() {
        use std::sync::{Arc, Mutex};
        let n = na(&[3.0, 1.0, 4.0, 1.0, 5.0]);
        let acc = Arc::new(Mutex::new(0.0f64));
        let a2 = acc.clone();
        let collect = Value::native(move |a: &[Value]| {
            *a2.lock().unwrap() += a[0].as_number().unwrap() + a[1].as_number().unwrap();
            Value::Null
        });
        assert!(matches!(for_each(&n, &collect), Value::Null));
        // sum(elems)=14 + sum(idx 0..5)=10.
        assert_eq!(*acc.lock().unwrap(), 24.0);
    }

    #[test]
    fn non_function_callback_falls_through() {
        // A NumberArray with a non-callable 2nd arg must not take the fast path; mirrors the boxed
        // path's `Value::Null` (map/filter) without panicking.
        let n = na(&[1.0, 2.0]);
        assert!(matches!(map(&n, &Value::Number(1.0)), Value::Null));
        assert!(matches!(filter(&n, &Value::Null), Value::Null));
    }

    #[test]
    fn for_each_stops_on_pending_throw() {
        use std::sync::{Arc, Mutex};
        // #303: once the callback parks a throw, for_each must stop invoking it (no extra iterations).
        let _ = tishlang_core::take_pending_throw(); // start with a clean slot
        let arr = from_vec(vec![
            Value::Number(1.0),
            Value::Number(2.0),
            Value::Number(3.0),
            Value::Number(4.0),
        ]);
        let calls = Arc::new(Mutex::new(0usize));
        let c2 = calls.clone();
        let cb = Value::native(move |a: &[Value]| {
            *c2.lock().unwrap() += 1;
            if a[0].as_number() == Some(2.0) {
                tishlang_core::set_pending_throw(Value::Null);
            }
            Value::Null
        });
        for_each(&arr, &cb);
        assert_eq!(
            *calls.lock().unwrap(),
            2,
            "for_each should stop after the throwing element, not run all 4"
        );
        let _ = tishlang_core::take_pending_throw(); // drain so it doesn't leak into other tests
    }

    #[test]
    fn sort_with_comparator_throw_restores_original_order() {
        // #303: a comparator that throws must NOT corrupt the array — sort_with_comparator restores the
        // original order and leaves the throw parked for the caller to re-raise.
        let _ = tishlang_core::take_pending_throw();
        let arr = from_vec(vec![
            Value::Number(5.0),
            Value::Number(4.0),
            Value::Number(3.0),
            Value::Number(2.0),
            Value::Number(1.0),
        ]);
        // Park a throw on the first comparison and return a dummy ordering value.
        let cmp = Value::native(|_a: &[Value]| {
            tishlang_core::set_pending_throw(Value::String("cmp".into()));
            Value::Null
        });
        let _ = sort_with_comparator(&arr, &cmp);
        if let Value::Array(a) = &arr {
            let got: Vec<f64> = a
                .borrow()
                .iter()
                .map(|v| v.as_number().unwrap_or(f64::NAN))
                .collect();
            assert_eq!(
                got,
                vec![5.0, 4.0, 3.0, 2.0, 1.0],
                "array must be left in its original order after a comparator throw"
            );
        } else {
            panic!("expected a boxed array");
        }
        assert!(
            tishlang_core::take_pending_throw().is_some(),
            "the parked throw must survive for the caller to re-raise"
        );
    }
}
