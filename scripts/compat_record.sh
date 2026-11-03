#!/usr/bin/env bash
# Records what the reference tools give for each compatibility case
# (docs/design/quality.md): runs every case under crates/cli/tests/compat/
# against the real tool, at its pinned version, in a container, and writes
# the case's `stdout` and `exit`. Commit what changes after reading it.
#
#   scripts/compat_record.sh            every group
#   scripts/compat_record.sh base64     one group
#
# A case is a folder: `args` (one argument a line; @stdin is the path of the
# case's stdin file), `stdin`, and optionally `files/`, the folder the command
# runs in, with the files it is given.
set -euo pipefail
cd "$(dirname "$0")/.."

# The reference of each group, and the image that has it at its pinned version.
declare -A IMAGE COMMAND VERSION
for tool in base64 md5sum sha1sum sha256sum sha512sum b2sum; do
  IMAGE[$tool]=debian:13
  COMMAND[$tool]=$tool
  VERSION[$tool]="$tool (GNU coreutils) 9.7"
done

groups=("${@:-${!IMAGE[@]}}")
for group in "${groups[@]}"; do
  image=${IMAGE[$group]:?no reference for $group}
  docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/w" -w /w "$image" bash -c '
    set -euo pipefail
    command=$1 version=$2 dir=$3
    got=$("$command" --version | head -n 1)
    [ "$got" = "$version" ] || { echo "$command is \"$got\", not \"$version\"" >&2; exit 1; }
    for case in "$dir"/*/; do
      case=$(cd "$case" && pwd)
      mapfile -t args < "$case/args"
      args=("${args[@]/#@stdin/$case/stdin}")
      cd "$case/files" 2> /dev/null || cd "$case"
      set +e
      "$command" "${args[@]}" < "$case/stdin" > "$case/stdout" 2> /dev/null
      echo $? > "$case/exit"
      set -e
      cd /w
    done
  ' _ "${COMMAND[$group]}" "${VERSION[$group]}" "crates/cli/tests/compat/$group"
  echo "$group: recorded $(ls -d crates/cli/tests/compat/"$group"/*/ | wc -l) cases"
done
