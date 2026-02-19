// Auto-generated entry point
import { Module_1 } from "./modules/module_1";
import { Module_2 } from "./modules/module_2";
import { Module_3 } from "./modules/module_3";
import { Module_4 } from "./modules/module_4";
import { Module_5 } from "./modules/module_5";
import { Module_6 } from "./modules/module_6";
import { Module_7 } from "./modules/module_7";
import { Module_8 } from "./modules/module_8";
import { Module_9 } from "./modules/module_9";
import { Module_10 } from "./modules/module_10";

async function main() {
  const m1 = new Module_1();
  console.log(m1.run(), m1.describe());
  const m2 = new Module_2();
  console.log(m2.run(), m2.describe());
  const m3 = new Module_3();
  console.log(m3.run(), m3.describe());
  const m4 = new Module_4();
  console.log(m4.run(), m4.describe());
  const m5 = new Module_5();
  console.log(m5.run(), m5.describe());
  const m6 = new Module_6();
  console.log(m6.run(), m6.describe());
  const m7 = new Module_7();
  console.log(m7.run(), m7.describe());
  const m8 = new Module_8();
  console.log(m8.run(), m8.describe());
  const m9 = new Module_9();
  console.log(m9.run(), m9.describe());
  const m10 = new Module_10();
  console.log(m10.run(), m10.describe());
}

main();
