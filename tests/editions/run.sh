#!/usr/bin/env bash
# Build and run every case against `refield`, as Rust 2021 and Rust 2024
# crates. Case names carry the exploration dimensions they check (D6, D7, ...;
# see docs/). Rust 2021 callers are also covered by `cargo test`; Rust 2024
# callers and the lint and span diagnostics only here.
set -u
cd "$(dirname "$0")"
cases=(
  d06_d14_source_borrows
  d07_prepare_tle
  d15_d16_d51_tle
  d17_d24_order
  d25_d34_types_and_moves
  d35_d37_flow_const_async
  d44_d52_lints
  d49_diagnostics
  post_value
  source_diagnostics
)
# `cfg case expected`: each must fail to compile with a diagnostic containing
# every ` && `-separated part of `expected` (spans as `file:line:column`).
negatives=(
  "d44 d44_d52_lints unused variable: \`unused\`"
  "d45 d44_d52_lints this lint expectation is unfulfilled"
  "d48 d44_d52_lints use of deprecated field \`OldSourceField::a\`"
  "d51 d44_d52_lints use of deprecated function \`old_raw\`"
  "d52 d44_d52_lints use of deprecated function \`old_raw\`"
  "fields post_value a value source cannot follow written fields"
  "misplaced post_value \`source..\` must be the first entry in the body"
  "d49_unknown d49_diagnostics error[E0560] && d49_diagnostics.rs:21:38"
  "d49_duplicate d49_diagnostics error[E0062] && d49_diagnostics.rs:23:44"
  "d49_type d49_diagnostics error[E0308] && d49_diagnostics.rs:25:41"
  "d49_attr d49_diagnostics error[E0805] && d49_diagnostics.rs:33:11"
  "missing source_diagnostics error[E0609] && source_diagnostics.rs:74:39"
  "mismatch source_diagnostics error[E0308] && source_diagnostics.rs:76:31"
  "behind source_diagnostics error[E0507] && source_diagnostics.rs:80:29"
  "primitive source_diagnostics error[E0610] && source_diagnostics.rs:83:31"
  "deprecated_place source_diagnostics use of deprecated static \`OLD\` && source_diagnostics.rs:45:25"
  "unused_closure source_diagnostics unused variable: \`x\` && source_diagnostics.rs:54:27"
)
status=0
for ed in 2021 2024; do
  for case in "${cases[@]}"; do
    if out=$(cargo run --offline -q -p "refield-editions-e$ed" --bin "${case}_e$ed" 2>&1); then
      echo "pass  e$ed $case: $out"
    else
      echo "FAIL  e$ed $case"
      echo "$out" | head -30
      status=1
    fi
  done
  for neg in "${negatives[@]}"; do
    read -r cfg case expected <<<"$neg"
    out=$(cargo rustc --offline -q -p "refield-editions-e$ed" --bin "${case}_e$ed" -- --cfg "$cfg" 2>&1)
    code=$?
    found=$([ $code -ne 0 ] && echo yes)
    while IFS= read -r part; do
      grep -qF -- "$part" <<<"$out" || found=
    done <<<"${expected// && /$'\n'}"
    if [ -n "$found" ]; then
      echo "pass  e$ed $case --cfg $cfg: rejected with \"$expected\""
    else
      echo "FAIL  e$ed $case --cfg $cfg: expected a compile error containing \"$expected\""
      echo "$out" | head -30
      status=1
    fi
  done
done
exit $status
