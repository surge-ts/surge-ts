// @strict: true
const foo = { bar: 'a' };
const x = ({ [foo.bar]: c }) => undefined;
const y = ({ ['lit']: d }) => undefined;
