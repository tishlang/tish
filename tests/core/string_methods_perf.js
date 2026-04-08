// String methods performance test - intensive operations

let iterations = 500

// Test indexOf/includes
let text = "The quick brown fox jumps over the lazy dog"
let found = 0
let i = 0
while (i < iterations) {
    if (text.includes("fox")) {
        found = found + 1
    }
    if (text.indexOf("lazy") > -1) {
        found = found + 1
    }
    if (text.lastIndexOf("dog") > -1) {
        found = found + 1
    }
    i = i + 1
}
console.log(found)

// Test split/join roundtrip
let csv = "a,b,c,d,e,f,g"
let result = ""
i = 0
while (i < iterations) {
    let parts = csv.split(",")
    result = parts.join("-")
    i = i + 1
}
console.log(result)

// Test slice/substring
let str = "Hello World!"
let sliced = ""
i = 0
while (i < iterations) {
    sliced = str.slice(0, 5)
    sliced = str.substring(6, 11)
    i = i + 1
}
console.log(sliced)

// Test replace/replaceAll
let template = "{{name}} is {{age}} years old"
let replaced = ""
i = 0
while (i < iterations) {
    replaced = template.replace("{{name}}", "John")
    replaced = replaced.replace("{{age}}", "25")
    i = i + 1
}
console.log(replaced)

// Test toUpperCase/toLowerCase
let mixed = "Hello World"
let upper = ""
let lower = ""
i = 0
while (i < iterations) {
    upper = mixed.toUpperCase()
    lower = mixed.toLowerCase()
    i = i + 1
}
console.log(upper)
console.log(lower)

// Test trim
let padded = "   trimmed   "
let trimmed = ""
i = 0
while (i < iterations) {
    trimmed = padded.trim()
    i = i + 1
}
console.log(trimmed)

// Test startsWith/endsWith
let path = "/api/users/123"
let count = 0
i = 0
while (i < iterations) {
    if (path.startsWith("/api")) {
        count = count + 1
    }
    if (path.endsWith("123")) {
        count = count + 1
    }
    i = i + 1
}
console.log(count)

// Test repeat/pad
let base = "x"
let repeated = ""
i = 0
while (i < 200) {
    repeated = base.repeat(10)
    let padded2 = "5".padStart(5, "0")
    i = i + 1
}
console.log(repeated.length)

// Test charAt/charCodeAt
let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
let sum = 0
i = 0
while (i < iterations) {
    let code = alphabet.charCodeAt(0)
    sum = sum + code
    i = i + 1
}
console.log(sum)
