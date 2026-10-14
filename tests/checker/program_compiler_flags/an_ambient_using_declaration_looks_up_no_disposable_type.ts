// @target: es2022
// @lib: es2022
// @noEmit: true
// The global `Disposable` types are looked up for an initializer's check, and
// an ambient `using` has none.
declare namespace UsingInAmbient {
  using resource: any;
}
declare namespace AwaitUsingInAmbient {
  await using resource: any;
}
