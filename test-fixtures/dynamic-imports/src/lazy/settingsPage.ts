export async function handleSettings(): Promise<void> {
  console.log("Showing settings page");
}

export function getDefaultSettings(): Record<string, string> {
  return {
    theme: "light",
    language: "en",
  };
}
