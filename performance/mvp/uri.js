// MVP perf: decodeURI and encodeURI
console.log(encodeURI("hello world"));
console.log(decodeURI("hello%20world"));
console.log(encodeURI("a=b&c=d"));
console.log(decodeURI("a%3Db%26c%3Dd"));
