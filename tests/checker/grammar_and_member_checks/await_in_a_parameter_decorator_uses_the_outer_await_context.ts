// @target: esnext
// @experimentalDecorators: true
declare function dec(a: any): any;
function fn(value: Promise<number>): any {
  class Class {
    async method(@dec(await value) arg: number) {}
  }
  return Class;
}
async function outer(value: Promise<number>) {
  class Class {
    method(@dec(await value) arg: number) {}
  }
  return Class;
}
