// @jsx: preserve
// @strict: true
namespace JSX {
  export interface Element {}
}
function test<P>(wrappedProps: P) {
  const MySFC = function (props: P) { return null; };
  const x = <MySFC />;
  const z = <MySFC {...wrappedProps} />;
  const Arrow = (props: P) => null;
  const y = <Arrow />;
}
function test2<P extends { a?: number }>(p: P) {
  const Arrow = (props: P) => null;
  const x4 = <Arrow />;
  const x5 = <Arrow a={1} />;
  const x6 = <Arrow {...p} />;
}
function Generic<T>(props: T) { return null; }
const outside = <Generic />;
