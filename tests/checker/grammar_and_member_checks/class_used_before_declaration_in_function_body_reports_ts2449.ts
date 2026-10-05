function f() {
    new D();
    const y = D.x;
    let t: D | undefined;
    const g = () => new D();
    class D { static x = 1 }
    new D();
}
function h() {
    console.log(ClassA.Value);
    return;
    class ClassA { static Value = 1234; }
}
