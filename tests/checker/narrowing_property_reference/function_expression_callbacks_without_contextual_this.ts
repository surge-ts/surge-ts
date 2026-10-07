// @noImplicitThis: true
class Test2 {
    data = [1];
    m() { this.data.find(function (d) { return d === this.data.length; }); }
}
declare function on(cb: (this: { a: number }) => void): void;
on(function () { this.a; });
const o = { m() { return this; }, get g() { return this; } };
declare const MyElement: { prototype: { remove(): void; parentNode: any } };
MyElement.prototype.remove = function () { this.parentNode; };
