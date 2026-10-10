// @module: commonjs
// @resolveJsonModule: true
// tsc parses a JSON file as one value (`parseJSONText`) and reports where its
// text is no JSON — a single-quoted key or string, a bare word — as syntax.
import single = require("./single.json");
import bare = require("./bare.json");
// @filename: single.json
{
    'a': true,
    "b": 'hello'
}
// @filename: bare.json
contents Not read
