// #247: String.split — empty separator yields chars (no surrounding empties), plus limit and the
// ordinary string-separator cases. interp/vm/native/cranelift/node must agree. Valid in tish + node.
console.log(JSON.stringify("xyz".split("")))
console.log(JSON.stringify("".split("")))
console.log(JSON.stringify("abcd".split("", 2)))
console.log(JSON.stringify("a,b,c".split(",")))
console.log(JSON.stringify("a,b,c,d".split(",", 2)))
console.log(JSON.stringify("".split(",")))
console.log(JSON.stringify("a-b-c".split("-", 0)))
console.log(JSON.stringify("hello".split("l")))
console.log(JSON.stringify("a,,b".split(",")))
