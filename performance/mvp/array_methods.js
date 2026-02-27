// Array methods tests

// push - adds elements to end, returns new length
let arr = [1, 2, 3]
let len = arr.push(4)
console.log(len)
console.log(arr.length)
console.log(arr[3])

arr.push(5, 6)
console.log(arr.length)

// pop - removes and returns last element
let last = arr.pop()
console.log(last)
console.log(arr.length)

// shift - removes and returns first element
let first = arr.shift()
console.log(first)
console.log(arr.length)
console.log(arr[0])

// unshift - adds elements to beginning, returns new length
let arr2 = [3, 4]
let newLen = arr2.unshift(1, 2)
console.log(newLen)
console.log(arr2[0])
console.log(arr2[1])

// indexOf - returns index of element, or -1
let nums = [10, 20, 30, 20]
console.log(nums.indexOf(20))
console.log(nums.indexOf(99))

// includes - returns boolean
console.log(nums.includes(30))
console.log(nums.includes(99))

// join - joins elements with separator
let parts = ["a", "b", "c"]
console.log(parts.join("-"))
console.log(parts.join(""))
console.log(parts.join())

// reverse - reverses in place, returns array
let rev = [1, 2, 3]
rev.reverse()
console.log(rev[0])
console.log(rev[2])

// slice - returns new array (non-mutating)
let src = [1, 2, 3, 4, 5]
let sl1 = src.slice(1, 3)
console.log(sl1.length)
console.log(sl1[0])
console.log(sl1[1])

// negative indices
let sl2 = src.slice(-2)
console.log(sl2.length)
console.log(sl2[0])

// concat - combines arrays
let a1 = [1, 2]
let a2 = [3, 4]
let combined = a1.concat(a2)
console.log(combined.length)
console.log(combined[2])

// concat with values
let combined2 = a1.concat(5, [6, 7])
console.log(combined2.length)

// Empty array operations
let empty = []
console.log(empty.pop() ?? null)
console.log(empty.shift() ?? null)
console.log(empty.length)
