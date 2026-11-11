// Test helpers only: nothing in the app imports this.

/**
 * The fastest of `runs` timed calls of `fn`, in ms, with the last call's result.
 *
 * For guards against an algorithm turning quadratic again, not for frame budgets. The
 * fastest run is the one least disturbed by other work on the machine (a busy CI runner,
 * GC, a cold JIT), so it tracks the code's own cost; pair it with a limit far above
 * normal and far below the regression it guards against.
 */
export function fastestOf<T>(runs: number, fn: () => T): { ms: number; result: T } {
  let ms = Infinity;
  let result!: T;
  for (let i = 0; i < runs; i++) {
    const start = performance.now();
    result = fn();
    ms = Math.min(ms, performance.now() - start);
  }
  return { ms, result };
}
