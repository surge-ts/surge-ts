// @allowJs: true
// @checkJs: true
// @strict: true
// @filename: rectangle.js
class Rectangle {}
module.exports = { Rectangle };

// @filename: chunk.js
module.exports = class Chunk {
    constructor() {
        this.chunk = 1;
    }
};

// @filename: index.js
const { Rectangle } = require('./rectangle');
/** @type {Rectangle} */
var r;
const Chunk = require('./chunk');
/** @type {Chunk} */
var c;
c.chunk;
