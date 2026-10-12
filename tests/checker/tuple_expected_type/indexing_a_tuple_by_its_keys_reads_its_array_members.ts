type Pair = [{ name: "x" }, { name: "y" }];
type Values = Pair[keyof Pair];
const values: boolean = null! as Values;
type CopyWithin = [1, 2]["copyWithin"];
const copyWithin: boolean = null! as CopyWithin;
