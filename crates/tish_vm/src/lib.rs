//! Bytecode VM for Tish execution.

mod vm;

pub use vm::{run, Vm};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytecode_for_of_break_continue_switch_try() {
        let src = r#"
let sum = 0;
for (let x of [1, 2, 3]) { sum = sum + x; }
console.log(sum);
let n = 0;
do { n = n + 1; } while (n < 3);
console.log(n);
const k = 2;
switch (k) {
  case 1: console.log(1); break;
  case 2: console.log(2); break;
  default: console.log(0);
}
        try { throw "ok"; } catch (e) { console.log(e); }
        let x = 10;
        x++;
        ++x;
        x += 3;
        console.log(x);
        fn f(a, b, ...rest) { return rest.length; }
        console.log(f(1,2,3,4));
"#;
        let program = tish_parser::parse(src).expect("parse");
        let chunk = tish_bytecode::compile(&program).expect("compile");
        let _ = run(&chunk).expect("run");
    }
}
