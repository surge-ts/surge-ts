// @noImplicitAny: true
// @target: es2015
var calls = { f() { return this.f(); } };
var mutual = { f() { return this.g(); }, g() { return this.f(); } };
var getter = { get p() { return this.p; } };
var prints = { f() { return this.missing; }, g() { return 1; } };
var alias = { get p() { const self = this; return self.missing; } };
var inCall = { f() { return this.missing(); } };
var converted = { f() { return this as number; } };
var afterReturn = { f() { this.missing; return 1; } };
var nested = { f() { return () => this.f(); } };
var annotated = { f(): number { return this.missing; } };
var chain = { f() { return this.g(); }, g() { return 1; } };
var typed: number = calls.f();
export {};
