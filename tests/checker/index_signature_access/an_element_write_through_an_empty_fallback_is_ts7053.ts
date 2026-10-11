function foo(options?: { a: string, b: number }) {
    let x1 = (options || {})["a"];
    (options || {})["a"] = 1;
}
declare let u: { a: string } | { b: number };
u["a"] = "x";
declare let w: { a: string } | { [k: string]: string };
w["a"] = "y";
w["z"] = "y";
