// #247 cross-backend built-in divergences (Math round/min/max/hypot, parseInt radix, includes(NaN),
// toFixed, -0 ToString) — fixed to match the JS target. interp/vm/native/cranelift/node must all
// agree. Valid in tish and node. (instanceof/at/findLast tracked separately.)
console.log("round-neg", Math.round(-2.5))
console.log("round-pos", Math.round(2.5))
console.log("round-half", Math.round(0.5))
console.log("min-empty", Math.min())
console.log("max-empty", Math.max())
console.log("min-nan", Math.min(1, 0 / 0))
console.log("min-vals", Math.min(3, 1, 2))
console.log("max-vals", Math.max(3, 1, 2))
console.log("hypot", Math.hypot(3, 4))
console.log("hypot3", Math.hypot(3, 4, 12))
console.log("hypot-empty", Math.hypot())
console.log("hypot-nan", Math.hypot(0 / 0, 4))
console.log("hypot-inf", Math.hypot(1 / 0, 0 / 0))
console.log("atan2", Math.atan2(1, 1))
// Math.imul: exact 32-bit multiply (ToInt32 both operands, wrapping) — NOT a saturating cast. The
// large-arg cases (args >= 2^31) are the ones the old `as i32` cast got wrong; all backends now agree.
console.log("imul-small", Math.imul(3, 4))
console.log("imul-u32", Math.imul(0xffffffff, 5))
console.log("imul-over", Math.imul(3000000000, 1))
console.log("imul-neg", Math.imul(-5, -5))
// Math.sign preserves the sign of -0 (1/-0 = -Infinity distinguishes -0 from +0).
console.log("sign-neg0", 1 / Math.sign(-0))
console.log("sign-pos0", 1 / Math.sign(0))
console.log("sign-neg", Math.sign(-5))
// padStart/padEnd with an explicit empty fill add NO padding; an absent fill defaults to a space.
console.log("pad-empty", "x".padStart(3, ""))
console.log("pad-space", "x".padStart(3))
console.log("padend-empty", "x".padEnd(3, ""))
// [].reduce with no initial value throws TypeError; an explicit init (incl. null) is returned as-is.
let redThrew = "no"
try { [].reduce((a, b) => a + b) } catch (e) { redThrew = e.name }
console.log("reduce-empty-throw", redThrew)
console.log("reduce-empty-init", [].reduce((a, b) => a + b, 42))
console.log("reduce-empty-null", [].reduce((a, b) => a + b, null))
// Out-of-range / negative string index reads falsy (null in tish, undefined in node) — no throw.
console.log("str-oob", "abc"[10] || "none")
console.log("str-neg", "abc"[-1] || "none")
// JSON.stringify escapes control chars (valid JSON), not raw bytes.
console.log("json-nul", JSON.stringify(String.fromCharCode(0)))
console.log("json-bs", JSON.stringify(String.fromCharCode(8)))
// 2nd-arg support: Array.indexOf fromIndex; String.startsWith/endsWith position.
console.log("idxof-from", [1, 2, 1, 3].indexOf(1, 1))
console.log("idxof-neg", [1, 2, 3, 2, 1].indexOf(2, -2))
console.log("startsw-pos", "abc".startsWith("bc", 1))
console.log("endsw-pos", "abc".endsWith("ab", 2))
// Global isNaN/isFinite coerce their arg via ToNumber (like Number()).
console.log("isnan-str", isNaN("3"), isNaN("x"), isNaN(""))
console.log("isfinite-str", isFinite("3"), isFinite("x"), isFinite(""))
console.log("trimstart", "[" + "  x  ".trimStart() + "]")
console.log("trimend", "[" + "  x  ".trimEnd() + "]")
console.log("num-isint", Number.isInteger(5), Number.isInteger(5.5), Number.isInteger("5"))
console.log("num-safe", Number.isSafeInteger(9007199254740991), Number.isSafeInteger(9007199254740993))
console.log("num-nan", Number.isNaN(0/0), Number.isNaN("x"), Number.isFinite(1/0))
console.log("num-max", Number.MAX_SAFE_INTEGER, Number.MIN_SAFE_INTEGER)
console.log("objis", Object.is(NaN, NaN), Object.is(0, -0), Object.is(2, 2))
console.log("arrof", Array.of(7).length, Array.of(1, 2, 3).join(","))
console.log("lastidx", [1,2,1,3,1].lastIndexOf(1), [1,2,3].lastIndexOf(5), [1,2,1].lastIndexOf(1,-2))
console.log("copywithin", [1,2,3,4,5].copyWithin(0,3).join(","))
console.log("includes-nan", [1, 0 / 0].includes(0 / 0))
console.log("includes-no", [1, 2].includes(3))
console.log("parseint-hex", parseInt("0x1F", 16))
console.log("parseint-auto", parseInt("0x1F"))
console.log("parseint-junk", parseInt("12px"))
console.log("parseint-neg", parseInt("-0x10", 16))
console.log("tofixed", (2.5).toFixed(0))
console.log("tofixed-neg", (-2.5).toFixed(0))
console.log("tofixed-pi", (3.14159).toFixed(2))
// #438: toFixed rounds the number's TRUE decimal value, ties away from zero. Values stored just
// below an apparent tie (2.675 is really 2.67499…) round DOWN — the old `(n*10^d).round()` impl
// rounded these UP. These lock the node-exact behavior across every backend.
console.log("tofixed-tielo", (2.675).toFixed(2), (0.615).toFixed(2), (1.45).toFixed(1), (5.55).toFixed(1))
console.log("tofixed-tielo2", (4.55).toFixed(1), (0.015).toFixed(2), (0.045).toFixed(2), (8.575).toFixed(2))
console.log("tofixed-exact", (0.125).toFixed(2), (0.5).toFixed(0), (9.999).toFixed(2), (999.995).toFixed(2))
// -0 ToString drops the sign (`String(-0) === "0"`); the console *inspect* form keeps it.
console.log("negzero-tostring", (-0).toString())
console.log("negzero-string", String(-0))
console.log("negzero-concat", "" + (-0))
console.log("negzero-template", `${-0}`)
console.log("negzero-inspect", -0)
console.log("cpat", "abc".codePointAt(0), "abc".codePointAt(2))
console.log("substr", "hello".substr(1, 3), "hello".substr(-2))
console.log("from-str", Array.from("abc").join(","))
console.log("from-set", Array.from(new Set([1,1,2,3])).join(","))
console.log("from-len", Array.from({length: 3}, (x, i) => i * 10).join(","))
console.log("from-map", Array.from([1,2,3], x => x * 2).join(","))
console.log("rep-amp", "a-b-c".replace("-", "[$&]"), "a-b-c".replaceAll("-", "[$&]"))
console.log("rep-ctx", "abc".replace("b", "$`"), "abc".replace("b", "$'"), "abc".replace("b", "$$"))
console.log("clz32", Math.clz32(1), Math.clz32(0), Math.clz32(0xffffffff))
console.log("fround", Math.fround(1.1), Math.fround(5.5))
console.log("expm1", Math.expm1(0), Math.log1p(0))
console.log("gopn", Object.getOwnPropertyNames({a: 1, b: 2}).join(","))
console.log("enc-comp", encodeURIComponent("a b&c=d/e?f"))
console.log("dec-comp", decodeURIComponent("a%20b%26c%3Dd"))
console.log("dec-uri", decodeURI("a%20b%26c"))
console.log("rreduce", ["a","b","c"].reduceRight((a, x) => a + x), [1,2,3,4].reduceRight((a, x) => a - x))
console.log("rreduce-init", ["a","b"].reduceRight((a, x) => a + x, "z"))
console.log("akeys", [...["a","b","c"].keys()].join(","))
console.log("avalues", [...["a","b","c"].values()].join(","))
console.log("aentries", [...["a","b"].entries()].map(e => e.join(":")).join("|"))
// structuredClone — deep copy + cycle preservation (#437)
let _sc = { n: 1, arr: [1, { d: "x" }] };
let _scb = structuredClone(_sc);
_scb.n = 9; _scb.arr[1].d = "y";
console.log("sc", _sc.n, _scb.n, _sc.arr[1].d, _scb.arr[1].d, _sc === _scb);
console.log("scprim", structuredClone(7), structuredClone("hi"));
console.log("scarr", JSON.stringify(structuredClone([1,[2,3]])));
let _scc = { k: 1 }; _scc.self = _scc;
let _scd = structuredClone(_scc);
console.log("sccycle", _scd.self === _scd);
// ES2023 change-array-by-copy — toReversed/toSorted/with/toSpliced (#437)
let _bc = [3, 1, 2];
console.log("bcSorted", JSON.stringify(_bc.toSorted()), JSON.stringify(_bc));
console.log("bcSortedCmp", JSON.stringify([3,1,2,10].toSorted((x,y)=>x-y)));
console.log("bcReversed", JSON.stringify(_bc.toReversed()), JSON.stringify(_bc));
console.log("bcWith", JSON.stringify(_bc.with(1, 99)), JSON.stringify([1,2,3].with(-1, 9)));
console.log("bcSpliced", JSON.stringify([1,2,3,4].toSpliced(1, 2, "a", "b")), JSON.stringify([1,2,3,4].toSpliced(1)));
let _bcerr = "none";
try { [1,2,3].with(5, 0); } catch (e) { _bcerr = e.name; }
console.log("bcWithErr", _bcerr);
// Number.toExponential/toPrecision (half-away ties) + Object.hasOwn (#437)
console.log("exp", (12345).toExponential(2), (0.00001).toExponential(2), (2.5).toExponential(0));
console.log("expNo", (12345).toExponential(), (1).toExponential());
console.log("prec", (3.14159).toPrecision(3), (123.456).toPrecision(2), (0.0001234).toPrecision(2));
console.log("precTie", (2.5).toPrecision(1), (1.25).toPrecision(2), (100.5).toPrecision(3), (2.675).toPrecision(3));
console.log("precBig", (123).toPrecision(5), (1000000).toPrecision(3));
let _pe = "none"; try { (1).toPrecision(0); } catch (e) { _pe = e.name; }
console.log("precErr", _pe);
console.log("hasOwn", Object.hasOwn({a:1}, "a"), Object.hasOwn({a:1}, "b"), Object.hasOwn({a:1}, "toString"));
console.log("hasOwnArr", Object.hasOwn([9,8], 0), Object.hasOwn([9,8], 5), Object.hasOwn([9], "length"));
// String.normalize NFC/NFD/NFKC/NFKD (#437)
let _nc = "\u00e9";      // composed e-acute
let _nd = "e\u0301";     // decomposed e + combining acute
console.log("normEq", _nc === _nd, _nc.length, _nd.length);
console.log("normNFC", _nc.normalize("NFC") === _nd.normalize("NFC"), _nd.normalize("NFC").length);
console.log("normNFD", _nc.normalize("NFD") === _nd.normalize("NFD"), _nc.normalize("NFD").length);
console.log("normDefault", "\u00e9".normalize() === "\u00e9".normalize("NFC"));
console.log("normNFKC", "\ufb01".normalize("NFKC"), "\ufb01".normalize("NFKD"));
let _nerr = "none"; try { "x".normalize("BOGUS"); } catch (e) { _nerr = e.name; }
console.log("normErr", _nerr);
// String.matchAll — iterator of exec-style match objects (#437)
let _ma = [..."a1b2c3".matchAll(/([a-z])(\d)/g)];
console.log("maCount", _ma.length, _ma[0][0], _ma[0][1], _ma[0][2], _ma[0].index);
console.log("maAll", _ma.map(m => m[1] + "=" + m[2]).join(","));
let _mao = [];
for (const m of "x9y8".matchAll(/(\d)/g)) { _mao.push(m[1]); }
console.log("maForof", _mao.join(","));
console.log("maEmpty", [..."abc".matchAll(/\d/g)].length);
console.log("maIdx", [..."aXbXc".matchAll(/X/g)].map(m => m.index).join(","));
let _maerr = "none"; try { [..."ab".matchAll(/a/)]; } catch (e) { _maerr = e.name; }
console.log("maErr", _maerr);
// Array string-key indexing coerces canonical integer keys (#432)
let _ai = [10, 20, 30];
console.log("aidx", _ai["0"], _ai["1"], _ai["2"], _ai["0"] === _ai[0]);
let _aik = "";
for (let k in _ai) { _aik = _aik + k + ":" + _ai[k] + " "; }
console.log("aforin", _aik);
_ai["1"] = 99;
console.log("awrite", _ai[1], JSON.stringify(_ai));
// Map key/value VARIABLES reused after set — native E0382 regression (#442). The key/val are moved
// into map_set in the pre-fix codegen; a string key keeps this cross-backend (object keys need
// identity the interpreter can't preserve across the value bridge — tracked separately).
let _mk = new Map();
let _mkey = "id1";
let _mval = "v";
_mk.set(_mkey, _mval);
console.log("mapReuse", _mk.get(_mkey), _mkey, _mval, _mk.size);
let _mk2 = new Map();
let _mv = [1, 2, 3];
_mk2.set("a", _mv);
console.log("mapArrVal", _mk2.get("a")[0], _mv.length);
// function expressions in value position (#464)
let _fe = [1, 2, 3];
_fe.forEach(function(x) { console.log("feEach", x); });
console.log("feMap", _fe.map(function(x) { return x * 2; }).join(","));
let _feAssigned = function(n) { return n + 100; };
console.log("feAssigned", _feAssigned(5));
let _feNamed = function helper(n) { return n * 3; };
console.log("feNamed", _feNamed(4));
console.log("feIife", (function(x) { return x + 1; })(41));
let _feBase = 10;
let _feClosure = function(x) { return _feBase + x; };
console.log("feClosure", _feClosure(5));
