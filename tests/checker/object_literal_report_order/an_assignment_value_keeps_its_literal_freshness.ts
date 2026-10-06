function fx10(obj1: { x?: number }, obj2: { x?: number; y?: number }) {
  obj1 = obj2 = { x: 1, y: 2 };
  obj2 = obj1 = { x: 1, y: 2 };
}

function fx11(): { x?: number } {
  let obj: { x?: number; y?: number };
  return obj = { x: 1, y: 2 };
}

function fx12() {
  let d: { x?: number };
  let e: { x?: number; y?: number } = {};
  d = (e = { y: 3 });
  const f: { x?: number } = e = { x: 1, y: 2 };
  const g: { x?: number; y?: number } = e = { x: 1 };
  return [d, f, g];
}

export { fx10, fx11, fx12 };
