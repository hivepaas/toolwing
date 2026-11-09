#!/usr/bin/env bash
# Records what the reference tools give for each compatibility case
# (docs/design/quality.md): runs every case under crates/cli/tests/compat/
# against the real tool, at its pinned version, in a container, and writes
# the case's `stdout` and `exit`, and its `stderr` for the groups whose
# messages are matched too (STDERR). Commit what changes after reading it.
#
#   scripts/compat_record.sh            every group
#   scripts/compat_record.sh base64     one group
#
# A case is a folder: `args` (one argument a line; @stdin is the path of the
# case's stdin file), `stdin`, and optionally `files/`, the folder the command
# runs in, with the files it is given.
#
# For the groups in WORKDIR (commands that write and remove files, gzip), the
# command runs in a copy of `files/` (files 644, folders 755), every time set
# to 1700000000, in the group's LOCALE if it has one,
# stdin through a pipe, TZ=UTC, with the variables of the case's `env` file
# (NAME=VALUE a line); and what the folder holds after is written to the
# case's `after`: each file's mode, time ("now" when made by the run, under
# umask 022) and name, and the SHA-256 of its
# bytes; for a gzip file, its header (10 bytes, and the name it keeps), its
# last 8 bytes and the SHA-256 of what it decompresses to ("bad" when it does
# not), since tw's deflate bytes are not gzip's.
set -euo pipefail
cd "$(dirname "$0")/.."

# The reference of each group, and the image that has it at its pinned version.
declare -A IMAGE COMMAND VERSION BUILD VERSION_OF STDERR WORKDIR LOCALE
for tool in base64 base32 md5sum sha1sum sha256sum sha512sum b2sum; do
  IMAGE[$tool]=debian:13
  COMMAND[$tool]=$tool
  VERSION[$tool]="$tool (GNU coreutils) 9.7"
done
IMAGE[diff]=debian:13
COMMAND[diff]=diff
VERSION[diff]="diff (GNU diffutils) 3.10"
# jq: its release's static binary, checked against the release's sha256.
JQ=target/compat-ref/jq-1.8.2-linux-amd64
JQ_SHA256=b1c22172dd303f3be49e935aa56aa48a8b7a46e0bc838b4997d3bb451495870f
IMAGE[jq]=debian:13
COMMAND[jq]=/w/$JQ # the container sees the repository at /w
VERSION[jq]="jq-1.8.2"
# b3sum: no Debian package, so an image of Alpine 3.22's (bash for the loop
# below), built here once.
IMAGE[b3sum]=toolwing-ref/b3sum:1.8.2
BUILD[b3sum]=$'FROM alpine:3.22\nRUN apk add --no-cache bash b3sum'
COMMAND[b3sum]=b3sum
VERSION[b3sum]="b3sum 1.8.2"
# argon2: the reference C's command, Debian's build of it. It has no
# --version, so the package's version is checked.
IMAGE[argon2]=toolwing-ref/argon2:20190702
BUILD[argon2]=$'FROM debian:13\nRUN apt-get update -qq && apt-get install -y -qq argon2 > /dev/null && rm -rf /var/lib/apt/lists/*'
COMMAND[argon2]=argon2
VERSION[argon2]="0~20190702+dfsg-4+b2"
VERSION_OF[argon2]="dpkg-query -W -f=\${Version} argon2"
# ipcalc: Red Hat's ipcalc 1.0.3, which Debian ships as ipcalc-ng.
IMAGE[ipcalc]=toolwing-ref/ipcalc:1.0.3
BUILD[ipcalc]=$'FROM debian:13\nRUN apt-get update -qq && apt-get install -y -qq ipcalc-ng > /dev/null && rm -rf /var/lib/apt/lists/*'
COMMAND[ipcalc]=ipcalc-ng
VERSION[ipcalc]="ipcalc-ng 1.0.3"
# qrencode: libqrencode's command, Debian's build.
IMAGE[qrencode]=toolwing-ref/qrencode:4.1.1
BUILD[qrencode]=$'FROM debian:13\nRUN apt-get update -qq && apt-get install -y -qq qrencode > /dev/null && rm -rf /var/lib/apt/lists/*'
COMMAND[qrencode]=qrencode
VERSION[qrencode]="qrencode version 4.1.1"
VERSION_OF[qrencode]="qrencode -V"
# zstd: Debian's build of zstd 1.5.7, libzstd 1.5.7 as zstd-sys bundles it.
IMAGE[zstd]=toolwing-ref/zstd:1.5.7
BUILD[zstd]=$'FROM debian:13\nRUN apt-get update -qq && apt-get install -y -qq zstd > /dev/null && rm -rf /var/lib/apt/lists/*'
COMMAND[zstd]=zstd
VERSION[zstd]="*** Zstandard CLI (64-bit) v1.5.7, by Yann Collet ***"
STDERR[zstd]=1
# gzip: Debian's gzip 1.13.
IMAGE[gzip]=debian:13
COMMAND[gzip]=gzip
VERSION[gzip]="gzip 1.13"
STDERR[gzip]=1
WORKDIR[gzip]=1
# The text tools: Debian's coreutils 9.7, in the C.UTF-8 locale tw's are.
for tool in wc uniq sort; do
  IMAGE[$tool]=debian:13
  COMMAND[$tool]=$tool
  VERSION[$tool]="$tool (GNU coreutils) 9.7"
  STDERR[$tool]=1
  WORKDIR[$tool]=1
  LOCALE[$tool]=C.UTF-8
done

groups=("${@:-${!IMAGE[@]}}")
for group in "${groups[@]}"; do
  if [ "$group" = jq ] && ! echo "$JQ_SHA256  $JQ" | sha256sum -c --status > /dev/null 2>&1; then
    mkdir -p "$(dirname "$JQ")"
    curl -fsSL -o "$JQ" https://github.com/jqlang/jq/releases/download/jq-1.8.2/jq-linux-amd64
    echo "$JQ_SHA256  $JQ" | sha256sum -c --quiet
    chmod +x "$JQ"
  fi
  image=${IMAGE[$group]:?no reference for $group}
  if [ -n "${BUILD[$group]:-}" ] && ! docker image inspect "$image" > /dev/null 2>&1; then
    printf '%s\n' "${BUILD[$group]}" | docker build -q -t "$image" - > /dev/null
  fi
  docker run --rm -u "$(id -u):$(id -g)" -v "$PWD:/w" -w /w "$image" bash -c '
    set -euo pipefail
    command=$1 version=$2 dir=$3 version_of=$4 stderr=$5 workdir=$6 locale=$7
    # What a folder holds: mode, time, name, and its content'"'"'s digest.
    listing() (
      # head closes its pipes early: no pipefail here.
      set +o pipefail
      find . -mindepth 1 | LC_ALL=C sort | while IFS= read -r f; do
        if [ -d "$f" ]; then echo "dir $f"; continue; fi
        if [ "$(head -c 2 "$f" | od -An -tx1 | tr -d " \n")" = 1f8b ]; then
          h=$(head -c 10 "$f" | od -An -tx1 | tr -d " \n")
          if [ $(( 0x$(head -c 4 "$f" | tail -c 1 | od -An -tx1 | tr -d " ") & 8 )) -ne 0 ]; then
            h="$h:$(tail -c +11 "$f" | head -c 1024 | tr "\0" "\n" | head -n 1)"
          fi
          t=$(tail -c 8 "$f" | od -An -tx1 | tr -d " \n")
          st=0; gzip -dc < "$f" > /tmp/content 2> /dev/null || st=$?
          if [ "$st" -eq 1 ]; then d=bad; else d=$(sha256sum < /tmp/content | cut -c1-64); fi
          d="gz:$h:$t:$d"
        else
          d=$(sha256sum < "$f" | cut -c1-64)
        fi
        t=$(stat -c %Y "$f")
        if [ "$t" -ge "$started" ]; then t=now; fi
        echo "$(stat -c %a "$f") $t $f $d"
      done
    )
    got=$(${version_of:-"$command" --version} | sed -n 1p)
    [ "$got" = "$version" ] || { echo "$command is \"$got\", not \"$version\"" >&2; exit 1; }
    for case in "$dir"/*/; do
      case=$(cd "$case" && pwd)
      mapfile -t args < "$case/args"
      args=("${args[@]/#@stdin/$case/stdin}")
      if [ -n "$workdir" ]; then
        work=$(mktemp -d)
        if [ -d "$case/files" ]; then cp -r "$case/files/." "$work/"; fi
        find "$work" -mindepth 1 -type f -exec chmod 644 {} +
        find "$work" -mindepth 1 -type d -exec chmod 755 {} +
        find "$work" -mindepth 1 -exec touch -h -d @1700000000 {} +
        envs=(TZ=UTC)
        if [ -n "$locale" ]; then envs+=("LC_ALL=$locale"); fi
        if [ -f "$case/env" ]; then mapfile -t more < "$case/env"; envs+=("${more[@]}"); fi
        cd "$work"
        umask 022
        started=$(date +%s)
        set +e
        cat "$case/stdin" | env "${envs[@]}" "$command" "${args[@]}" > "$case/stdout" 2> "$case/stderr"
        # The status of the command, not of cat (which a command not reading
        # its input ends with SIGPIPE).
        echo "${PIPESTATUS[1]}" > "$case/exit"
        set -e
        listing > "$case/after"
        cd /w
        continue
      fi
      cd "$case/files" 2> /dev/null || cd "$case"
      set +e
      if [ -n "$stderr" ]; then
        "$command" "${args[@]}" < "$case/stdin" > "$case/stdout" 2> "$case/stderr"
      else
        "$command" "${args[@]}" < "$case/stdin" > "$case/stdout" 2> /dev/null
      fi
      echo $? > "$case/exit"
      set -e
      cd /w
    done
  ' _ "${COMMAND[$group]}" "${VERSION[$group]}" "crates/cli/tests/compat/$group" "${VERSION_OF[$group]:-}" "${STDERR[$group]:-}" "${WORKDIR[$group]:-}" "${LOCALE[$group]:-}"
  echo "$group: recorded $(ls -d crates/cli/tests/compat/"$group"/*/ | wc -l) cases"
done
