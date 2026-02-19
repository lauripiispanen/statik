# Expected Results for Test Fixtures

This document describes the expected output from statik for each test fixture project.
It serves as the ground truth for validation testing.

---

## 1. basic-project (11 source files)

### Files
- `src/index.ts` - entry point
- `src/models/user.ts` - User model
- `src/models/post.ts` - Post model
- `src/utils/format.ts` - formatting utilities
- `src/utils/logger.ts` - Logger class
- `src/utils/helpers.ts` - entirely dead utility file
- `src/services/userService.ts` - user service
- `src/services/postService.ts` - dead service
- `src/controllers/userController.ts` - dead controller

### Expected Dependency Graph (from index.ts)
```
index.ts
  -> services/userService.ts
     -> models/user.ts
     -> utils/logger.ts
        -> utils/format.ts
  -> utils/format.ts
  -> utils/logger.ts
     -> utils/format.ts
```

### Dead Code - Expected Detections

#### Dead Files (entirely unreachable from entry point)
- `src/utils/helpers.ts` - no file imports this
- `src/services/postService.ts` - no file imports this
- `src/controllers/userController.ts` - no file imports this

#### Dead Exported Symbols (in reachable files)
| File | Symbol | Type | Why Dead |
|------|--------|------|----------|
| `models/user.ts` | `UserPreferences` | interface | Never imported |
| `models/user.ts` | `UserWithoutId` | type | Never imported |
| `models/user.ts` | `deleteUser` | function | Never imported |
| `models/user.ts` | `UserValidator` | class | Never imported |
| `models/post.ts` | `PostComment` | interface | Never imported |
| `models/post.ts` | `slugify` | function | Never imported |
| `utils/format.ts` | `formatCurrency` | function | Never imported |
| `utils/format.ts` | `truncate` | function | Never imported |
| `utils/format.ts` | `capitalize` | function | Never imported |
| `utils/logger.ts` | `createChildLogger` | function | Never imported |
| `services/userService.ts` | `removeUser` (method) | method | Never called |
| `services/userService.ts` | `getUsersByRole` (method) | method | Never called |

#### Live Symbols (should NOT be flagged as dead)
| File | Symbol | Used By |
|------|--------|---------|
| `models/user.ts` | `User` | userService, postService (if reachable) |
| `models/user.ts` | `UserRole` | userService |
| `models/user.ts` | `createUser` | userService |
| `utils/format.ts` | `formatDate` | index.ts |
| `utils/format.ts` | `formatDateTime` | format.ts (internal, via formatLogMessage) |
| `utils/format.ts` | `formatLogMessage` | logger.ts |
| `utils/logger.ts` | `Logger` | index.ts, userService |
| `utils/logger.ts` | `LogLevel` | logger.ts (internal type) |
| `services/userService.ts` | `UserService` | index.ts |

---

## 2. circular-deps (6 source files)

### Expected Circular Dependencies
1. **A <-> B cycle**: `moduleA.ts` imports `moduleB.ts`, `moduleB.ts` imports `moduleA.ts`
2. **Triangular cycle**: `triangleX.ts` -> `triangleZ.ts` -> `triangleY.ts` -> `triangleX.ts`

### Expected Symbols
- `ModuleA` class (used by index.ts)
- `ModuleB` class (used by moduleA)
- `getModuleAVersion` function (used by moduleB)
- `TriangleX`, `TriangleY`, `TriangleZ` classes (only X is used if reached from index, but they are interconnected)
- `TreeNode` interface, `createTree`, `flattenTree` (selfRef.ts - potentially dead if not imported from index)

### Note
`selfRef.ts` is NOT imported by `index.ts`, so it is a dead file. The triangle files are also not imported from `index.ts`, so they are dead files too.

---

## 3. dynamic-imports (9 source files)

### Expected Behavior
Dynamic imports (`await import(...)`) should be tracked as dependencies.

### Dynamic Import Chain
```
index.ts
  -> router.ts (static import)
     -~> lazy/usersPage.ts (dynamic import)
         -> modules/stringUtils.ts
     -~> lazy/postsPage.ts (dynamic import)
     -~> lazy/settingsPage.ts (dynamic import)
  -~> modules/math.ts (dynamic import)
  -~> modules/analytics.ts (conditional dynamic import)
```

### Barrel File (modules/index.ts)
The barrel file re-exports from math, analytics, and stringUtils. It is NOT directly imported by anyone in this fixture (index.ts imports from the individual modules directly). The barrel file itself is dead.

### Dead Code
| File | Symbol | Reason |
|------|--------|--------|
| `modules/math.ts` | `fibonacci` | Never imported directly; re-exported as `fib` in barrel but barrel is dead |
| `modules/analytics.ts` | `trackError` | Never called |
| `modules/stringUtils.ts` | `reverseString` | Never imported |
| `modules/stringUtils.ts` | `padLeft` | Only re-exported through dead barrel |
| `lazy/settingsPage.ts` | `getDefaultSettings` | Never called (and page itself is arguable) |
| `modules/index.ts` | entire barrel file | Never imported by any module |

### Tricky Case
- `settingsPage.ts` IS registered as a dynamic route in the router, meaning it CAN be loaded. But the route is never navigated to from index.ts. Statik should probably mark this as reachable (via dynamic import registration), not dead.

---

## 4. barrel-exports (16 source files)

### Purpose
Tests barrel file (index.ts re-exports) patterns extensively.

### What IS Used (traced through barrels)
- `Button` class (via components/index.ts barrel)
- `Input` class (via components/index.ts barrel)
- `useToggle` function (via hooks/index.ts barrel)
- `clamp` function (via utils/index.ts wildcard barrel)
- `ClickableProps` interface (used by Button)
- `ComponentProps` interface (used by Input, types barrel)

### What is DEAD (exported through barrel but never consumed)
| Barrel | Dead Export | Defined In |
|--------|------------|------------|
| `components/index.ts` | `Select` | `components/Select.ts` |
| `components/index.ts` | `Modal` | `components/Modal.ts` |
| `components/index.ts` | `Dropdown` | `components/Dropdown.ts` |
| `hooks/index.ts` | `useCounter` | `hooks/useCounter.ts` |
| `hooks/index.ts` | `useDebounce` | `hooks/useDebounce.ts` |
| `utils/index.ts` (wildcard) | `lerp` | `utils/math.ts` |
| `utils/index.ts` (wildcard) | `randomInt` | `utils/math.ts` |
| `utils/index.ts` (wildcard) | `isEmail` | `utils/validation.ts` |
| `utils/index.ts` (wildcard) | `isURL` | `utils/validation.ts` |
| `utils/index.ts` (wildcard) | `isNotEmpty` | `utils/validation.ts` |

### Tricky: Wildcard Re-exports
`export * from "./math"` makes ALL exports of math.ts available. But only `clamp` is actually used by consumers. The rest (`lerp`, `randomInt`) are "transitively exported" but never consumed.

---

## 5. edge-cases (10 source files)

### Default Exports
- `defaultExport.ts` exports `Greeter` as default, imported as `MyClass` in index.ts
- `DEFAULT_NAME` constant: dead (never imported)
- `createGreeter` function: dead (never imported)
- `farewell` method on Greeter: dead (never called)

### Namespace Imports
- `import * as ns from "./namespaceImport"` - `helperA` and `helperB` are used via `ns.helperA()` / `ns.helperB()`
- `helperC` is exported but NOT accessed through the namespace. This is a judgment call:
  - Conservative: `helperC` is "available" via namespace, not dead
  - Aggressive: `helperC` is never actually called, so it's dead
  - Statik should ideally detect that `ns.helperC()` is never called

### Renamed Exports
- `secretValue as renamed` is imported as `originalName` - used
- `secretValue as alsoRenamed` - dead (never imported)
- `getSecret` function - dead (never imported)

### Type-Only Exports
- `Config` interface - used (imported by index.ts)
- `ConfigKey` type - dead
- `DatabaseConfig` interface - dead
- `DEFAULT_CONFIG` constant - dead (runtime value)
- `mergeConfigs` function - dead

### Function Overloads
- `processItems` - used (both overloads)
- `Parser` class - dead

### Const Enums
- `Color` enum - used
- `Direction` enum - dead (never imported)
- `Priority` enum - dead
- `colorToString` function - dead (never imported)

### Higher-Order Functions
- `createFactory` - used
- `compose` - dead
- `pipe` - dead
- `memoize` - dead

### Side-Effect Import
- `sideEffects.ts` is imported as `import "./sideEffects"` (side-effect only)
- The module IS imported and its top-level code runs
- But `register` and `get` exports are never used by the importer
- Judgment call: are `register`/`get` dead? The module is alive but its named exports are unused by the only importer.

### Mixed Exports
- `CONSTANT_A` - dead (never imported from index.ts, which imports `MixedClass`)
- `CONSTANT_B` - dead
- `MixedClass` - used
- `AnotherClass` - dead
- `internalFunc` - dead
- `defaultFunc` (default export) - dead (index.ts imports `{ MixedClass }` named)
- `MixedType` - dead
- `UnusedType` - dead

---

## 6. monorepo (10 source files across 4 packages)

### Cross-Package Dependencies
```
api/src/index.ts -> core/src/index.ts -> shared/src/index.ts
ui/src/index.ts  -> core/src/index.ts -> shared/src/index.ts
```

### Dead Code Across Packages
| Package | Symbol | Reason |
|---------|--------|--------|
| shared | `unwrap` | Not imported by core, api, or ui |
| shared | `unwrapOr` | Not imported by core, api, or ui |
| shared | `Session` | Not imported by core, api, or ui |
| shared | `Pagination` | Not imported by core, api, or ui |
| shared | `validatePassword` | Not imported by core, api, or ui |
| shared | `EventEmitter.once` | Method never called |
| shared | `EventEmitter.removeAllListeners` | Method never called |
| core | `NotificationService` | Exported but never imported by api or ui |
| core | `UserManager.deleteUser` | Method never called by api or ui |
| api | `handleDeleteUser` | Function never called |
| ui | `UserDetailComponent` | Class never imported |

### Tricky: Package Boundary Analysis
Statik needs to handle cross-package imports. The monorepo uses relative paths (`../../shared/src`) which is different from workspace protocols (`@shared/result`). Both patterns should be supported.
