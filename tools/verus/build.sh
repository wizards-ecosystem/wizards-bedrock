#!/bin/bash
# build.sh DIR: build Z3 and Verus at the commits VERSION pins, under DIR.
# Afterwards put DIR/verus-src/source/target-verus/release on PATH for `cargo xtask verify`.
set -eux
here=$(cd "$(dirname "$0")" && pwd)
root=$1
pin() { sed -n "s/^$1 //p" "$here/VERSION"; }
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-3}
mkdir -p "$root"
cd "$root"

fetch() {
    if [ ! -d "$1/.git" ]; then
        git init -q "$1"
        git -C "$1" remote add origin "$2"
    fi
    git -C "$1" fetch -q --depth 1 origin "$3"
    git -C "$1" checkout -q --detach "$3"
    test "$(git -C "$1" rev-parse HEAD)" = "$3"
}

fetch z3-src https://github.com/Z3Prover/z3 "$(pin z3-commit)"
if [ ! -x z3-src/build/z3 ]; then
    rm -rf z3-src/build compat
    # g++ 12 ships no <format>; the shim stands in for it, and the Z3 tree stays as pinned.
    mkdir compat
    cp "$here/format" compat/format
    (cd z3-src && CXXFLAGS="-I$root/compat" python3 scripts/mk_make.py && cd build && make -j"$CARGO_BUILD_JOBS")
fi
z3-src/build/z3 --version | grep -q "Z3 version $(pin z3) "

fetch verus-src https://github.com/verus-lang/verus "$(pin commit)"
cp z3-src/build/z3 verus-src/source/z3
cd verus-src/source
rustup toolchain install
bash -c 'source ../tools/activate && vargo build --release'
./target-verus/release/verus --version | grep -q "Version: $(pin verus)$"
cd target-verus/release
sha256sum verus cargo-verus rust_verify z3 libvstd.rlib vstd.vir > "$root/SHA256SUMS"
