#!/usr/bin/env bash

set -euo pipefail

BINARY="$1"
DEST="$2"

# Create target directories
mkdir -p "$DEST/bin"
mkdir -p "$DEST/lib"

# Copy the binary itself
cp -v "$BINARY" "$DEST/bin/"

# Use ldd to find shared libraries
ldd "$BINARY" | awk '{
    for(i=1;i<=NF;i++){
        if($i ~ /^\//){ print $i }
    }
}' | sort -u | while read -r lib; do
    if [[ -f "$lib" ]]; then
        cp -v -n "$lib" "$DEST/lib/"
    fi
done
