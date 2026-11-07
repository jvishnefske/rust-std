{
  description = "The real Rust std on *-unknown-linux-none (no libc)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; overlays = [ rust-overlay.overlays.default ]; };
      # the nightly pinned in rust-toolchain.toml, with rust-src and rust-lld
      toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      fs = pkgs.lib.fileset;

      # xtask has no dependencies: build it without cargo (and its index)
      xtask = pkgs.stdenv.mkDerivation {
        name = "linux-none-xtask";
        src = fs.toSource { root = ./.; fileset = ./xtask; };
        nativeBuildInputs = [ toolchain ];
        buildPhase = ''
          export CARGO_MANIFEST_DIR=$PWD/xtask
          rustc --edition 2024 --crate-type lib --crate-name xtask -o libxtask.rlib xtask/src/lib.rs
          rustc --edition 2024 --crate-name xtask_bin -o xtask-bin --extern xtask=libxtask.rlib xtask/src/main.rs
        '';
        installPhase = "install -D xtask-bin $out/bin/xtask";
      };

      # rust-src with the libc fixup and std-patches/ applied, rt copied in:
      # what `cargo xtask sysroot` puts in target/linux-none/std-src
      std-src = pkgs.stdenv.mkDerivation {
        name = "linux-none-std-src";
        src = fs.toSource {
          root = ./.;
          fileset = fs.unions [ ./xtask ./std-patches ./rt/Cargo.toml ./rt/build.rs ./rt/src ];
        };
        nativeBuildInputs = [ toolchain pkgs.git ];
        dontFixup = true;
        # xtask finds rt/ and std-patches/ next to the xtask/ it was built from
        buildPhase = ''
          export CARGO_MANIFEST_DIR=$PWD/xtask
          rustc --edition 2024 --crate-type lib --crate-name xtask -o libxtask.rlib xtask/src/lib.rs
          rustc --edition 2024 --crate-name xtask_bin -o xtask-bin --extern xtask=libxtask.rlib xtask/src/main.rs
          ./xtask-bin sysroot --out $PWD/std-src --no-toolchain
          rm -rf std-src/.git
        '';
        installPhase = "mv std-src $out";
      };

      # every crate a build may need, for the network-less sandbox: std's own
      # (rust-src vendors them) and those of the apps/ and root workspaces
      vendor = pkgs.symlinkJoin {
        name = "linux-none-vendor";
        paths = [
          "${std-src}/library/vendor"
          (pkgs.rustPlatform.importCargoLock { lockFile = ./apps/Cargo.lock; })
          (pkgs.rustPlatform.importCargoLock { lockFile = ./Cargo.lock; })
        ];
      };

      # $CARGO_HOME/config.toml for offline builds from `vendor`
      cargo-config = pkgs.writeText "config.toml" ''
        [source.crates-io]
        replace-with = "vendored"

        [source.vendored]
        directory = "${vendor}"

        [net]
        offline = true
      '';

      # cargo in the sandbox: offline from `vendor`, std from `std-src`
      # (apps/stdtests also reads std's tests from target/linux-none/std-src)
      cargoSetup = ''
        export CARGO_HOME=$TMPDIR/cargo-home
        mkdir -p $CARGO_HOME target/linux-none
        cp ${cargo-config} $CARGO_HOME/config.toml
        ln -s ${std-src} target/linux-none/std-src
        export LINUX_NONE_STD_SRC=${std-src}
        export __CARGO_TESTS_ONLY_SRC_ROOT=${std-src}/library
      '';

      # the example programs, built in release mode for one target
      examplesFor = name: targetArg: pkgs.stdenv.mkDerivation {
        name = "linux-none-examples-${name}";
        src = fs.toSource {
          root = ./.;
          fileset = fs.unions [ ./apps/Cargo.toml ./apps/Cargo.lock ./apps/.cargo ./apps/examples ./apps/fuzzee ./apps/stdtests ./targets ];
        };
        nativeBuildInputs = [ toolchain ];
        # static binaries: nothing for fixup (strip, patchelf) to do
        dontFixup = true;
        buildPhase = ''
          ${cargoSetup}
          cd apps
          cargo build --release -p examples -p fuzzee --target ${targetArg}
        '';
        installPhase = ''
          mkdir -p $out/bin
          find target/${name}/release -maxdepth 1 -type f -executable -exec cp {} $out/bin/ \;
        '';
      };
      examples-x86_64 = examplesFor "x86_64-unknown-linux-none" "x86_64-unknown-linux-none";
      examples-aarch64 = examplesFor "aarch64-unknown-linux-none" "../targets/aarch64-unknown-linux-none.json";

      # `cargo test <args>` (the harness) in the sandbox
      harnessCheck = name: args: pkgs.stdenv.mkDerivation {
        name = "linux-none-${name}";
        src = fs.toSource {
          root = ./.;
          fileset = fs.difference (fs.gitTracked ./.) (fs.unions [ ./flake.nix ./flake.lock ./.github ./README.md ]);
        };
        # getent: the args-env example is checked against the passwd database
        nativeBuildInputs = [ toolchain pkgs.qemu pkgs.strace pkgs.getent ];
        QEMU_AARCH64 = "${pkgs.qemu}/bin/qemu-aarch64";
        buildPhase = ''
          ${cargoSetup}
          cargo test ${args}
        '';
        installPhase = "touch $out";
      };
    in
    {
      packages.${system} = {
        inherit toolchain xtask std-src vendor cargo-config examples-x86_64 examples-aarch64;
        default = pkgs.symlinkJoin { name = "linux-none-examples"; paths = [ examples-x86_64 examples-aarch64 ]; };
      };

      checks.${system} = {
        # static, non-PIE executables for the right machine, with no
        # PT_INTERP / PT_DYNAMIC and no undefined symbol
        static-elf = pkgs.runCommand "linux-none-static-elf" { nativeBuildInputs = [ pkgs.binutils ]; } ''
          check() {
            for f in $1/bin/*; do
              readelf -hW $f | grep -q "Type: *EXEC" || { echo "$f: not ET_EXEC"; exit 1; }
              readelf -hW $f | grep -q "Machine: *$2" || { echo "$f: not $2"; exit 1; }
              ! readelf -lW $f | grep -qE "INTERP|DYNAMIC" || { echo "$f: dynamically linked"; exit 1; }
              [ -z "$(nm -u $f)" ] || { echo "$f: undefined symbols:"; nm -u $f; exit 1; }
            done
          }
          check ${examples-x86_64} "Advanced Micro Devices X86-64"
          check ${examples-aarch64} AArch64
          touch $out
        '';
        # examples and differential fuzzing, both arches (aarch64 under qemu)
        harness = harnessCheck "harness" "--test examples --test fuzz";
        # std's own test suites (slow)
        stdtests = harnessCheck "stdtests" "--test stdtests --test std_unittests";
      };

      devShells.${system}.default = pkgs.mkShell {
        packages = [ toolchain pkgs.git pkgs.qemu pkgs.strace pkgs.binutils ];
        QEMU_AARCH64 = "${pkgs.qemu}/bin/qemu-aarch64";
      };
    };
}
