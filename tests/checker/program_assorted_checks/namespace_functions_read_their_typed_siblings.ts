// @strict: true
// @lib: es2023
namespace Geo {
  export const PI = 3.14;
  export const unit = "cm";
  export function area(r: number) {
    return PI * r * r;
  }
  export function label(r: number) {
    return `${area(r)}${unit}`;
  }
}
const wrongArea: string = Geo.area(1);
const wrongLabel: number = Geo.label(1);
