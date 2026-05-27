#!/bin/bash

# Combined lint script
# Runs standard clippy (strict) + baseline clippy rules

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Source the baseline functions
source "$SCRIPT_DIR/clippy-baseline.sh"

echo "🔍 Running all clippy checks..."

FIX_MODE=0
[[ "$1" == "--fix" ]] && FIX_MODE=1

run_clippy() {
  if [[ "$FIX_MODE" -eq 1 ]]; then
    cargo fmt
    cargo clippy --all-targets --jobs 2 \
      --fix --allow-dirty --allow-staged \
      -- -D warnings $BASELINE_ALLOWS
  else
    cargo clippy --all-targets --jobs 2 -- -D warnings $BASELINE_ALLOWS
  fi
}

# Baseline allow-list: lints that have many pre-existing violations on main
# and are tracked separately rather than blocking every PR. New PRs should
# avoid introducing additional violations of these lints; they will be
# tightened back to deny once the baseline is cleaned up.
BASELINE_ALLOWS="\
  -A clippy::clone_on_copy \
  -A clippy::cloned_ref_to_slice_refs \
  -A clippy::collapsible_if \
  -A clippy::collapsible_str_replace \
  -A clippy::derivable_impls \
  -A clippy::enum_variant_names \
  -A clippy::filter_next \
  -A clippy::if_same_then_else \
  -A clippy::items_after_test_module \
  -A clippy::large_enum_variant \
  -A clippy::manual_contains \
  -A clippy::mut_range_bound \
  -A clippy::needless_borrow \
  -A clippy::needless_lifetimes \
  -A clippy::needless_match \
  -A clippy::needless_option_as_deref \
  -A clippy::needless_range_loop \
  -A clippy::nonminimal_bool \
  -A clippy::ptr_arg \
  -A clippy::question_mark \
  -A clippy::redundant_closure \
  -A clippy::result_large_err \
  -A clippy::too_many_arguments \
  -A clippy::type_complexity \
  -A clippy::unnecessary_lazy_evaluations \
  -A clippy::unnecessary_sort_by \
  -A clippy::useless_conversion \
  -A clippy::useless_format \
  -A clippy::useless_vec"

if [[ "$FIX_MODE" -eq 1 ]]; then
  echo "🛠  Applying fixes..."
else
  echo "🔍 Running clippy..."
fi

run_clippy
echo ""
check_all_baseline_rules
echo ""
echo "🔒 Checking for banned TLS crates..."
"$SCRIPT_DIR/check-no-native-tls.sh"
echo ""
echo "✅ Done"
