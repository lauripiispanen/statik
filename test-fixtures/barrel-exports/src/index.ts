// Entry point that imports through barrel files
import { Button, Input } from "./components";
import { useToggle } from "./hooks";
import { clamp } from "./utils";

// Use only some of the barrel-exported items
const button = new Button("Click me");
button.render();

const input = new Input("text", "Enter name");
input.render();

const toggle = useToggle(false);
toggle.toggle();
console.log(toggle.value);

console.log(clamp(15, 0, 10)); // 10
