// @filename: node_modules/h/package.json
{ "name": "h", "types": "index.d.ts" }
// @filename: node_modules/h/index.d.ts
type InputType<D> = { (): D; <P>(schema: P): P };
declare const file: InputType<number>;
declare const helpers_file: typeof file;
declare namespace helpers {
  export { helpers_file as file };
}
export { file, helpers as zfd };
// @filename: main.ts
import { zfd, file } from 'h';
const a: 1 = zfd.file();
const b: 1 = file();
const c: 1 = zfd.file;
