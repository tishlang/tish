//! A JIT-off VM must not touch the process-global JIT state. Its own test binary (one test, one
//! process) because other tests JIT in parallel and would move the cache counts.

use tishlang_bytecode::compile;
use tishlang_vm::{jit_cache_lens, Vm};

#[test]
fn jit_off_vm_never_compiles() {
    let src = "fn sq(x) { return x * x }\n\
               fn sum(n) { let s = 0\nfor (let i = 0; i < n; i = i + 1) { s = s + sq(i) }\nreturn s }\n\
               fn make() { fn calc(x) { return sq(x) + 1 }\nreturn calc }\n\
               let total = sum(20000) + make()(3)";
    let chunk = compile(&tishlang_parser::parse(src).expect("parse")).expect("compile");

    let before = jit_cache_lens();
    let mut off = Vm::new();
    off.set_jit_enabled(false);
    off.run(&chunk).expect("jit off runs");
    assert_eq!(jit_cache_lens(), before, "a JIT-off VM compiled something");

    let mut on = Vm::new();
    on.run(&chunk).expect("jit on runs");
    assert_ne!(jit_cache_lens(), before, "control: the same program JIT-compiles with the JIT on");
}
