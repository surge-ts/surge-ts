interface OpTypes {
    readonly equal: unique symbol;
    readonly other: unique symbol;
}
declare const Op: OpTypes;

const twice = {
    [Op.equal]: "first",
    [Op.equal]: "last",
};
const distinct = {
    [Op.equal]: "first",
    [Op.other]: "last",
};
const widened: symbol = Op.equal;
const notTheOther: typeof Op.other = Op.equal;
const iterator = { [Symbol.iterator]: 1, [Symbol.asyncIterator]: 2 };
