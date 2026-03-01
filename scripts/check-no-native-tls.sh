#!/bin/bash
# Prevent native-tls/OpenSSL from being added to the dependency tree.
# These cause Linux compatibility issues with OpenSSL version mismatches.
# See: https://github.com/jsjm1986/AGIME/issues/6034

set -e

BANNED_CRATES=("native-tls" "openssl-sys" "openssl")
FOUND_BANNED=0

is_allowed_openssl_sys_chain() {
    local tree_output="$1"
    local first_level

    # Strip ANSI colors if present.
    local clean_tree
    clean_tree="$(printf '%s\n' "$tree_output" | sed -E 's/\x1B\[[0-9;]*[mK]//g')"

    # Only inspect first-level reverse dependencies of openssl-sys.
    first_level="$(
        printf '%s\n' "$clean_tree" \
            | grep -E '^[├└]── ' \
            | sed -E 's/^[├└]── ([^ ]+).*/\1/' \
            | sort -u
    )"

    if [ -z "$first_level" ]; then
        return 1
    fi

    # Temporary controlled exception:
    # openssl-sys is currently required only through libgit2-sys/git2.
    local disallowed
    disallowed="$(printf '%s\n' "$first_level" | grep -Ev '^(git2|libgit2-sys)$' || true)"
    [ -z "$disallowed" ]
}

for crate in "${BANNED_CRATES[@]}"; do
    tree_output="$(cargo tree -i "$crate" 2>/dev/null || true)"
    if printf '%s\n' "$tree_output" | grep -q "$crate"; then
        if [ "$crate" = "openssl-sys" ] && is_allowed_openssl_sys_chain "$tree_output"; then
            echo "WARN: Found '$crate' only through approved chain (git2/libgit2-sys); allowing for now."
            echo "Dependency chain:"
            printf '%s\n' "$tree_output"
            echo ""
            continue
        fi

        echo "ERROR: Found banned crate '$crate' in dependency tree"
        echo "This causes Linux compatibility issues with OpenSSL versions."
        echo "Use rustls-based alternatives instead (e.g., rustls-tls-native-roots)."
        echo ""
        echo "Dependency chain:"
        printf '%s\n' "$tree_output"
        echo ""
        FOUND_BANNED=1
    fi
done

if [ $FOUND_BANNED -eq 1 ]; then
    exit 1
fi

echo "✓ No banned TLS crates found (native-tls, openssl, openssl-sys)"
