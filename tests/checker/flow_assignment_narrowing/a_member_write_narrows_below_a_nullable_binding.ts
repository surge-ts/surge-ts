// @strict: true
// @lib: es2023,dom
// The written member's declared type is read through the binding's one object
// member, so the write narrows it however the binding itself was narrowed.
interface Entry { canvas: HTMLDivElement | undefined }
declare function make(): HTMLDivElement;
declare const entries: Map<string, Entry>;
export function inGuard(id: string) {
  const entry = entries.get(id);
  if (entry) {
    if (!entry.canvas) entry.canvas = make();
    return entry.canvas.style;
  }
  return undefined;
}
export function afterReturn(id: string) {
  let entry: Entry | undefined = entries.get(id);
  if (!entry) return;
  if (!entry.canvas) entry.canvas = make();
  return entry.canvas.style;
}
