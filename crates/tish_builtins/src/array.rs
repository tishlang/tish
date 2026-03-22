//! Array builtin methods.

use std::cell::RefCell;
use std::rc::Rc;
use tishlang_core::Value;
use crate::helpers::normalize_index;

/// Create a new array Value from a Vec of Values.
pub fn from_vec(v: Vec<Value>) -> Value {
    Value::Array(Rc::new(RefCell::new(v)))
}

/// Get the length of an array.
pub fn len(arr: &Value) -> Option<usize> {
    match arr {
        Value::Array(a) => Some(a.borrow().len()),
        _ => None,
    }
}

pub fn push(arr: &Value, args: &[Value]) -> Value {
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
    if let Value::Array(arr) = arr {
        arr.borrow_mut().pop().unwrap_or(Value::Null)
    } else {
        Value::Null
    }
}

pub fn shift(arr: &Value) -> Value {
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
    if let Value::Array(arr) = arr {
        let separator = match sep {
            Value::String(s) => s.to_string(),
            _ => ",".to_string(),
        };
        let arr_borrow = arr.borrow();
        let parts: Vec<String> = arr_borrow.iter().map(|v| v.to_display_string()).collect();
        Value::String(parts.join(&separator).into())
    } else {
        Value::Null
    }
}

pub fn reverse(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        arr.borrow_mut().reverse();
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

/// Fisher-Yates shuffle. Returns a new shuffled array (does not mutate).
pub fn shuffle(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut v = arr.borrow().clone();
        use rand::seq::SliceRandom;
        v.shuffle(&mut rand::rng());
        Value::Array(Rc::new(RefCell::new(v)))
    } else {
        Value::Null
    }
}

pub fn splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
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
        Value::Array(Rc::new(RefCell::new(removed)))
    } else {
        Value::Null
    }
}

pub fn slice(arr: &Value, start: &Value, end: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len() as i64;
        let start_idx = normalize_index(start, len, 0);
        let end_idx = normalize_index(end, len, len as usize);
        let sliced = if start_idx < end_idx { arr_borrow[start_idx..end_idx].to_vec() } else { vec![] };
        Value::Array(Rc::new(RefCell::new(sliced)))
    } else {
        Value::Null
    }
}

pub fn concat(arr: &Value, args: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut result = arr.borrow().clone();
        for v in args {
            if let Value::Array(other) = v {
                result.extend(other.borrow().iter().cloned());
            } else {
                result.push(v.clone());
            }
        }
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

pub fn flat(arr: &Value, depth: &Value) -> Value {
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
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

// Higher-order array methods require a callback function.
// These take NativeFn from tishlang_core::Value::Function

pub fn map(arr: &Value, callback: &Value) -> Value {
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let result: Vec<Value> = arr_borrow.iter().enumerate().map(|(i, v)| {
            cb(&[v.clone(), Value::Number(i as f64)])
        }).collect();
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

pub fn filter(arr: &Value, callback: &Value) -> Value {
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let result: Vec<Value> = arr_borrow.iter().enumerate().filter_map(|(i, v)| {
            let keep = cb(&[v.clone(), Value::Number(i as f64)]);
            if keep.is_truthy() { Some(v.clone()) } else { None }
        }).collect();
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

pub fn reduce(arr: &Value, callback: &Value, initial: &Value) -> Value {
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        let len = arr_borrow.len();
        let (start_idx, mut acc) = if matches!(initial, Value::Null)
            && !arr_borrow.is_empty()
        {
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
    if let (Value::Array(arr), Value::Function(cb)) = (arr, callback) {
        let arr_borrow = arr.borrow();
        for (i, v) in arr_borrow.iter().enumerate() {
            cb(&[v.clone(), Value::Number(i as f64)]);
        }
    }
    Value::Null
}

pub fn find(arr: &Value, callback: &Value) -> Value {
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
        Value::Array(Rc::new(RefCell::new(result)))
    } else {
        Value::Null
    }
}

fn sort_by_impl<F>(arr: &Value, cmp: F) -> Value
where F: FnMut(&Value, &Value) -> std::cmp::Ordering {
    if let Value::Array(arr) = arr {
        arr.borrow_mut().sort_by(cmp);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn sort_default(arr: &Value) -> Value {
    sort_by_impl(arr, |a, b| a.to_display_string().cmp(&b.to_display_string()))
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
        
        *arr_mut = indices.into_iter().map(|i| std::mem::replace(&mut elements[i], Value::Null)).collect();
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
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
    if asc { cmp } else { cmp.reverse() }
}

pub fn sort_numeric_asc(arr: &Value) -> Value {
    sort_by_impl(arr, |a, b| num_cmp(a, b, true))
}

pub fn sort_numeric_desc(arr: &Value) -> Value {
    sort_by_impl(arr, |a, b| num_cmp(a, b, false))
}

/// Sort array of objects by numeric property: arr.sort((a,b)=>a.prop-b.prop)
pub fn sort_by_property_numeric(arr: &Value, prop: &str, asc: bool) -> Value {
    let prop_arc = std::sync::Arc::from(prop);
    sort_by_impl(arr, move |a, b| {
        let na = get_prop_number(a, &prop_arc);
        let nb = get_prop_number(b, &prop_arc);
        let cmp = na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal);
        if asc { cmp } else { cmp.reverse() }
    })
}

fn get_prop_number(v: &Value, prop: &std::sync::Arc<str>) -> f64 {
    match v {
        Value::Object(o) => o.borrow().get(prop.as_ref()).map(|v| v.as_number().unwrap_or(f64::NAN)).unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}
