interface Success<T> { data: T; type?: 'data'; id?: string }
interface Idle<E> { type: 'state'; state: 'idle'; error: E | null }
interface Busy { type: 'state'; state: 'busy'; error: null }
type ConnectionState<E> = Idle<E> | Busy;

interface Envelope<T, E> {
  result: { type: 'started'; data?: never } | Success<T> | ConnectionState<E>;
}

export function lastId<TFallback>(envelope: Envelope<number, Error>, fallback: TFallback) {
  if ((!envelope.result.type || envelope.result.type === 'data') && envelope.result.id) {
    return envelope.result.id;
  }
  if (envelope.result.type === 'state') {
    return envelope.result.state;
  }
  return fallback;
}
