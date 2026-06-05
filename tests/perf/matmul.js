function bench(N) {
  let len = N * N
  let a = []
  let b = []
  let c = []
  for (let i = 0; i < len; i++) { a.push((i % 997) / 997); b.push((i % 991) / 991); c.push(0) }
  let t0 = Date.now()
  for (let i = 0; i < N; i++) {
    for (let j = 0; j < N; j++) {
      let sum = 0
      for (let k = 0; k < N; k++) { sum = sum + a[i * N + k] * b[k * N + j] }
      c[i * N + j] = sum
    }
  }
  console.log("GAUNTLET matmul " + (Date.now() - t0) + " " + Math.floor((c[0] + c[N - 1]) * 1000))
}
bench(256)
