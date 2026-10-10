// @jsx: preserve
// A tag that several of `IntrinsicElements`' template literal index
// signatures take gets the intersection of their props: `foobarbaz` is both
// `foo${string}` and `foobar${string}`, so its `prop` must be `'literal'`.
declare namespace JSX {
  interface IntrinsicElements {
    [k: `foo${string}`]: { prop: string };
    [k: `foobar${string}`]: { prop: 'literal' };
  }
}

<foobaz prop="smth" />;
<foobaz prop={10} />;

<foobarbaz prop="literal"/>;
<foobarbaz prop="smth"/>;
