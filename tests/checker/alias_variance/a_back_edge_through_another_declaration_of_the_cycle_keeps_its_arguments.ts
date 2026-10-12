interface List<T> {
    data: T;
    next: List<T>;
    owner: OwnerList<T>;
}
interface OwnerList<U> extends List<List<U>> {
    name: string;
}
declare var list: List<string>;
declare var ownerList: OwnerList<string>;
list = ownerList;
