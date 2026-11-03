#!/usr/bin/env python3
"""Run jq's .test files against any jq-like binary, judging as jq's own
runner does (src/jq_test.c): outputs compared as values in order, an error
after the expected outputs is fine, %%FAIL passes when the program does not
compile (messages not compared). Prints one JSON line per test."""
import json, math, os, subprocess, sys, tempfile

def parse(line):
    s = line.strip()
    try:
        return json.loads(s)
    except ValueError:
        # jq's parser accepts nan/NaN literals; json.loads only NaN.
        return json.loads(s.replace("nan", "NaN"))

def equal(a, b):
    if isinstance(a, bool) or isinstance(b, bool):
        return type(a) is type(b) and a == b
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(equal(x, y) for x, y in zip(a, b))
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(equal(a[k], b[k]) for k in a)
    return type(a) is type(b) and a == b

def skip(line):
    s = line.lstrip(" \t")
    return s == "" or s.startswith("#") or s == "\n"

def cases(path):
    lines = open(path, encoding="utf-8").read().split("\n")
    i, must_fail = 0, False
    while i < len(lines):
        line, lineno = lines[i], i + 1
        i += 1
        if skip(line):
            continue
        if line in ("%%FAIL", "%%FAIL IGNORE MSG"):
            must_fail = True
            continue
        block = []
        while i < len(lines) and not skip(lines[i]):
            block.append(lines[i]); i += 1
        if must_fail:
            yield dict(line=lineno, prog=line, fail=True, msg=block)
        else:
            yield dict(line=lineno, prog=line, fail=False,
                       input=block[0] if block else None, expected=block[1:])
        must_fail = False

def run(binary, mods, case, timeout):
    with tempfile.NamedTemporaryFile("w", suffix=".jq", delete=False) as f:
        f.write(case["prog"])
    try:
        p = subprocess.run([binary, "-c", "-L", mods, "-f", f.name],
                           input=(case.get("input") or "null").encode(),
                           capture_output=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return "timeout", ""
    finally:
        os.unlink(f.name)
    if case["fail"]:
        return ("pass", "") if p.returncode == 3 else ("compiled", p.stderr.decode()[:200])
    if p.returncode == 3:
        return "compile-error", p.stderr.decode(errors="replace")[:300]
    got = [l for l in p.stdout.decode(errors="replace").split("\n") if l != ""]
    for n, want in enumerate(case["expected"]):
        if n >= len(got):
            why = p.stderr.decode(errors="replace").strip()[:300] or "fewer outputs"
            return "insufficient", why
        try:
            ok = equal(parse(want), parse(got[n]))
        except ValueError:
            ok = False
        if not ok:
            return "wrong", f"want {want.strip()} got {got[n][:200]}"
    if len(got) > len(case["expected"]):
        return "superfluous", got[len(case["expected"])][:200]
    return "pass", ""

if __name__ == "__main__":
    binary, mods, *files = sys.argv[1:]
    for path in files:
        for c in cases(path):
            status, detail = run(binary, mods, c, 10)
            print(json.dumps(dict(file=os.path.basename(path), line=c["line"],
                                  prog=c["prog"], status=status, detail=detail)))
