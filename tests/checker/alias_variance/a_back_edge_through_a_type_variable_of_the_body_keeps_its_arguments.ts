interface List<T> {
    data: T;
    next: List<T>;
    owner: OwnerList<T>;
}
interface OwnerList<U> extends List<List<U>> {
    name: string;
}
function other<T>(x: T) {
    var list: List<T>;
    var ownerList!: OwnerList<T>;
    list = ownerList;
}
