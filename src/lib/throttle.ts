/**
 * A leading-edge throttle: `run(fn)` calls `fn` unless the previous accepted call was less than
 * `ms` ago, and says whether it did. Time comes from `Date.now`, so a test can fake it.
 */
export function createThrottle(ms: number, now: () => number = Date.now) {
  let last = Number.NEGATIVE_INFINITY;
  return {
    run(fn: () => void): boolean {
      const at = now();
      if (at - last < ms) return false;
      last = at;
      fn();
      return true;
    },
  };
}
