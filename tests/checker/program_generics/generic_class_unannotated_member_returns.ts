class Client<TRouter> {
  private async request<TOutput = unknown>(path: string): Promise<TOutput> {
    void path;
    return null!;
  }
  private count(): number {
    return 1;
  }
  query(path: string) {
    return this.request<unknown>(path);
  }
  size() {
    return this.count();
  }
  router(): TRouter {
    return null!;
  }
}

export async function use(client: Client<{ a: 1 }>) {
  const result = await client.query('x');
  const asString: string = result;
  const size: string = client.size();
  return [asString, size];
}
