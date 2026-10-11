// @noImplicitAny: true
// @target: es2022
// @surge-compare: messages
declare const k: string;
const lit = "lit";
class A {
    set ["y"](value) {}
    set [k](value) {}
    set [lit](value) {}
}
const o = {
    set [k](value) {},
};
