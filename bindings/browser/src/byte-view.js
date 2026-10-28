// These intrinsics use typed-array internal slots, not a caller's own length getter.
const typedArrayLength = Object.getOwnPropertyDescriptor(
  Object.getPrototypeOf(Uint8Array.prototype), 'length',
).get;
const typedArraySet = Uint8Array.prototype.set;

export function isStageRejection(value) {
  return value instanceof Error;
}

export function isMeaningBytes(value) {
  return value instanceof Uint8Array;
}

export function meaningLength(value) {
  return typedArrayLength.call(value);
}

export function copyMeaning(value, destination) {
  if (meaningLength(value) !== meaningLength(destination)) {
    throw new RangeError('meaning byte view changed length during admission');
  }
  typedArraySet.call(destination, value);
}
