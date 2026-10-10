// @allowJs: true
// @checkJs: true
// @strict: true
// A function that reads `arguments` may take its parameters that way, so a
// `@param` naming none is no TS8024 there; its last `@param` would name
// them, though, and one typed as no array is TS8029.
// @filename: a.js
/**
 * @param {string} first
 */
function concat(/* first, second, ... */) {
  var s = ''
  for (var i = 0, l = arguments.length; i < l; i++) {
    s += arguments[i]
  }
  return s
}

/**
 * @param {...string} strings
 */
function correct() {
    arguments
}

/**
 * @param {string} unrelated
 */
function noArguments() {
}
