declare const anything: any;
declare const key: string;

function f1({ [anything]: x }) {}
const f2 = ({ [anything]: y }) => {};
function f3({ [key]: z }) {}
function f4({ [anything]: w }: Record<string, number>) {}
