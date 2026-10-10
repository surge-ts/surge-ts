// @strict: true
// @lib: es2023
const text = "abcabc";
const matches = [...text.matchAll(/a/g)];
const wrongMatches: number = matches;
const indexes = Array.from(text.matchAll(/b/g), (match) => match.index);
const wrongIndexes: string[] = indexes;
const points = [text.codePointAt(0)];
const wrongPoints: number[] = points;
const values = [3, 1, 2];
const sorted = [...values.toSorted()];
const wrongSorted: string[] = sorted;
