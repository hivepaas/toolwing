#!/usr/bin/env python3
"""jaq with compat.jq included before the filter: the prelude prototype, for
measuring only. $JAQ names the jaq binary (default: jaq on PATH)."""
import os, shutil, sys, tempfile

JAQ = os.environ.get("JAQ") or shutil.which("jaq")
LIB = os.path.dirname(os.path.abspath(__file__))
TAKES = {"--arg": 2, "--argjson": 2, "--slurpfile": 2, "--rawfile": 2, "--indent": 1,
         "-L": 1, "--library-path": 1, "--from": 1, "--to": 1}

args, out, i, seen_filter = sys.argv[1:], ["-L", LIB], 0, False
from_file = "-f" in args
while i < len(args):
    a = args[i]
    if not seen_filter and a in TAKES:
        n = TAKES[a]
        out += args[i:i + 1 + n]
        i += 1 + n
        continue
    if not seen_filter and a.startswith("-") and a != "-":
        out.append(a)
        i += 1
        continue
    if not seen_filter:
        seen_filter = True
        if from_file:
            with tempfile.NamedTemporaryFile("w", suffix=".jq", delete=False) as f:
                f.write('include "compat"; ' + open(a).read())
            a = f.name
        else:
            a = 'include "compat"; ' + a
    out.append(a)
    i += 1
os.execv(JAQ, [JAQ] + out)
