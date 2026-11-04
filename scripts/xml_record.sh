#!/usr/bin/env bash
# Records what xmllint gives for each document in crates/tools/testdata/xml
# (docs/design/quality.md): `xmllint --format` as NAME.format and
# `xmllint --noblanks` as NAME.noblanks, each with its exit code on its first
# line. Debian 13's xmllint, libxml2 2.9.14. Commit what changes after
# reading it.
set -euo pipefail
cd "$(dirname "$0")/.."
image=toolwing-ref/xmllint:2.9.14
if ! docker image inspect "$image" > /dev/null 2>&1; then
  printf 'FROM debian:13\nRUN apt-get update -qq && apt-get install -y -qq libxml2-utils > /dev/null && rm -rf /var/lib/apt/lists/*\n' \
    | docker build -q -t "$image" - > /dev/null
fi
docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/w" -w /w/crates/tools/testdata/xml "$image" bash -c '
  set -euo pipefail
  got=$(xmllint --version 2>&1 | sed -n 1p)
  [ "$got" = "xmllint: using libxml version 20914" ] || { echo "xmllint is \"$got\"" >&2; exit 1; }
  for f in *.xml; do
    name=${f%.xml}
    for mode in format noblanks; do
      set +e
      out=$(xmllint --$mode - < "$f" 2> /dev/null; echo "~$?")
      set -e
      code=${out##*~}
      { echo "$code"; printf "%s" "${out%~*}"; } > "$name.$mode"
    done
  done
'
echo "recorded $(ls crates/tools/testdata/xml/*.xml | wc -l) documents"
