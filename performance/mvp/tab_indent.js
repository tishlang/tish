// MVP test: tab indentation (JS equivalent of tab_indent.tish)
const x = 1;
if (x === 1) {
	console.log("tab-indented");
	const y = 2;
	if (y === 2)
		console.log("nested with tabs");
}
console.log("done");
