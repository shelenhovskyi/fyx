/** Own-property lookup, so input such as `"constructor"` never resolves to `Object.prototype`. */
export function own<T>(record: Readonly<Record<string, T>>, key: string): T | undefined {
  return Object.hasOwn(record, key) ? record[key] : undefined;
}
