type Context<K, P = never> = [P] extends [never] ? { key: K; page?: unknown } : { key: K; page: P };
type Handler<T, K, P = never> = (context: Context<K, P>) => T;

export function relate<P>(open: Handler<number, string, P>, paged: Handler<number, string, string>) {
  const fromOpen: Handler<number, string, never> = open;
  const fromPaged: Handler<number, string, never> = paged;
  const wrongResult: Handler<string, string, never> = open;
  return [fromOpen, fromPaged, wrongResult];
}

interface BaseOptions<T, P = never> {
  handler?: Handler<T, string, P>;
  tag?: string;
}

interface PagedOptions<T, P> extends BaseOptions<T, P> {
  initialPage: P;
}

class Observer<T> {
  setOptions(options: BaseOptions<T>): void {
    void options;
  }
}

export class PagedObserver<T, P> extends Observer<T> {
  setOptions(options: PagedOptions<T, P>): void {
    super.setOptions(options);
  }
}
