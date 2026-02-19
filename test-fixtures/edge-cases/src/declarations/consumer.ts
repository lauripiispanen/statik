import styles from "./styles.css";
import { track, identify } from "custom-analytics";

export function initApp(): void {
  console.log(styles.container);
  identify("user-123");
  track("app_init");
}
