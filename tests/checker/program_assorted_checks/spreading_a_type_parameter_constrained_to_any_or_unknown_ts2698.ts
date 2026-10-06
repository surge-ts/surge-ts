function f<T extends any>(props: T) { const o = { ...props }; return o; }
function g<T>(props: T) { const o = { ...props }; return o; }
function h<T extends unknown>(props: T) { const o = { ...props }; return o; }
