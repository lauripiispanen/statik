export interface CounterState {
  count: number;
  increment: () => void;
  decrement: () => void;
  reset: () => void;
}

export function useCounter(initial: number = 0): CounterState {
  let count = initial;
  return {
    get count() { return count; },
    increment: () => { count++; },
    decrement: () => { count--; },
    reset: () => { count = initial; },
  };
}
