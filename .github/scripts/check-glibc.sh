#!/usr/bin/env bash
set -euo pipefail

usage="usage: check-glibc.sh <binary> <highest allowed glibc, e.g. 2.35>"
binary=${1:?$usage}
limit=${2:?$usage}

needed=$(objdump -T "$binary" | grep -o 'GLIBC_[0-9][0-9.]*' | sort -uV | tail -n 1 || true)
needed=${needed#GLIBC_}
if [[ -z $needed ]]; then
    echo "::error::no versioned glibc symbols found in $binary" >&2
    exit 1
fi

highest=$(printf '%s\n%s\n' "$needed" "$limit" | sort -V | tail -n 1)
if [[ $highest != "$limit" ]]; then
    echo "::error::$binary needs glibc $needed, newer than the supported $limit" >&2
    exit 1
fi
echo "$binary needs glibc $needed (limit $limit)"
