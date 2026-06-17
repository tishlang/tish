// #180/#247: JSON.stringify number formatting (integer fast-path + ECMAScript ToString for the
// rest — `1e+21`/`1e-7`/`-0`→`0`) and JSON.parse key order (insertion order, not hash order) must
// match the JS target across interp/vm/native/cranelift/node. Valid in both tish and node.
console.log("nums", JSON.stringify([0, -0, 1, 42, -7, 1.5, 4.5, 0.1, 100000, 1e21, 1e-7, 12345.678]))
console.log("keyorder", Object.keys(JSON.parse('{"k1":1,"zoo":2,"alpha":3,"m":4,"b":5}')).join(","))
console.log("roundtrip", JSON.stringify(JSON.parse('{"a":[1,2.5,-0,1e21],"b":"he\\"llo\\n","c":{"d":true,"e":null}}')))
console.log("negzero-parse", 1 / JSON.parse("-0"))
console.log("bigint", JSON.stringify(JSON.parse("9007199254740993")))
console.log("escapes", JSON.stringify("tab\tnl\nq\"bs\\"))
