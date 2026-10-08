class Store<T> {
  req<TOut = unknown>(o: number): Promise<TOut> {
    void o;
    return null!;
  }
  pair<U>(u: U): [T, U] {
    void u;
    return null!;
  }
  check() {
    const explicit: Promise<string> = this.req<number>(1);
    const inferred: [T, number] = this.pair('x');
    return [explicit, inferred];
  }
}
export { Store };
