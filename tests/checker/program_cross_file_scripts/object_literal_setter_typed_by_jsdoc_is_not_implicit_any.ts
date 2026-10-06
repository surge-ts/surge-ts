// @allowJs: true
// @checkJs: true
// @noImplicitAny: true
// @filename: index.js
export const typed = {
  /**
   * @param {number} a
   */
  set x(a) {},
};

export const getterTyped = {
  /** @returns {string} */
  get y() { return ""; },
  set y(a) {},
};

export const untyped = {
  set z(a) {},
};
