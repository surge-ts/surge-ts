// @strict: true
// @lib: es2023
const inObject = { map: new Map<string, boolean>(), set: new Set<number>() };
const mapRead: number = inObject.map;
const setRead: number = inObject.set;
class Holder {
  map = new Map<string, boolean>();
  dates = new Set<Date>();
}
const holder = new Holder();
const heldMap: number = holder.map;
const heldDates: number = holder.dates;
