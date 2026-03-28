#![allow(unused, non_snake_case)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use tishlang_runtime::ObjectMap;
use tishlang_runtime::{console_debug as tish_console_debug, console_info as tish_console_info, console_log as tish_console_log, console_warn as tish_console_warn, console_error as tish_console_error, boolean as tish_boolean, decode_uri as tish_decode_uri, encode_uri as tish_encode_uri, in_operator as tish_in_operator, is_finite as tish_is_finite, is_nan as tish_is_nan, json_parse as tish_json_parse, json_stringify as tish_json_stringify, math_abs as tish_math_abs, math_ceil as tish_math_ceil, math_floor as tish_math_floor, math_max as tish_math_max, math_min as tish_math_min, math_round as tish_math_round, math_sqrt as tish_math_sqrt, parse_float as tish_parse_float, parse_int as tish_parse_int, math_random as tish_math_random, math_pow as tish_math_pow, math_sin as tish_math_sin, math_cos as tish_math_cos, math_tan as tish_math_tan, math_log as tish_math_log, math_exp as tish_math_exp, math_sign as tish_math_sign, math_trunc as tish_math_trunc, date_now as tish_date_now, array_is_array as tish_array_is_array, string_from_char_code as tish_string_from_char_code, object_assign as tish_object_assign, object_keys as tish_object_keys, object_values as tish_object_values, object_entries as tish_object_entries, object_from_entries as tish_object_from_entries, TishError, Value};
use tishlang_runtime::{read_file as tish_read_file, write_file as tish_write_file, file_exists as tish_file_exists, read_dir as tish_read_dir, mkdir as tish_mkdir};
use tishlang_runtime::regexp_new;

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut console = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("debug"), Value::Function(Rc::new(|args: &[Value]| { tish_console_debug(args); Value::Null }))),
        (Arc::from("info"), Value::Function(Rc::new(|args: &[Value]| { tish_console_info(args); Value::Null }))),
        (Arc::from("log"), Value::Function(Rc::new(|args: &[Value]| { tish_console_log(args); Value::Null }))),
        (Arc::from("warn"), Value::Function(Rc::new(|args: &[Value]| { tish_console_warn(args); Value::Null }))),
        (Arc::from("error"), Value::Function(Rc::new(|args: &[Value]| { tish_console_error(args); Value::Null }))),
    ]))));
    let Boolean = Value::Function(Rc::new(|args: &[Value]| tish_boolean(args)));
    let parseInt = Value::Function(Rc::new(|args: &[Value]| tish_parse_int(args)));
    let parseFloat = Value::Function(Rc::new(|args: &[Value]| tish_parse_float(args)));
    let decodeURI = Value::Function(Rc::new(|args: &[Value]| tish_decode_uri(args)));
    let encodeURI = Value::Function(Rc::new(|args: &[Value]| tish_encode_uri(args)));
    let isFinite = Value::Function(Rc::new(|args: &[Value]| tish_is_finite(args)));
    let isNaN = Value::Function(Rc::new(|args: &[Value]| tish_is_nan(args)));
    let Infinity = Value::Number(f64::INFINITY);
    let NaN = Value::Number(f64::NAN);
    let Math = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("abs"), Value::Function(Rc::new(|args: &[Value]| tish_math_abs(args)))),
        (Arc::from("sqrt"), Value::Function(Rc::new(|args: &[Value]| tish_math_sqrt(args)))),
        (Arc::from("min"), Value::Function(Rc::new(|args: &[Value]| tish_math_min(args)))),
        (Arc::from("max"), Value::Function(Rc::new(|args: &[Value]| tish_math_max(args)))),
        (Arc::from("floor"), Value::Function(Rc::new(|args: &[Value]| tish_math_floor(args)))),
        (Arc::from("ceil"), Value::Function(Rc::new(|args: &[Value]| tish_math_ceil(args)))),
        (Arc::from("round"), Value::Function(Rc::new(|args: &[Value]| tish_math_round(args)))),
        (Arc::from("random"), Value::Function(Rc::new(|args: &[Value]| tish_math_random(args)))),
        (Arc::from("pow"), Value::Function(Rc::new(|args: &[Value]| tish_math_pow(args)))),
        (Arc::from("sin"), Value::Function(Rc::new(|args: &[Value]| tish_math_sin(args)))),
        (Arc::from("cos"), Value::Function(Rc::new(|args: &[Value]| tish_math_cos(args)))),
        (Arc::from("tan"), Value::Function(Rc::new(|args: &[Value]| tish_math_tan(args)))),
        (Arc::from("log"), Value::Function(Rc::new(|args: &[Value]| tish_math_log(args)))),
        (Arc::from("exp"), Value::Function(Rc::new(|args: &[Value]| tish_math_exp(args)))),
        (Arc::from("sign"), Value::Function(Rc::new(|args: &[Value]| tish_math_sign(args)))),
        (Arc::from("trunc"), Value::Function(Rc::new(|args: &[Value]| tish_math_trunc(args)))),
        (Arc::from("PI"), Value::Number(std::f64::consts::PI)),
        (Arc::from("E"), Value::Number(std::f64::consts::E)),
    ]))));
    let JSON = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("parse"), Value::Function(Rc::new(|args: &[Value]| tish_json_parse(args)))),
        (Arc::from("stringify"), Value::Function(Rc::new(|args: &[Value]| tish_json_stringify(args)))),
    ]))));
    let Array = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("isArray"), Value::Function(Rc::new(|args: &[Value]| tish_array_is_array(args)))),
    ]))));
    let String = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("fromCharCode"), Value::Function(Rc::new(|args: &[Value]| tish_string_from_char_code(args)))),
    ]))));
    let Date = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("now"), Value::Function(Rc::new(|args: &[Value]| tish_date_now(args)))),
    ]))));
    let Object = Value::Object(Rc::new(RefCell::new(ObjectMap::from([
        (Arc::from("assign"), Value::Function(Rc::new(|args: &[Value]| tish_object_assign(args)))),
        (Arc::from("keys"), Value::Function(Rc::new(|args: &[Value]| tish_object_keys(args)))),
        (Arc::from("values"), Value::Function(Rc::new(|args: &[Value]| tish_object_values(args)))),
        (Arc::from("entries"), Value::Function(Rc::new(|args: &[Value]| tish_object_entries(args)))),
        (Arc::from("fromEntries"), Value::Function(Rc::new(|args: &[Value]| tish_object_from_entries(args)))),
    ]))));
    let readFile = Value::Function(Rc::new(|args: &[Value]| tish_read_file(args)));
    let writeFile = Value::Function(Rc::new(|args: &[Value]| tish_write_file(args)));
    let fileExists = Value::Function(Rc::new(|args: &[Value]| tish_file_exists(args)));
    let readDir = Value::Function(Rc::new(|args: &[Value]| tish_read_dir(args)));
    let mkdir = Value::Function(Rc::new(|args: &[Value]| tish_mkdir(args)));
    let RegExp = Value::Function(Rc::new(|args: &[Value]| regexp_new(args)));
    let mut files = ({
        let f = &readDir;
        match f { Value::Function(cb) => cb(&[Value::String("content".into()).clone()]), _ => panic!("Not a function") }
    });
    ({
        let f = &tishlang_runtime::get_prop(&console, "log");
        match f { Value::Function(cb) => cb(&[Value::String("files:".into()).clone(), files.clone()]), _ => panic!("Not a function") }
    });
    {
        let mut i = Value::Number(0_f64);
'for_loop_0: loop {
            if !Value::Bool({ let Value::Number(a) = &(i) else { panic!("cmp: expected number left") }; let Value::Number(b) = &(tishlang_runtime::get_prop(&files, "length")) else { panic!("cmp: expected number right") }; *a < *b }).is_truthy() { break; }
            {
                let mut f = (tishlang_runtime::get_index(&files, &i)).clone();
                ({
                    let f = &tishlang_runtime::get_prop(&console, "log");
                    match f { Value::Function(cb) => cb(&[Value::String("f:".into()).clone(), f.clone()]), _ => panic!("Not a function") }
                });
                let mut raw = ({
                    let f = &readFile;
                    match f { Value::Function(cb) => cb(&[{ match (&Value::String("content/".into()), &f) {
                    (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
                    (Value::String(a), Value::String(b)) => Value::String(format!("{}{}", a, b).into()),
                    (a, b) => Value::String(format!("{}{}", (a as &Value).to_display_string(), (b as &Value).to_display_string()).into()),
                } }.clone()]), _ => panic!("Not a function") }
                });
                ({
                    let f = &tishlang_runtime::get_prop(&console, "log");
                    match f { Value::Function(cb) => cb(&[Value::String("raw is string:".into()).clone(), Value::String(match &raw { Value::Number(_) => "number".into(), Value::String(_) => "string".into(), Value::Bool(_) => "boolean".into(), Value::Null => "object".into(), Value::Array(_) => "object".into(), Value::Object(_) => "object".into(), Value::Function(_) => "function".into(), _ => "object".into() }).clone()]), _ => panic!("Not a function") }
                });
                let mut parts = tishlang_runtime::string_split(&raw, &Value::String("---".into()));
                ({
                    let f = &tishlang_runtime::get_prop(&console, "log");
                    match f { Value::Function(cb) => cb(&[Value::String("parts.length:".into()).clone(), tishlang_runtime::get_prop(&parts, "length").clone()]), _ => panic!("Not a function") }
                });
                break 'for_loop_0;
            }
            { let _v = { match (&i, &Value::Number(1_f64)) {
                    (Value::Number(a), Value::Number(b)) => Value::Number(a + b),
                    (Value::String(a), Value::String(b)) => Value::String(format!("{}{}", a, b).into()),
                    (a, b) => Value::String(format!("{}{}", (a as &Value).to_display_string(), (b as &Value).to_display_string()).into()),
                } }; i = _v.clone(); _v };
        }
    }
    Ok(())
}
