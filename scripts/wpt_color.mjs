// Turns web-platform-tests' css/css-color/parsing tests into the cases
// `color` is tested with (crates/tools/testdata/color/): each page's
// scripts run with the test functions recording what they are given.
//
//   node scripts/wpt_color.mjs WPT_CHECKOUT/css/css-color/parsing COMMIT
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";

const [dir, commit] = process.argv.slice(2);
const out = path.join(path.dirname(new URL(import.meta.url).pathname), "..", "crates/tools/testdata/color");
const computed = [], invalid = [];
// What needs a page around it: lengths, variables, currentcolor, system
// colors, and what CSS Color 5 adds.
const needsPage = /sign\(|\d(em|px|cqw|vw|vh|rem)\b|var\(|currentcolor|from |color-mix|light-dark|contrast-color|color-layers|canvas|buttontext|linktext|highlight|field/i;
for (const f of fs.readdirSync(dir).sort()) {
  if (!/^color-(computed|invalid)-(rgb|hsl|hwb|lab|hex-color|named-color|color-function)\.html$/.test(f)) continue;
  const html = fs.readFileSync(path.join(dir, f), "utf8");
  const scripts = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(m => m[1]);
  const record = (input, want) => {
    if (needsPage.test(input)) return;
    computed.push([input, Array.isArray(want) ? want : [want]]);
  };
  const sandbox = {
    test_computed_value: (prop, input, want) => prop === "color" && record(input, want),
    fuzzy_test_computed_color: (input, want) => record(input, want),
    test_invalid_value: (prop, input) => { if (prop === "color" && !needsPage.test(input)) invalid.push(input); },
    document: { getElementById: () => ({ style: {} }) },
    console,
  };
  vm.createContext(sandbox);
  for (const s of scripts) vm.runInContext(s, sandbox);
  // Pages that list their cases and loop over them after.
  if (Array.isArray(sandbox.tests)) {
    for (const t of sandbox.tests) {
      if (f.startsWith("color-invalid")) { if (!needsPage.test(t[0])) invalid.push(t[0]); }
    }
  }
}
const uniq = (a, key) => [...new Map(a.map(x => [key(x), x])).values()];
const lines = (a) => a.map(x => JSON.stringify(x)).join("\n") + "\n";
fs.writeFileSync(path.join(out, "wpt-computed.jsonl"), lines(uniq(computed, x => x[0])));
fs.writeFileSync(path.join(out, "wpt-invalid.jsonl"), lines(uniq(invalid, x => x)));
fs.writeFileSync(path.join(out, "SOURCE"),
  `wpt-*.jsonl: web-platform-tests ${commit}, css/css-color/parsing (BSD-3-Clause), by scripts/wpt_color.mjs\n` +
  `colorjs.jsonl: Color.js 0.5.2 (MIT), by scripts/colorjs_record.sh\n`);
console.log(computed.length, invalid.length);
