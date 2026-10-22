# Standard tasks. `just` lists them.

# Scratch Library, thumbnail cache and settings for manual testing, kept apart
# from the real ones.
scratch := justfile_directory() / "target/scratch"

# Where `just install` puts the app (binary in bin/, launcher in share/).
prefix := env("PREFIX", env("HOME") / ".local")
desktop := "io.github.rxvt.Armarium.desktop"

# Sample books to import for manual testing (kept outside the repo).
books := env("ARMARIUM_SAMPLE_BOOKS", "")

[private]
default:
    @just --list

# Build the whole workspace
build:
    cargo build --workspace --all-targets

# Run a development build (its own settings, cache and Library under armarium-devel)
[positional-arguments]
run *args:
    cargo run -p armarium -- "$@"

# Build a release and install it for this user, with an app launcher entry
install:
    cargo build --release -p armarium
    install -Dm755 target/release/armarium {{ quote(prefix / "bin/armarium") }}
    sed 's|^Exec=armarium$|Exec={{ prefix }}/bin/armarium|' app/data/{{ desktop }} \
        | install -Dm644 /dev/stdin {{ quote(prefix / "share/applications" / desktop) }}
    @echo "Installed {{ prefix }}/bin/armarium"

# Remove what `just install` added (the Library and settings are kept)
uninstall:
    rm -f {{ quote(prefix / "bin/armarium") }} {{ quote(prefix / "share/applications" / desktop) }}

# Run every automated test: Rust crates and the cooldown script
test:
    cargo test --workspace
    python3 -I scripts/test_cooldown_check.py

# Format all code
fmt:
    cargo fmt --all

# Fail if any code is unformatted
fmt-check:
    cargo fmt --all --check

# Lint with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Licences, sources and advisories (needs network)
deny:
    cargo deny check

# RustSec advisories (needs network)
audit:
    cargo audit

# Check Cargo.lock against the 7-day cooldown (needs network)
cooldown:
    python3 -I scripts/cooldown_check.py

# Everything CI would run, slowest last
check: fmt-check clippy test cooldown deny audit

# Enable the repo's git hooks (once per clone)
hooks:
    git config core.hooksPath .githooks

# Import files or folders into a Library from the command line
[positional-arguments]
import library +paths:
    cargo run -p armarium-core --example import -- "$@"

# Convert an EPUB to AZW3 with its own Metadata and Cover, to try on a Kindle
azw3 epub out:
    cargo run --release -p armarium-core --example azw3 -- {{ quote(epub) }} {{ quote(out) }}

# Manual testing: fresh scratch Library, import sample books, open the app
try books_dir=books: reset-scratch
    #!/usr/bin/env sh
    set -eu
    if [ -n {{ quote(books_dir) }} ]; then
        just import {{ quote(scratch / "library") }} {{ quote(books_dir) }}
    else
        echo "No sample books: pass a folder or set ARMARIUM_SAMPLE_BOOKS" >&2
    fi
    just scratch

# Open the app on the existing scratch Library
[positional-arguments]
scratch *args:
    ARMARIUM_LIBRARY={{ quote(scratch / "library") }} XDG_CACHE_HOME={{ quote(scratch / "cache") }} \
        ARMARIUM_SETTINGS={{ quote(scratch / "settings.ini") }} \
        cargo run -p armarium -- "$@"

# Open the app on the scratch Library with a scratch Kindle (a plain folder) connected
[positional-arguments]
scratch-kindle *args:
    mkdir -p {{ quote(scratch / "kindle/documents") }} {{ quote(scratch / "kindle/system") }}
    ARMARIUM_DEVICE={{ quote(scratch / "kindle") }} just scratch "$@"

# Render the scratch Library's window to a PNG and quit
screenshot out=(scratch / "screenshot.png"):
    ARMARIUM_SCREENSHOT={{ quote(out) }} just scratch
    @echo "Saved {{ out }}"

# Delete the scratch Library and its cache
reset-scratch:
    rm -rf {{ quote(scratch) }}
    mkdir -p {{ quote(scratch / "library") }}
