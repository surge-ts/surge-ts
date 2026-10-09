// @strict: true
// @lib: es2023
interface User { id: string; email: string }
class Repo<T> {
  private items = new Map<string, T>();
  private list: T[] = [];
  get(id: string) { return this.items.get(id); }
  all() { return [...this.items.values()]; }
  first() { return this.list[0]; }
  store() { return this.items; }
}
const repo = new Repo<User>();
const got: User = repo.get("1");
const all: number = repo.all();
const first: number = repo.first();
const store: number = repo.store();
function spread<T>(m: Map<string, T>, s: Set<T>, xs: T[], x: T) {
  const values: number = [...m.values()];
  const members: number = [...s];
  const mixed: string = [x, ...xs];
}
