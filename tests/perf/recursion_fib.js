function fib(n) {
  if (n < 2) { return n }
  return fib(n - 1) + fib(n - 2)
}
let t0 = Date.now()
let r = fib(35)
console.log("GAUNTLET recursion_fib " + (Date.now() - t0) + " " + r)
