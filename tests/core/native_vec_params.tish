// #175: fns over plain number[]/boolean[] params de-virtualized to native free fns threaded by
// &/&mut Vec, with pairwise-distinct array args (no alias guard). Must stay byte-identical to the
// boxed path and node. Valid in both tish and node; deterministic integer checks.

// (1) &mut boolean[] params + recursion + distinct forwarded args (queens shape).
function mark(n, row, a, b) {
  if (row === n) { return 0 }
  let hits = 0
  let col = 0
  while (col < n) {
    let d = row + col
    if (!a[col] && !b[d]) {
      a[col] = true
      b[d] = true
      hits = hits + 1 + mark(n, row + 1, a, b)
      a[col] = false
      b[d] = false
    }
    col = col + 1
  }
  return hits
}

// (2) &number[] (read) + &mut number[] (write) params, distinct args (spectral shape, no evalA).
function scaleInto(n, src, dst) {
  let i = 0
  while (i < n) {
    dst[i] = src[i] * 2 + 1
    i = i + 1
  }
}

function run() {
  let a = []
  let b = []
  let i = 0
  while (i < 5) { a.push(false); i = i + 1 }
  i = 0
  while (i < 9) { b.push(false); i = i + 1 }
  let m = mark(5, 0, a, b)

  let src = []
  let dst = []
  i = 0
  while (i < 6) { src.push(i); dst.push(0); i = i + 1 }
  scaleInto(6, src, dst)
  let s = 0
  i = 0
  while (i < 6) { s = s + dst[i]; i = i + 1 }

  return m * 1000 + s
}

console.log("native_vec " + run())
