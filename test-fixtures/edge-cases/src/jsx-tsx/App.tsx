import { Button } from "./Button";
import { useAppState } from "./hooks";

export function App() {
  const { count, increment } = useAppState();

  return (
    <div>
      <h1>Count: {count}</h1>
      <Button label="Increment" onClick={increment} />
    </div>
  );
}
