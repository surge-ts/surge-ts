// @strict: true
let s: string | boolean;
if ((typeof s !== 'string' && !s) || typeof s !== 'boolean') {
  let b: string = (typeof s !== 'boolean') ? s : "x";
}
function f() {
  let t: string | boolean;
  let d: string = (typeof t !== 'boolean') ? t : "x";
  if (typeof t !== 'boolean') { let e: string = t; }
  if ((typeof t !== 'string' && !t) || typeof t !== 'boolean') {
    let g: string = (typeof t !== 'boolean') ? t : "x";
  }
}
