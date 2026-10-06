type AOrArrA<T> = T | T[];
const arr: AOrArrA<{ x?: "ok" }> = [{ x: "ok" }];
arr.push({ x: "ok" });
function f() {
    const local: AOrArrA<{ x?: "ok" }> = [{ x: "ok" }];
    local.push({ x: "ok" });
}
