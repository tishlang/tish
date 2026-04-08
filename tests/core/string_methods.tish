// String methods tests

let str = "Hello, World!"

// indexOf
console.log(str.indexOf("World"))
console.log(str.indexOf("xyz"))
console.log(str.indexOf("o", 5))
console.log(str.indexOf("H", 1))

// includes
console.log(str.includes("World"))
console.log(str.includes("xyz"))
console.log(str.includes("o", 5))
console.log(str.includes("h", -1))

// slice
console.log(str.slice(0, 5))
console.log(str.slice(7))
console.log(str.slice(-6))
console.log(str.slice(0, -7))

// substring
console.log(str.substring(0, 5))
console.log(str.substring(7, 12))

// split
let csv = "a,b,c,d"
let parts = csv.split(",")
console.log(parts.length)
console.log(parts[0])
console.log(parts[2])

// trim
let padded = "  hello  "
console.log(padded.trim())

// toUpperCase / toLowerCase
let mixed = "Hello"
console.log(mixed.toUpperCase())
console.log(mixed.toLowerCase())

// startsWith / endsWith
let path = "/api/users"
console.log(path.startsWith("/api"))
console.log(path.startsWith("/home"))
console.log(path.endsWith("users"))
console.log(path.endsWith("admin"))

// replace / replaceAll
let text = "foo bar foo"
console.log(text.replace("foo", "baz"))
console.log(text.replaceAll("foo", "baz"))

// charAt
console.log("abc".charAt(0))
console.log("abc".charAt(1))
console.log("abc".charAt(99))

// charCodeAt
console.log("ABC".charCodeAt(0))
console.log("ABC".charCodeAt(1))

// repeat
console.log("ab".repeat(3))
console.log("x".repeat(0))

// padStart / padEnd
console.log("5".padStart(3, "0"))
console.log("hi".padEnd(5, "!"))
console.log("hello".padStart(3))

// lastIndexOf
let hay = "abcabc"
console.log(hay.lastIndexOf("a"))
console.log(hay.lastIndexOf("a", 2))
console.log(hay.lastIndexOf("x"))
console.log(hay.lastIndexOf(""))
console.log(hay.lastIndexOf("", 3))
// BMP only so Node (UTF-16 indices) matches Tish (scalar indices)
let uni = "éaé"
console.log(uni.lastIndexOf("a"))
console.log(uni.lastIndexOf("é"))
console.log("aba".lastIndexOf("a", null))

// length property
console.log("test".length)
console.log("".length)
