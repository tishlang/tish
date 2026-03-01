// Array methods performance test - intensive operations

let iterations = 1000

// Test push/pop performance
let arr = []
let i = 0
while (i < iterations) {
    arr.push(i)
    i = i + 1
}
console.log(arr.length)

while (arr.length > 0) {
    arr.pop()
}
console.log(arr.length)

// Test shift/unshift (more expensive O(n))
let arr2 = []
i = 0
let limit = 100
while (i < limit) {
    arr2.unshift(i)
    i = i + 1
}
console.log(arr2.length)

while (arr2.length > 0) {
    arr2.shift()
}
console.log(arr2.length)

// Test indexOf/includes
let searchArr = []
i = 0
while (i < 500) {
    searchArr.push(i)
    i = i + 1
}

let found = 0
i = 0
while (i < 100) {
    if (searchArr.includes(i * 5)) {
        found = found + 1
    }
    i = i + 1
}
console.log(found)

// Test slice/concat
let base = [1, 2, 3, 4, 5]
let result = []
i = 0
while (i < 200) {
    let sliced = base.slice(1, 4)
    result = result.concat(sliced)
    i = i + 1
}
console.log(result.length)

// Test join
let words = ["hello", "world", "test", "array", "methods"]
let joined = ""
i = 0
while (i < 200) {
    joined = words.join("-")
    i = i + 1
}
console.log(joined)

// Test reverse
let revArr = [1, 2, 3, 4, 5]
i = 0
while (i < 500) {
    revArr.reverse()
    i = i + 1
}
console.log(revArr[0])
