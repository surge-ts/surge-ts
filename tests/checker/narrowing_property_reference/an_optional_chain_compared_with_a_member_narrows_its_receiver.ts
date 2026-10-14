// @strict: true
// @lib: es2023,dom
// `a?.p === e.p` holds only where `a` is present when `e.p` cannot be
// `undefined`, whatever expression supplies the compared value.
type Drag = { pointerId: number; dragged: boolean };
export function up(drag: Drag | undefined, e: PointerEvent, maybe: { id?: number }) {
  const dragged = drag?.pointerId === e.pointerId && drag.dragged;
  if (drag?.pointerId === e.pointerId) drag.dragged;
  if (drag?.pointerId === maybe.id) drag.dragged;
  return dragged;
}
