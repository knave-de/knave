#!/usr/bin/env bash
set -euo pipefail

prefix="$HOME/.local"
mode="user"

while (($#)); do
    case "$1" in
        --user)
            prefix="$HOME/.local"
            mode="user"
            ;;
        --system)
            prefix="/usr/local"
            mode="system"
            ;;
        --prefix)
            shift
            prefix="$1"
            mode="custom"
            ;;
        *)
            printf 'usage: %s [--user|--system|--prefix PATH]\n' "$0" >&2
            exit 2
            ;;
    esac
    shift
done

if [[ "$mode" == system ]] && [[ "$(id -u)" -ne 0 ]]; then
    exec sudo -- "$0" --system
fi

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cargo build --release --locked --manifest-path "$root/Cargo.toml"
install -Dm755 "$root/target/release/knave" "$prefix/bin/knave"
install -Dm755 "$root/target/release/knavectl" "$prefix/bin/knavectl"
install -Dm755 "$root/target/release/knave-session" "$prefix/bin/knave-session"
install -Dm644 "$root/README.md" "$prefix/share/doc/knave/README.md"
printf 'installed Knave binaries to %s/bin\n' "$prefix"
