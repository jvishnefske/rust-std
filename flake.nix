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
        # xtask has no dependencies: build it without cargo (and its index)
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
    in
    {
      packages.${system} = { inherit toolchain std-src vendor cargo-config; };

      devShells.${system}.default = pkgs.mkShell {
        packages = [ toolchain pkgs.git pkgs.qemu pkgs.strace pkgs.binutils ];
        QEMU_AARCH64 = "${pkgs.qemu}/bin/qemu-aarch64";
      };
    };
}
