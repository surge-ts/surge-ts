// @strict: true
class C { x = 1; m() { let y: typeof this.x = "s"; } }
function Test3(this: { no: number }) { let x: typeof this.no = "s"; }
