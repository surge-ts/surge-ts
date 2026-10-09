// @strict: true
declare namespace PropTypes {
  export const number: { checks: 'number' };
  export const text: { checks: 'string' };
}
declare const plain: { p: 1 };
class Component {
  static propTypes = { foo: PropTypes.number, bar: PropTypes.text };
  static fallback = plain;
  instance = PropTypes.number;
}
export const foo: 'string' = Component.propTypes.foo.checks;
export const fallback: 2 = Component.fallback.p;
export const declared: 'string' = new Component().instance.checks;
