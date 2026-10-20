type Thenable<Value> = {
  then<V>(onFulfilled: (value: Value) => V | Promise<V> | Thenable<V>): Thenable<V>
}

export const toThenable =
  <Result, Input>(fn: (input: Input) => Result | Promise<Result> | Thenable<Result>) =>
  (input: Input): Thenable<Result> => {
    const result = fn(input)
    return {
      then(onFulfilled) {
        return toThenable(onFulfilled)(result as Result)
      },
    }
  }
