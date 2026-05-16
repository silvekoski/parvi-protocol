#!/usr/bin/env bash
# Apply capabilities so the binary can open raw/monitor sockets without running as root.
# Run after every `cargo build --release`.
set -euo pipefail

BINS=("${@:-./target/release/tacticalmesh-bin ./target/release/link-test}")

# If no args given, default to both release binaries
if [[ $# -eq 0 ]]; then
    BINS=(./target/release/tacticalmesh-bin ./target/release/link-test)
fi

for BIN in "${BINS[@]}"; do
    if [[ ! -f "$BIN" ]]; then
        echo "warning: binary not found at $BIN, skipping" >&2
        continue
    fi
    sudo setcap cap_net_raw,cap_net_admin=eip "$BIN"
    echo "setcap applied to $BIN"
done
