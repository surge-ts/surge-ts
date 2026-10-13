// @jsx: react-jsx
// @module: esnext
// @moduleResolution: bundler
// @filename: node_modules/react/package.json
{"name":"react","version":"1.0.0","types":"index.d.ts"}
// @filename: node_modules/react/index.d.ts
export {};
// @filename: node_modules/react/jsx-runtime.d.ts
import "./global";
// @filename: node_modules/react/global.d.ts
declare namespace JSX {
    interface IntrinsicElements { [x: string]: any; }
}
declare var fromDependency: number;
// @filename: file.tsx
export const a = <div></div>;
const s: string = fromDependency;
