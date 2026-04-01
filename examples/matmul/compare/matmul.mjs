// Same algorithm as ../src/main.tish — keep `N` in sync when comparing runtimes.
const N = 256
const len = N * N

const a = new Float64Array(len)
const b = new Float64Array(len)
const c = new Float64Array(len)
for (let i = 0; i < len; i++) {
  a[i] = (i % 997) / 997
  b[i] = (i % 991) / 991
}

const t0 = Date.now()
for (let i = 0; i < N; i++) {
  for (let j = 0; j < N; j++) {
    let sum = 0
    for (let k = 0; k < N; k++) {
      sum += a[i * N + k] * b[k * N + j]
    }
    c[i * N + j] = sum
  }
}
const t1 = Date.now()

const check = c[0] + c[N - 1] + c[(N - 1) * N] + c[len - 1]
console.log(`matmul ${N}x${N} ms=${t1 - t0} check=${check}`)
