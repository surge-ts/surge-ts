function f() {
    console.log(Baz.value);
    return;
    namespace Baz { export const value = 1234 }
}
