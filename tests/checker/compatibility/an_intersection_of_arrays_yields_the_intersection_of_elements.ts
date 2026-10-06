type X = { x: 'x' };
type Y = { y: 'y' };
declare const arr: X[] & Y[];
for (const item of arr) {
    item.x;
    item.y;
    item.z;
}
const first = arr[0];
first.y;
