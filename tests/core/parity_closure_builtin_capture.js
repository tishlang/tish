// #513: a nested value-closure that captures a LOCAL and also references a clone-hoisted
// BUILTIN must compile natively. The arrow emitter's implicit-capture skip-list (BUILTINS)
// contained names its clone-hoist list lacked (`Boolean`, `serve`) — those were referenced raw
// inside the `move` closure, MOVING them out of the enclosing `Fn` closure (E0507) whenever a
// captured local forced real capture context. Boolean is the default-features member of that
// set, so this fixture locks the class corpus-wide; the `serve` shape is covered by the serve
// smoke suite. Identical output across interp/vm/native + node.

function main() {
  let x = 1
  let flag = function() { return Boolean(x) }
  console.log(flag())

  // two levels + both a local and a builtin at each level
  let outer = function() {
    let y = 0
    let inner = function() { return Boolean(y) }
    return Boolean(x) && !inner()
  }
  console.log(outer())
}
main()
