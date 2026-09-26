#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/install.sh [--user|--system|--prefix PATH]

Builds and installs Knave's CLI, public desktop control client, and session
supervisor. --user installs to ~/.local, --system installs to /usr/local using
sudo when needed, and --prefix selects an explicit staging prefix.
EOF
}

prefix="${HOME}/.local"
while (($# > 0)); do
    case "$1" in
        --user)
            prefix="${HOME}/.local"
            ;;
        --system)
            prefix="/usr/local"
            ;;
        --prefix)
            if (($# < 2)); then
                echo "--prefix requires a path" >&2
                exit 2
            fi
            prefix="$2"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
cargo build --workspace --release --locked --manifest-path "$root/Cargo.toml"

install_cmd=(install)
if [[ "$prefix" == /usr/* ]] && [[ "$(id -u)" -ne 0 ]]; then
    install_cmd=(sudo install)
fi

"${install_cmd[@]}" -Dm755 "$root/target/release/knave" "$prefix/bin/knave"
"${install_cmd[@]}" -Dm755 "$root/target/release/knavectl" "$prefix/bin/knavectl"
"${install_cmd[@]}" -Dm755 "$root/target/release/knave-session" "$prefix/bin/knave-session"
"${install_cmd[@]}" -Dm644 "$root/README.md" "$prefix/share/doc/knave/README.md"
printf 'installed Knave binaries to %s/bin\n' "$prefix"
