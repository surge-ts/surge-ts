interface List<T> {
    data: T;
    next: List<T>;
    owner: OwnerList<T>;
}
interface OwnerList<U> extends List<List<U>> {
    name: string;
}
declare function take<T>(l: List<T>): void;
function assigned<T>(x: T) {
    var list: List<T>;
    var ownerList!: OwnerList<T>;
    list = ownerList;
}
function returned<T>(o: OwnerList<T>): List<T> {
    return o;
}
function passed<T>(o: OwnerList<T>) {
    take<T>(o);
}
function fits<T>(o: OwnerList<T>, l: List<T>) {
    l = o.next.data;
}
