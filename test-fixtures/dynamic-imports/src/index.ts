// Entry point with dynamic imports
import { Router } from "./router";

const router = new Router();

async function main() {
  // Dynamic import based on route
  const handler = await router.getHandler("/users");
  if (handler) {
    await handler();
  }

  // Direct dynamic import
  const { mathUtils } = await import("./modules/math");
  console.log(mathUtils.add(1, 2));

  // Conditional dynamic import
  if (process.env.ENABLE_ANALYTICS) {
    const analytics = await import("./modules/analytics");
    analytics.trackEvent("app_start");
  }
}

main();
