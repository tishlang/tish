//! Array builtin methods.
//!
//! Shared array method implementations used by both tish_runtime (compiled code)
//! and can be adapted for tish_eval (interpreter).

use std::cell::RefCell;
use std::rc::Rc;
use tish_core::Value;

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

pub fn includes(arr: &Value, search: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let arr_borrow = arr.borrow();
        for v in arr_borrow.iter() {
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

pub fn splice(arr: &Value, start: &Value, delete_count: Option<&Value>, items: &[Value]) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        let len = arr_mut.len() as i64;
        
        let start_idx = match start {
            Value::Number(n) => {
                let n = *n as i64;
                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
            }
            _ => 0,
        };
        
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
        let start_idx = match start {
            Value::Number(n) => {
                let n = *n as i64;
                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
            }
            _ => 0,
        };
        let end_idx = match end {
            Value::Null => len as usize,
            Value::Number(n) => {
                let n = *n as i64;
                if n < 0 { (len + n).max(0) as usize } else { n.min(len) as usize }
            }
            _ => len as usize,
        };
        let sliced: Vec<Value> = if start_idx < end_idx {
            arr_borrow[start_idx..end_idx].to_vec()
        } else {
            vec![]
        };
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
// These take NativeFn from tish_core::Value::Function

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
        let mut acc = initial.clone();
        for (i, v) in arr_borrow.iter().enumerate() {
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

pub fn sort_default(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        arr_mut.sort_by(|a, b| {
            let sa = a.to_display_string();
            let sb = b.to_display_string();
            sa.cmp(&sb)
        });
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn sort_with_comparator(arr: &Value, comparator: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        
        if let Value::Function(cmp_fn) = comparator {
            let len = arr_mut.len();
            let mut indices: Vec<usize> = (0..len).collect();
            let mut elements: Vec<Value> = std::mem::take(&mut *arr_mut);
            let mut args_buf: [Value; 2] = [Value::Null, Value::Null];
            
            indices.sort_by(|&a, &b| {
                args_buf[0] = elements[a].clone();
                args_buf[1] = elements[b].clone();
                let result = cmp_fn(&args_buf);
                match result {
                    Value::Number(n) => {
                        if n < 0.0 {
                            std::cmp::Ordering::Less
                        } else if n > 0.0 {
                            std::cmp::Ordering::Greater
                        } else {
                            std::cmp::Ordering::Equal
                        }
                    }
                    _ => std::cmp::Ordering::Equal,
                }
            });
            
            let sorted: Vec<Value> = indices.into_iter().map(|i| {
                std::mem::replace(&mut elements[i], Value::Null)
            }).collect();
            *arr_mut = sorted;
        }
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn sort_numeric_asc(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        arr_mut.sort_by(|a, b| {
            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
            na.partial_cmp(&nb).unwrap_or(std::cmp::Ordering::Equal)
        });
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}

pub fn sort_numeric_desc(arr: &Value) -> Value {
    if let Value::Array(arr) = arr {
        let mut arr_mut = arr.borrow_mut();
        arr_mut.sort_by(|a, b| {
            let na = match a { Value::Number(n) => *n, _ => f64::NAN };
            let nb = match b { Value::Number(n) => *n, _ => f64::NAN };
            nb.partial_cmp(&na).unwrap_or(std::cmp::Ordering::Equal)
        });
        drop(arr_mut);
        Value::Array(Rc::clone(arr))
    } else {
        Value::Null
    }
}
