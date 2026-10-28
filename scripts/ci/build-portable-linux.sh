#!/usr/bin/env bash
# Build the release `forge` binary for a Linux target inside the pinned Debian Bullseye image.
#
# Bullseye ships glibc 2.31, which is the floor every released Linux binary keeps (see
# scripts/check-linux-runtime-deps.sh). The build is NATIVE: the x86_64 leg runs on an x64 runner
# and the aarch64 leg on GitHub's free ubuntu-24.04-arm runner, both pulling the matching platform
# of the same pinned image index, so no cross compiler, sysroot or qemu is involved.
#
#   scripts/ci/build-portable-linux.sh <rust-target>
#
# Output: target/<rust-target>/release/forge, owned by the invoking user so rust-cache can save it.
set -euo pipefail

TARGET="${1:?usage: build-portable-linux.sh <rust-target>}"
case "$TARGET" in
  x86_64-unknown-linux-gnu) want_arch=x86_64 ;;
  aarch64-unknown-linux-gnu) want_arch=aarch64 ;;
  *) echo "unsupported target: $TARGET" >&2; exit 2 ;;
esac
[[ "$(uname -m)" == "$want_arch" ]] || {
  echo "$TARGET must be built natively on $want_arch (this host is $(uname -m))" >&2
  exit 2
}

image='rust:1.88.0-bullseye@sha256:b315f988b86912bafa7afd39a6ded0a497bf850ec36578ca9a3bdd6a14d5db4e'
workspace="$(pwd)"
mkdir -p "$workspace/target" "$HOME/.cargo/registry" "$HOME/.cargo/git"

docker pull -q "$image" >/dev/null

docker run --rm \
  --pids-limit 4096 \
  --env CARGO_TARGET_DIR=/out \
  --env CARGO_BUILD_JOBS \
  --env FORGE_DISTRIBUTION \
  --env FORGE_POSTHOG_HOST \
  --env FORGE_POSTHOG_KEY \
  --env TARGET="$TARGET" \
  --env HOST_UID="$(id -u)" \
  --env HOST_GID="$(id -g)" \
  --volume "$workspace:/workspace" \
  --volume "$workspace/target:/out" \
  --volume "$HOME/.cargo/registry:/usr/local/cargo/registry" \
  --volume "$HOME/.cargo/git:/usr/local/cargo/git" \
  --workdir /workspace \
  "$image" \
  bash -euxo pipefail -c '
    # Hand every file the container created back to the runner user, even when the build fails:
    # rust-cache runs as that user and cannot clean or save root-owned entries.
    trap "chown -R $HOST_UID:$HOST_GID /out /usr/local/cargo/registry /usr/local/cargo/git" EXIT

    # Bullseye is EOL and its security pool rotated out from under the live mirror: the index
    # advertises libc-dev-bin 2.31-13+deb11u14 while deb.debian.org has already deleted that .deb,
    # so apt 404s and the release build dies 24 seconds in, before a line of Rust is compiled. It
    # broke both Linux targets of v2.14.0. archive.debian.org is not the answer yet either:
    # mid-transition it has no bullseye-security Release file at all.
    #
    # snapshot.debian.org is: a frozen, timestamped view that keeps the security suite and cannot
    # rotate underneath a build. Pinning it makes this step reproducible the same way the image
    # digest and toolchain already are. Move the timestamp forward deliberately when the build
    # needs newer packages, never automatically.
    snapshot=20260901T000000Z
    base=http://snapshot.debian.org/archive
    printf "deb %s/debian/%s bullseye main\n" "$base" "$snapshot" > /etc/apt/sources.list
    printf "deb %s/debian-security/%s bullseye-security main\n" "$base" "$snapshot" >> /etc/apt/sources.list
    printf "deb %s/debian/%s bullseye-updates main\n" "$base" "$snapshot" >> /etc/apt/sources.list
    # A snapshot Release file is deliberately older than apt tolerates by default.
    printf "Acquire::Check-Valid-Until \"false\";\n" > /etc/apt/apt.conf.d/99forge-snapshot
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends cmake libclang-dev

    cargo build --release --locked --bin forge --target "$TARGET"
    "/out/$TARGET/release/forge" --version
  '

chmod 0755 "$workspace/target/$TARGET/release/forge"
