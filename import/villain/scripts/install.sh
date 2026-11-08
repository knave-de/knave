#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: scripts/install.sh [--user|--system|--prefix PATH]

Builds and installs the Villain compositor. knavectl is installed by the
Knave repository because it is the desktop environment's public control CLI.
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

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo build --manifest-path "${repo_root}/Cargo.toml" --release --locked

install_cmd=(install)
if [[ "$prefix" == /usr/* ]] && [[ "$(id -u)" -ne 0 ]]; then
    install_cmd=(sudo install)
fi

"${install_cmd[@]}" -Dm755 "${repo_root}/target/release/villain" \
    "${prefix}/bin/villain"
"${install_cmd[@]}" -Dm644 "${repo_root}/README.md" \
    "${prefix}/share/doc/villain/README.md"

printf 'Installed Villain to %s/bin/villain\n' "$prefix"
