#!/bin/bash
# Generates a large TypeScript project for performance testing
# Usage: ./generate-perf-test.sh [num_files] [output_dir]

NUM_FILES=${1:-100}
OUTPUT_DIR=${2:-"./perf-test-project"}

mkdir -p "$OUTPUT_DIR/src/modules"
mkdir -p "$OUTPUT_DIR/src/utils"
mkdir -p "$OUTPUT_DIR/src/services"

# Create tsconfig
cat > "$OUTPUT_DIR/tsconfig.json" << 'TSEOF'
{
  "compilerOptions": {
    "target": "ES2020",
    "module": "ESNext",
    "moduleResolution": "node",
    "strict": true,
    "outDir": "./dist",
    "rootDir": "./src"
  },
  "include": ["src/**/*.ts"]
}
TSEOF

# Create utility modules
for i in $(seq 1 $((NUM_FILES / 5))); do
  cat > "$OUTPUT_DIR/src/utils/util_${i}.ts" << EOF
export function utilFunc_${i}_a(x: number): number {
  return x * ${i};
}

export function utilFunc_${i}_b(s: string): string {
  return s + "_${i}";
}

export function utilFunc_${i}_c(arr: number[]): number {
  return arr.reduce((a, b) => a + b, 0) + ${i};
}

// Dead function
export function utilFunc_${i}_dead(): void {
  console.log("dead code in util_${i}");
}
EOF
done

# Create service modules that depend on utils
for i in $(seq 1 $((NUM_FILES / 5))); do
  UTIL_IDX=$((((i - 1) % (NUM_FILES / 5)) + 1))
  NEXT_UTIL_IDX=$(((i % (NUM_FILES / 5)) + 1))
  cat > "$OUTPUT_DIR/src/services/service_${i}.ts" << EOF
import { utilFunc_${UTIL_IDX}_a, utilFunc_${UTIL_IDX}_b } from "../utils/util_${UTIL_IDX}";
import { utilFunc_${NEXT_UTIL_IDX}_c } from "../utils/util_${NEXT_UTIL_IDX}";

export class Service_${i} {
  process(input: number): number {
    return utilFunc_${UTIL_IDX}_a(input);
  }

  format(input: string): string {
    return utilFunc_${UTIL_IDX}_b(input);
  }

  aggregate(items: number[]): number {
    return utilFunc_${NEXT_UTIL_IDX}_c(items);
  }
}

// Dead method
export function deadServiceHelper_${i}(): string {
  return "dead_${i}";
}
EOF
done

# Create modules that depend on services
for i in $(seq 1 $((NUM_FILES / 5))); do
  SVC_IDX=$((((i - 1) % (NUM_FILES / 5)) + 1))
  cat > "$OUTPUT_DIR/src/modules/module_${i}.ts" << EOF
import { Service_${SVC_IDX} } from "../services/service_${SVC_IDX}";

export class Module_${i} {
  private service: Service_${SVC_IDX};

  constructor() {
    this.service = new Service_${SVC_IDX}();
  }

  run(): number {
    return this.service.process(${i});
  }

  describe(): string {
    return this.service.format("module_${i}");
  }
}
EOF
done

# Create some dead modules (not imported by anything)
for i in $(seq 1 $((NUM_FILES / 10))); do
  cat > "$OUTPUT_DIR/src/modules/dead_module_${i}.ts" << EOF
// This entire module is dead code - never imported
export class DeadModule_${i} {
  getValue(): number {
    return ${i};
  }
}

export function deadFunc_${i}(): string {
  return "completely dead ${i}";
}
EOF
done

# Create entry point that imports about half the modules
HALF=$((NUM_FILES / 10))
cat > "$OUTPUT_DIR/src/index.ts" << 'EOF'
// Auto-generated entry point
EOF

for i in $(seq 1 $HALF); do
  echo "import { Module_${i} } from \"./modules/module_${i}\";" >> "$OUTPUT_DIR/src/index.ts"
done

cat >> "$OUTPUT_DIR/src/index.ts" << 'EOF'

async function main() {
EOF

for i in $(seq 1 $HALF); do
  echo "  const m${i} = new Module_${i}();" >> "$OUTPUT_DIR/src/index.ts"
  echo "  console.log(m${i}.run(), m${i}.describe());" >> "$OUTPUT_DIR/src/index.ts"
done

echo "}" >> "$OUTPUT_DIR/src/index.ts"
echo "" >> "$OUTPUT_DIR/src/index.ts"
echo "main();" >> "$OUTPUT_DIR/src/index.ts"

# Count files
TOTAL=$(find "$OUTPUT_DIR/src" -name "*.ts" | wc -l | tr -d ' ')
echo "Generated $TOTAL TypeScript files in $OUTPUT_DIR"
echo "  - utils: $(ls "$OUTPUT_DIR/src/utils" | wc -l | tr -d ' ') files"
echo "  - services: $(ls "$OUTPUT_DIR/src/services" | wc -l | tr -d ' ') files"
echo "  - modules: $(ls "$OUTPUT_DIR/src/modules" | wc -l | tr -d ' ') files"
echo "  - dead modules: $((NUM_FILES / 10)) files"
