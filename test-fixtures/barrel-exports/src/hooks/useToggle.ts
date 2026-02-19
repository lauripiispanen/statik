// useToggle hook - used via barrel import
export interface ToggleState {
  value: boolean;
  toggle: () => void;
  setTrue: () => void;
  setFalse: () => void;
}

export function useToggle(initial: boolean = false): ToggleState {
  let value = initial;
  return {
    get value() { return value; },
    toggle: () => { value = !value; },
    setTrue: () => { value = true; },
    setFalse: () => { value = false; },
  };
}
