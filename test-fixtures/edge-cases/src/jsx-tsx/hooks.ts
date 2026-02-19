export function useAppState() {
  let count = 0;
  return {
    get count() { return count; },
    increment: () => { count++; },
    decrement: () => { count--; },
  };
}

export function useTheme() {
  return { theme: "light" as const };
}
