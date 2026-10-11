#!/bin/sh
# Build the app packages Home Assistant installs on a Nest: one gzipped tar per
# app, holding exactly `manifest.json` and `app`, at apps/dist/<id>/nest-app.tar.gz.
#
#   apps/build.sh [--host | --target TRIPLE] [APP ...]
#
# With no APP, every app in this workspace is packaged (any directory here with a
# manifest.json). The app's cargo package is `nest-app-<id>` and so is its binary,
# which is copied into the package as `app`.
#
# --host (or BUILD_HOST=1) builds for this machine instead of the Nest, for
# testing the app against a fake Nest: see README.md. A host build is not a
# package to publish — it is not static and not ARM.
#
# --target TRIPLE (or BUILD_TARGET=TRIPLE) packages for another target instead of
# the Nest: the integration suite builds x86_64-unknown-linux-musl packages for a
# Nest app running in a container. The triple must be installed (`rustup target
# add`), and it must link statically (a musl target does): an app runs chrooted,
# where a glibc-linked binary would find no libc.
#
# The Nest target is armv7-unknown-linux-musleabihf, statically linked with the
# bundled rust-lld, exactly as nest-app builds it.
set -eu
cd "$(dirname "$0")" || exit 1

TARGET=armv7-unknown-linux-musleabihf
if [ -n "${BUILD_TARGET:-}" ]; then
    TARGET=$BUILD_TARGET
fi
MAX_APP=4194304
MAX_PACKAGE=8388608

usage() {
    cat <<'EOF'
Build the app packages Home Assistant installs on a Nest.

    apps/build.sh [--host | --target TRIPLE] [APP ...]

  --host          build for this machine instead of the Nest (BUILD_HOST=1 also works)
  --target TRIPLE package for another target's triple (BUILD_TARGET=TRIPLE also
                  works), e.g. x86_64-unknown-linux-musl for the integration suite
  APP             an app to package, e.g. demo or stm; default: every app here

Each package is written to apps/dist/<id>/nest-app.tar.gz, and holds exactly
manifest.json and app, at the root of the archive.
EOF
}

HOST=
APPS=
# The triple after a split `--target <triple>`.
NEXT_TARGET=
for arg in "$@"; do
    if [ -n "$NEXT_TARGET" ]; then
        TARGET=$arg
        NEXT_TARGET=
        continue
    fi
    case $arg in
        --host) HOST=1 ;;
        --target) NEXT_TARGET=1 ;;
        --target=*) TARGET=$arg ;;
        -h | --help)
            usage
            exit 0
            ;;
        -*)
            echo "build.sh: unknown option $arg" >&2
            usage >&2
            exit 2
            ;;
        *) APPS="$APPS $arg" ;;
    esac
done
if [ -n "$NEXT_TARGET" ]; then
    echo "build.sh: --target needs a triple" >&2
    usage >&2
    exit 2
fi
# --target=x, or the environment: the split form already set TARGET.
case $TARGET in
    --target=*) TARGET=${TARGET#--target=} ;;
esac
if [ -z "$TARGET" ]; then
    echo "build.sh: empty target" >&2
    exit 2
fi
if [ -n "${BUILD_HOST:-}" ]; then
    HOST=1
fi

# No app named: package every one in this workspace that has a manifest.
if [ -z "$APPS" ]; then
    for manifest in */manifest.json; do
        [ -f "$manifest" ] || continue
        APPS="$APPS ${manifest%/manifest.json}"
    done
fi

if [ -z "$APPS" ]; then
    echo "build.sh: no apps to build" >&2
    exit 2
fi

for id in $APPS; do
    case $id in
        */* | . | ..)
            echo "build.sh: $id is not an app id" >&2
            exit 2
            ;;
    esac
    package="nest-app-$id"
    if [ ! -f "$id/Cargo.toml" ] || [ ! -f "$id/manifest.json" ]; then
        echo "build.sh: no $id/Cargo.toml and $id/manifest.json: is $id an app?" >&2
        exit 2
    fi

    if [ -n "$HOST" ]; then
        cargo build --release -p "$package"
        binary="target/release/$package"
    else
        cargo build --release -p "$package" --target "$TARGET"
        binary="target/$TARGET/release/$package"
    fi

    stage=$(mktemp -d)
    trap 'rm -rf "$stage"' EXIT
    cp "$binary" "$stage/app"
    cp "$id/manifest.json" "$stage/manifest.json"
    # The package's `app` is a program: the Nest unpacks it and runs it.
    chmod 755 "$stage/app"

    app_bytes=$(wc -c < "$stage/app")
    if [ "$app_bytes" -gt "$MAX_APP" ]; then
        echo "build.sh: $id: app is $app_bytes bytes, over the 4 MiB a package may carry" >&2
        exit 1
    fi

    out="dist/$id"
    mkdir -p "$out"
    rm -f "$out/nest-app.tar.gz"
    # Exactly two entries, at the archive root: anything else is `bad_tar`.
    tar -czf "$out/nest-app.tar.gz" -C "$stage" manifest.json app
    rm -rf "$stage"

    package_bytes=$(wc -c < "$out/nest-app.tar.gz")
    if [ "$package_bytes" -gt "$MAX_PACKAGE" ]; then
        echo "build.sh: $id: the package is $package_bytes bytes, over the 8 MiB limit" >&2
        exit 1
    fi
    echo "$id: $out/nest-app.tar.gz ($package_bytes bytes, app $app_bytes bytes)"
done
