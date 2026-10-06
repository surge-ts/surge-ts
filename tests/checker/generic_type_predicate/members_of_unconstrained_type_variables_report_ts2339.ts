// @strict: true
function f<T>(x: T) { x.hasOwnProperty('a'); }
function g<T>(x: T | { data: T }) { x.hasOwnProperty('a'); }
function h<T extends object>(x: T) { x.hasOwnProperty('a'); }
function k<T>(x: T) { x.toString(); }
