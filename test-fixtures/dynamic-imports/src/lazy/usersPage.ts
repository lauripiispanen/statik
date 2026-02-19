// Lazily loaded module
import { formatName } from "../modules/stringUtils";

export async function handleUsers(): Promise<void> {
  console.log(formatName("john", "doe"));
}
