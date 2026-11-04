#!/usr/bin/env python3
"""Turns html5lib-tests' tokenizer tests of character references into the
pairs `html` is tested with (crates/tools/testdata/html/): one JSON array a
line, [input, output], for text (named.jsonl, numeric.jsonl, mixed.jsonl)
and for attribute values (attribute.jsonl).

  scripts/html5lib_entities.py PATH/TO/html5lib-tests/tokenizer COMMIT
"""
import json, os, re, sys

src, commit = sys.argv[1], sys.argv[2]
out = os.path.join(os.path.dirname(__file__), "..", "crates", "tools", "testdata", "html")
attr = re.compile(r'^<h a=(?:"(.*)"|\'(.*)\'|([^"\'\s>]*))>$', re.S)

def text(tokens):
    if all(t[0] == "Character" for t in tokens):
        return "".join(t[1] for t in tokens)
    return None

files = {"named.jsonl": [], "numeric.jsonl": [], "mixed.jsonl": [], "attribute.jsonl": []}
for name, target in [("namedEntities.test", "named.jsonl"), ("numericEntities.test", "numeric.jsonl"), ("entities.test", "mixed.jsonl")]:
    for t in json.load(open(os.path.join(src, name)))["tests"]:
        if t.get("doubleEscaped") or t.get("initialStates", ["Data state"]) != ["Data state"]:
            continue
        got = text(t["output"])
        if got is not None:
            files[target].append([t["input"], got])
            continue
        m = attr.match(t["input"])
        o = t["output"]
        if m and len(o) == 1 and o[0][0] == "StartTag" and list(o[0][2]) == ["a"]:
            raw = next(g for g in m.groups() if g is not None)
            files["attribute.jsonl"].append([raw, o[0][2]["a"]])
for f, pairs in files.items():
    with open(os.path.join(out, f), "w") as w:
        for p in pairs:
            w.write(json.dumps(p, ensure_ascii=True) + "\n")
    print(f, len(pairs))
with open(os.path.join(out, "SOURCE"), "w") as w:
    w.write(f"html5lib-tests {commit}, tokenizer/ (MIT), by scripts/html5lib_entities.py\n")
