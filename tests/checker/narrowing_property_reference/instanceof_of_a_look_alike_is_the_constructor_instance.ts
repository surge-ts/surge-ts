// @strict: true
// No union member derives from `Error`, so `getNarrowedType` falls back to
// the candidate itself, a subtype of the structurally identical member.
interface ErrorShaped {
  name: string;
  message: string;
  stack?: string;
}
interface Tagged {
  tag: number;
}
export function aLookAlikeIsNotAnInstance(value: ErrorShaped | Tagged): string {
  if (value instanceof Error) {
    const e: Error = value;
    return e.message + value.message;
  }
  return "tag" in value ? String(value.tag) : value.name;
}
