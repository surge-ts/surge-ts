// @strict: true
const bar = 'a' as string;
const x = ({ [bar]: c }) => undefined;
const y = ({ ['lit']: d }) => undefined;
