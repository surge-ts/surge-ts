// @target: esnext
async function* g3(): AsyncIterator<any, { x: 'x' }, any> {
  return Promise.resolve({ x: 'x' });
}
async function* g4(): AsyncIterator<any, { x: 'x' }, any> {
  const ret = { x: 'x' };
  return Promise.resolve(ret);
}
