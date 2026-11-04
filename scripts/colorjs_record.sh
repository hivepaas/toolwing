#!/usr/bin/env bash
# Records Color.js 0.5.2's conversions of the colors in
# crates/tools/testdata/color/colorjs-inputs.txt as colorjs.jsonl: sRGB as
# CSS Color 4 maps it, hsl and hwb of that, lab, lch, oklab, oklch, and the
# WCAG 2.1 contrast with white and black. Color.js is by the editors of CSS
# Color 4. In node:22-slim, from npm; commit what changes after reading it.
set -euo pipefail
cd "$(dirname "$0")/.."
dir=crates/tools/testdata/color
docker run --rm -u "$(id -u):$(id -g)" -e HOME=/tmp -v "$PWD/$dir:/data" -w /tmp node:22-slim bash -c '
  set -euo pipefail
  npm install -s colorjs.io@0.5.2 > /dev/null 2>&1
  got=$(node -p "require(\"fs\").readFileSync(\"node_modules/colorjs.io/package.json\",\"utf8\").match(/\"version\": \"([^\"]+)\"/)[1]")
  [ "$got" = 0.5.2 ] || { echo "Color.js is $got" >&2; exit 1; }
  cat > record.mjs <<"JS"
import Color from "colorjs.io";
import fs from "node:fs";
const r = (c) => c.coords.map(v => (v === null || Number.isNaN(v)) ? null : v);
const lines = [];
for (const s of fs.readFileSync("/data/colorjs-inputs.txt", "utf8").split("\n").filter(Boolean)) {
  const c = new Color(s);
  const mapped = c.to("srgb").toGamut({ method: "css" });
  lines.push(JSON.stringify({
    input: s, srgb: r(mapped), alpha: c.alpha, in_srgb: c.inGamut("srgb"),
    hsl: r(mapped.to("hsl")), hwb: r(mapped.to("hwb")),
    lab: r(c.to("lab")), lch: r(c.to("lch")), oklab: r(c.to("oklab")), oklch: r(c.to("oklch")),
    contrast_white: c.contrast(new Color("white"), "WCAG21"),
    contrast_black: c.contrast(new Color("black"), "WCAG21"),
  }));
}
fs.writeFileSync("/data/colorjs.jsonl", lines.join("\n") + "\n");
console.log(lines.length);
JS
  node record.mjs
'
