'use strict';

const mc = require("metacall");

mc.metacall_load_from_file("py", ["./math.py"]);

function compute_total(units, price) {
	return mc.metacall("multiply", units, price);
}

function missing_feature(value) {
	return mc.metacall("no_such_function", value);
}

module.exports = { compute_total, missing_feature };
