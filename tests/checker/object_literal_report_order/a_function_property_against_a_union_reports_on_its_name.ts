// `elaborateObjectLiteral` against a union: a function value that fits none of
// the members' types for its property is reported on the property's name,
// unless `elaborateArrowFunction` can point at its return expression instead.
interface ReturnsText {
    pick: (a: string, b: number) => string;
}
interface ReturnsCount {
    pick: (a: string, b: number) => number;
}
declare const textOrCount: string | number;
export const contextual: ReturnsText | ReturnsCount = {
    pick: (a, b) => textOrCount,
};
export const annotated: ReturnsText | ReturnsCount = {
    pick: (a: string, b: number) => textOrCount,
};
export const block: ReturnsText | ReturnsCount = {
    pick: (a, b) => {
        return textOrCount;
    },
};
export const wrongReturn: ReturnsText | ReturnsCount = {
    pick: (a, b) => true,
};
