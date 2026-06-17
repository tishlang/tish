// Cross-backend parity: JSON.parse must (a) preserve object key insertion order (not hash order)
// and (b) scan nested values string-aware. The interpreter used to have its own parser that
// depth-counted `{}`/`[]` brackets WITHOUT skipping string contents, so a nested value whose string
// held a bracket — e.g. `{"a":{"s":"}"}}` — mis-sliced and the whole parse failed to null, while
// Node/VM succeed. It now delegates to the shared tish_core parser. Object.keys order and
// JSON.stringify of the parsed value must match the JS target on interp/vm/native/cranelift.
// Valid in both tish and node.
console.log("flat", Object.keys(JSON.parse('{"k1":1,"k2":2,"k3":3,"k4":4,"k5":5}')).join(","))
console.log("desc", Object.keys(JSON.parse('{"zzz":1,"yyy":2,"xxx":3,"www":4}')).join(","))
console.log("many", Object.keys(JSON.parse('{"a":1,"b":2,"c":3,"d":4,"e":5,"f":6,"g":7,"h":8,"i":9,"j":10}')).join(","))
console.log("record", JSON.stringify(JSON.parse('{"id":1,"name":"x","value":2,"active":true,"tags":[1,2,3]}')))
console.log("brace-in-str", JSON.stringify(JSON.parse('{"a":{"s":"}{][ "},"b":2,"c":3}')))
console.log("bracket-arr", JSON.stringify(JSON.parse('{"x":["]","[",","],"y":3}')))
console.log("nested-order", Object.keys(JSON.parse('{"z":1,"a":{"y":2,"b":3},"m":4}')).join(","))
console.log("unicode-esc", JSON.parse('"\\u0041\\u00e9"'))
