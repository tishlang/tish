// Test262: URI encoding/decoding functions

// encodeURI - preserves allowed characters
assert.sameValue(encodeURI("hello"), "hello", "encodeURI plain text");
assert.sameValue(encodeURI("hello world"), "hello%20world", "encodeURI space");
assert.sameValue(encodeURI("a/b"), "a/b", "encodeURI preserves slash");
assert.sameValue(encodeURI("a:b"), "a:b", "encodeURI preserves colon");
assert.sameValue(encodeURI("a?b=c"), "a?b=c", "encodeURI preserves query");

// encodeURI - encodes special characters
assert.sameValue(encodeURI("hello world"), "hello%20world", "encodeURI space");

// decodeURI
assert.sameValue(decodeURI("hello"), "hello", "decodeURI plain text");
assert.sameValue(decodeURI("hello%20world"), "hello world", "decodeURI space");
assert.sameValue(decodeURI("a/b"), "a/b", "decodeURI with slash");

// encodeURI/decodeURI round-trip
let original = "hello world!";
let encoded = encodeURI(original);
let decoded = decodeURI(encoded);
assert.sameValue(decoded, original, "encodeURI/decodeURI round-trip");

// encodeURIComponent - encodes more characters
assert.sameValue(encodeURIComponent("hello"), "hello", "encodeURIComponent plain");
assert.sameValue(encodeURIComponent("hello world"), "hello%20world", "encodeURIComponent space");
assert.sameValue(encodeURIComponent("a/b"), "a%2Fb", "encodeURIComponent encodes slash");
assert.sameValue(encodeURIComponent("a:b"), "a%3Ab", "encodeURIComponent encodes colon");
assert.sameValue(encodeURIComponent("a?b=c"), "a%3Fb%3Dc", "encodeURIComponent encodes query");
assert.sameValue(encodeURIComponent("a&b"), "a%26b", "encodeURIComponent encodes ampersand");
assert.sameValue(encodeURIComponent("a=b"), "a%3Db", "encodeURIComponent encodes equals");

// decodeURIComponent
assert.sameValue(decodeURIComponent("hello"), "hello", "decodeURIComponent plain");
assert.sameValue(decodeURIComponent("hello%20world"), "hello world", "decodeURIComponent space");
assert.sameValue(decodeURIComponent("a%2Fb"), "a/b", "decodeURIComponent slash");
assert.sameValue(decodeURIComponent("a%3Ab"), "a:b", "decodeURIComponent colon");
assert.sameValue(decodeURIComponent("a%3Fb%3Dc"), "a?b=c", "decodeURIComponent query");

// encodeURIComponent/decodeURIComponent round-trip
original = "key=value&foo=bar/baz";
encoded = encodeURIComponent(original);
decoded = decodeURIComponent(encoded);
assert.sameValue(decoded, original, "encodeURIComponent/decodeURIComponent round-trip");

// Difference between encodeURI and encodeURIComponent
let url = "http://example.com/path?query=hello world";
let uriEncoded = encodeURI(url);
// encodeURI preserves URL structure
assert.sameValue(uriEncoded.includes("://"), true, "encodeURI preserves ://");
assert.sameValue(uriEncoded.includes("?"), true, "encodeURI preserves ?");
assert.sameValue(uriEncoded.includes("%20"), true, "encodeURI encodes space");

// Practical: building query strings
function buildQueryParam(key, value) {
    return encodeURIComponent(key) + "=" + encodeURIComponent(value);
}
assert.sameValue(buildQueryParam("name", "John Doe"), "name=John%20Doe", "query param with space");
assert.sameValue(buildQueryParam("a&b", "c=d"), "a%26b=c%3Dd", "query param with special chars");

// Multiple round-trips
let complex = "path/to/file?a=1&b=2#section";
let step1 = encodeURIComponent(complex);
let step2 = decodeURIComponent(step1);
assert.sameValue(step2, complex, "complex round-trip");

// Empty string
assert.sameValue(encodeURI(""), "", "encodeURI empty");
assert.sameValue(decodeURI(""), "", "decodeURI empty");
assert.sameValue(encodeURIComponent(""), "", "encodeURIComponent empty");
assert.sameValue(decodeURIComponent(""), "", "decodeURIComponent empty");

printTestResults();
