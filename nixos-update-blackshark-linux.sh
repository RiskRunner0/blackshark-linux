#!/usr/bin/env bash
set -euo pipefail

# ============================================================================
# BlackShark Linux Nix Package Updater
# ============================================================================
#
# Finds the latest stable release from GitHub and updates:
#
#   version = "..."
#   hash = "..."
#
# By default, the script recursively searches /etc/nixos for:
#
#   app-blackshark-linux.nix
#
# An explicit file path can also be supplied.
#
# Examples:
#
#   ./nixos-update-blackshark-linux.sh
#   ./nixos-update-blackshark-linux.sh --dry-run
#   ./nixos-update-blackshark-linux.sh /etc/nixos/custom-apps/app-blackshark-linux.nix
#
# ============================================================================

REPO_URL="https://github.com/RiskRunner0/blackshark-linux"
FILE_NAME="app-blackshark-linux.nix"

DRY_RUN=false
FILE=""

# ============================================================================
# Usage
# ============================================================================

usage() {
    cat <<EOF
Usage: $0 [OPTIONS] [FILE]

Update the BlackShark Linux Nix package to the latest release.

Options:
  --dry-run    Show changes without modifying the Nix file
  --help, -h   Show this help message

Arguments:
  FILE         Optional path to app-blackshark-linux.nix

If FILE is not provided, the script recursively searches:
  /etc/nixos

Examples:
    $0
    $0 --dry-run
    $0 /etc/nixos/custom-apps/app-blackshark-linux.nix
EOF
}

# ============================================================================
# Argument parsing
# ============================================================================

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=true
            shift
            ;;

        --help|-h)
            usage
            exit 0
            ;;

        -*)
            echo "Error: Unknown option: $1" >&2
            echo
            usage
            exit 1
            ;;

        *)
            if [[ -n "$FILE" ]]; then
                echo "Error: Multiple file paths were provided." >&2
                exit 1
            fi

            FILE="$1"
            shift
            ;;
    esac
done

# ============================================================================
# Find the package file
# ============================================================================

if [[ -z "$FILE" ]]; then
    echo "-> Searching /etc/nixos recursively for $FILE_NAME..."

    mapfile -t MATCHES < <(
        find -L /etc/nixos \
            -type f \
            -name "$FILE_NAME" \
            -print 2>/dev/null
    )

    case "${#MATCHES[@]}" in
        0)
            echo
            echo "Error: Could not find $FILE_NAME under /etc/nixos." >&2
            echo
            echo "You can specify the file manually:" >&2
            echo "  $0 /path/to/app-blackshark-linux.nix" >&2
            exit 1
            ;;

        1)
            FILE="${MATCHES[0]}"
            ;;

        *)
            echo
            echo "Error: Found multiple $FILE_NAME files:" >&2
            echo

            printf '  %s\n' "${MATCHES[@]}" >&2

            echo
            echo "Please specify which file to update:" >&2
            echo "  $0 /path/to/app-blackshark-linux.nix" >&2
            exit 1
            ;;
    esac
fi

if [[ ! -f "$FILE" ]]; then
    echo "Error: File not found:" >&2
    echo "  $FILE" >&2
    exit 1
fi

echo "-> Package file:"
echo "   $FILE"

# ============================================================================
# Check required commands
# ============================================================================

REQUIRED_COMMANDS=(
    git
    nix
    nix-prefetch-url
    awk
    diff
    mktemp
    cp
)

for command in "${REQUIRED_COMMANDS[@]}"; do
    if ! command -v "$command" >/dev/null 2>&1; then
        echo "Error: Required command not found: $command" >&2
        exit 1
    fi
done

# ============================================================================
# Find latest release
# ============================================================================

echo
echo "-> Fetching latest release..."
echo "   Repository: $REPO_URL"

LATEST_TAG="$(
    git ls-remote --tags --refs "$REPO_URL" |
        awk -F/ '
            $NF ~ /^v[0-9]+\.[0-9]+\.[0-9]+([.-].*)?$/ {
                print $NF
            }
        ' |
        sort -V |
        tail -n 1
)"

if [[ -z "$LATEST_TAG" ]]; then
    echo "Error: Could not determine the latest release." >&2
    exit 1
fi

VERSION="${LATEST_TAG#v}"

echo "-> Latest release:"
echo "   Version: $VERSION"

# ============================================================================
# Calculate source hash
# ============================================================================

TARBALL_URL="$REPO_URL/archive/refs/tags/$LATEST_TAG.tar.gz"

echo
echo "-> Calculating source hash..."
echo "   This may take a moment."

BASE32_HASH="$(
    nix-prefetch-url \
        --type sha256 \
        --unpack \
        "$TARBALL_URL" |
        tail -n 1
)"

if [[ -z "$BASE32_HASH" ]]; then
    echo "Error: Could not calculate the source hash." >&2
    exit 1
fi

SRI_HASH="$(
    nix hash convert \
        --hash-algo sha256 \
        --to sri \
        "$BASE32_HASH"
)"

if [[ -z "$SRI_HASH" ]]; then
    echo "Error: Could not convert the source hash." >&2
    exit 1
fi

echo "-> Source hash:"
echo "   $SRI_HASH"

# ============================================================================
# Read current package values
# ============================================================================

CURRENT_VERSION="$(
    awk '
        /^[[:space:]]*version[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }
    ' "$FILE"
)"

CURRENT_HASH="$(
    awk '
        /pkgs\.fetchFromGitHub[[:space:]]*\{/ {
            in_source = 1
        }

        in_source && /^[[:space:]]*hash[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*hash[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }

        in_source && /^[[:space:]]*};[[:space:]]*$/ {
            in_source = 0
        }
    ' "$FILE"
)"

echo
echo "-> Current package:"
echo "   Version: ${CURRENT_VERSION:-<not found>}"
echo "   Hash:    ${CURRENT_HASH:-<not found>}"

# ============================================================================
# Validate package structure
# ============================================================================

if [[ -z "$CURRENT_VERSION" ]]; then
    echo "Error: Could not find the package version in $FILE" >&2
    exit 1
fi

if [[ -z "$CURRENT_HASH" ]]; then
    echo "Error: Could not find the source hash in $FILE" >&2
    exit 1
fi

# ============================================================================
# Create updated temporary file
# ============================================================================

TMP_FILE="$(mktemp)"

cleanup() {
    rm -f "$TMP_FILE"
}

trap cleanup EXIT

awk \
    -v new_version="$VERSION" \
    -v new_hash="$SRI_HASH" '
BEGIN {
    in_source = 0
    found_version = 0
    found_source = 0
    found_hash = 0
}

{
    line = $0

    # ------------------------------------------------------------------------
    # Update package version
    # ------------------------------------------------------------------------

    if (!found_version &&
        line ~ /^[[:space:]]*version[[:space:]]*=/) {

        match(line, /^[[:space:]]*/)
        indent = substr(line, RSTART, RLENGTH)

        print indent "version = \"" new_version "\";"
        found_version = 1
        next
    }

    # ------------------------------------------------------------------------
    # Enter source block
    # ------------------------------------------------------------------------

    if (line ~ /pkgs\.fetchFromGitHub[[:space:]]*\{/) {
        in_source = 1
        found_source = 1
    }

    # ------------------------------------------------------------------------
    # Update source hash
    # ------------------------------------------------------------------------

    if (in_source &&
        line ~ /^[[:space:]]*hash[[:space:]]*=/) {

        match(line, /^[[:space:]]*/)
        indent = substr(line, RSTART, RLENGTH)

        print indent "hash = \"" new_hash "\";"
        found_hash = 1
        next
    }

    # ------------------------------------------------------------------------
    # Leave source block
    # ------------------------------------------------------------------------

    if (in_source &&
        line ~ /^[[:space:]]*};[[:space:]]*$/) {
        in_source = 0
    }

    print line
}

END {
    if (!found_version)
        exit 2

    if (!found_source)
        exit 3

    if (!found_hash)
        exit 4
}
' "$FILE" > "$TMP_FILE"

# ============================================================================
# Validate generated file
# ============================================================================

UPDATED_VERSION="$(
    awk '
        /^[[:space:]]*version[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }
    ' "$TMP_FILE"
)"

UPDATED_HASH="$(
    awk '
        /pkgs\.fetchFromGitHub[[:space:]]*\{/ {
            in_source = 1
        }

        in_source && /^[[:space:]]*hash[[:space:]]*=/ {
            line = $0
            sub(/^[[:space:]]*hash[[:space:]]*=[[:space:]]*"/, "", line)
            sub(/".*$/, "", line)
            print line
            exit
        }

        in_source && /^[[:space:]]*};[[:space:]]*$/ {
            in_source = 0
        }
    ' "$TMP_FILE"
)"

if [[ "$UPDATED_VERSION" != "$VERSION" ]]; then
    echo "Error: Failed to update the package version." >&2
    echo "Expected: $VERSION" >&2
    echo "Found:    ${UPDATED_VERSION:-<missing>}" >&2
    exit 1
fi

if [[ "$UPDATED_HASH" != "$SRI_HASH" ]]; then
    echo "Error: Failed to update the source hash." >&2
    echo "Expected: $SRI_HASH" >&2
    echo "Found:    ${UPDATED_HASH:-<missing>}" >&2
    exit 1
fi

# ============================================================================
# Show changes
# ============================================================================

echo
echo "==================== DIFF ===================="

if diff -u "$FILE" "$TMP_FILE"; then
    echo
    echo "No changes required."
    echo "The package is already up to date."
    exit 0
fi

echo "==============================================="

# ============================================================================
# Dry run
# ============================================================================

if [[ "$DRY_RUN" == true ]]; then
    echo
    echo "DRY RUN: no files were modified."
    exit 0
fi

# ============================================================================
# Create backup
# ============================================================================

BACKUP="${FILE}.bak"

cp "$FILE" "$BACKUP"

echo
echo "-> Backup created:"
echo "   $BACKUP"

# ============================================================================
# Apply update
# ============================================================================

cp "$TMP_FILE" "$FILE"

echo
echo "Successfully updated:"
echo "  $FILE"
echo
echo "Release:"
echo "  $VERSION"
echo
echo "Source hash:"
echo "  $SRI_HASH"
echo
echo "Backup:"
echo "  $BACKUP"
echo
echo "You can now rebuild with:"
echo
echo "  sudo nixos-rebuild switch"
