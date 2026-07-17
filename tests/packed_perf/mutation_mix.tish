// Sustained numeric-array mutation: push/pop/splice/sort in a hot loop. Exercises the packed
// method fast paths (no deopt — stays numeric). Checksum-guarded.
let ROUNDS = 200000
let t0 = Date.now()
let a = [5, 3, 8, 1, 9, 2, 7]
let check = 0
let r = 0
while (r < ROUNDS) {
  a.push((r * 31 + 17) % 100)
  a.push((r * 13 + 3) % 100)
  a.splice(1, 1)
  if (a.length > 12) { a.pop() }
  a.sort()
  check = (check + a[0] + a[a.length - 1]) % 1000000007
  r = r + 1
}
console.log("GAUNTLET packed_mut " + (Date.now() - t0) + " " + check)
