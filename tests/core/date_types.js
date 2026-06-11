// `new Date(...)` constructor + instance methods + statics, identical across interp / VM / native /
// node. All assertions use deterministic UTC / epoch / ISO surface (Tish Dates run in UTC), so the
// output never depends on the host timezone.

// Epoch.
let d0 = new Date(0)
console.log(d0.getTime())
console.log(d0.toISOString())
console.log(d0.getUTCFullYear())

// From epoch millis.
let d = new Date(1623242096789)
console.log(d.toISOString())
console.log(d.getUTCFullYear())
console.log(d.getUTCMonth())   // 0-based: June -> 5
console.log(d.getUTCDate())
console.log(d.getUTCDay())     // 2021-06-09 is a Wednesday -> 3
console.log(d.getUTCHours())
console.log(d.getUTCMinutes())
console.log(d.getUTCSeconds())
console.log(d.getUTCMilliseconds())
console.log(d.valueOf())
// NOTE: getTimezoneOffset() is intentionally 0 (Tish runs Dates in UTC); not asserted here because
// Node reports the *host* offset, which is machine-dependent.

// From ISO string.
let s = new Date("2000-01-01T00:00:00.000Z")
console.log(s.getTime())
console.log(s.getUTCFullYear())

// Date-only string is parsed as UTC midnight.
console.log(new Date("1970-01-02").getTime())

// Pre-epoch (negative millis).
let pre = new Date(-86400000)
console.log(pre.toISOString())
console.log(pre.getUTCDay())   // 1969-12-31 is a Wednesday -> 3

// Statics.
console.log(Date.UTC(2021, 5, 9, 12, 34, 56, 789))
console.log(Date.parse("2021-06-09T12:34:56.789Z"))
console.log(Date.parse("1970-01-01T01:00:00+01:00"))  // offset pulls back to epoch

// setTime mutates in place.
let m = new Date(0)
m.setTime(1000)
console.log(m.getTime())
console.log(m.toISOString())

// Date.now() is a number (value itself is nondeterministic, so only its type is asserted).
console.log(typeof Date.now())

// Leap day.
console.log(new Date(Date.UTC(2020, 1, 29)).toISOString())
