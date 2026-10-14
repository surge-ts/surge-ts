// @strict: true
// A `default`-less switch whose cases cover the discriminant ends the body, so
// a declaration's or method's inferred return type has no implicit `undefined`.
type Kind = "student" | "staff";
export function pick(type: Kind) {
  switch (type) {
    case "student":
      return { id: 1 };
    case "staff":
      return { id: 2 };
  }
}
export const id: number = pick("staff").id;
class Picker {
  pick(type: Kind) {
    switch (type) {
      case "student":
        return 1;
      case "staff":
        return 2;
    }
  }
}
export const n: number = new Picker().pick("student");
export function partial(type: Kind) {
  switch (type) {
    case "student":
      return 1;
  }
}
export const m: number = partial("student");
