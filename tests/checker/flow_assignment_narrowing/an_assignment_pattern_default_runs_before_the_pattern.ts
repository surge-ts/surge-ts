// @strict: false
// @target: esnext
{
    let a: 0 | 1 = 0;
    let b: 0 | 1 | 9;
    [{ [(a = 1)]: b } = [9, a] as const] = [];
    const bb: 0 = b;
}
{
    let a: 0 | 1 = 1;
    let b: 0 | 1 | 9;
    [{ [a]: b } = [9, a = 0] as const] = [];
    const bb: 9 = b;
}
{
    let a: 0 | 1 = 1;
    let b: 0 | 1 | 8 | 9;
    [{ [a]: b } = [a = 0, 9] as const] = [[8, 9] as const];
    const bb: 0 | 8 = b;
}
{
    let a: 0 | 1 = 1;
    let b: 0 | 1 | 9;
    [{ [a]: b } = [9, a = 0] as const] = [];
    const wrong: 0 = b;
}
