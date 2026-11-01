// @surge-compare: messages
interface List<T> { data: T; next: List<T>; owner: OwnerList<T>; }
interface OwnerList<U> extends List<List<U>> { name: string; }
function other<T>(x: T) {
    var list: List<T>;
    var ownerList!: OwnerList<T>;
    list = ownerList;
}

function single<T>(a: { data: number }) { let b: { data: T } = a; }
function firstFailingMember<T>(a: { data: number, other: string }) { let b: { data: T, other: number } = a; }
function nested<T>(a: { data: { x: number } }) { let b: { data: { x: T } } = a; }
function constrained<T extends string>(a: { data: string }) { let b: { data: T } = a; }
function indexed<T, K extends keyof T>(a: { data: number, y: string }) { let b: { data: T[K], y: number } = a; }
function conditional<T>(a: { data: number, y: string }) { let b: { data: T extends string ? 1 : 2, y: number } = a; }

interface Base<T> { data: T; other: string }
interface I<T> extends Base<T> { data: number; other: string }
interface J<T> extends Base<T> { data: number; other: number }
function returned<T>(cb: () => { data: number }) { const r: () => { data: T } = cb; }

function comparable<T>(s: { x: T | number, y: string }, o: { x: T, y: number }) {
    switch (o) { case s: break; }
}

export { other, single, firstFailingMember, nested, constrained, indexed, conditional, returned, comparable };
export type { I, J };
