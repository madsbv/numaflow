{
  description = "Numaflow Rust development environment";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      let
        buildDeps = with pkgs; [
          cmake
          pkg-config
          protobuf
          yarn
          rustPlatform.bindgenHook
          # Rust toolchain is managed by rustup + rust-toolchain.toml;
          # do not add rustc/cargo/clippy/rustfmt here as nixpkgs ships a
          # different version (1.94) than the pinned channel (1.93).
          rustup
        ];
      in
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = buildDeps;

          # krb5-src builds bundled C sources using the ambient gcc. GCC 15
          # (shipped by nixpkgs-unstable) defaults to -std=gnu23, which treats
          # empty parameter lists as zero-argument (C23/C++ semantics). The
          # krb5 RPC code uses the old "unspecified arguments" style, so we
          # force the C17 standard to restore the permissive interpretation.
          CFLAGS = "-std=gnu17";
        };

        apps.build-rust =
          let
            script = pkgs.writeShellApplication {
              name = "build-rust";
              runtimeInputs = buildDeps;
              text = ''
                REPO_ROOT="$(pwd)"

                # Map host architecture to Rust target triplet and Docker-style label.
                ARCH="$(uname -m)"
                case "$ARCH" in
                  x86_64)
                    TARGET="x86_64-unknown-linux-gnu"
                    LABEL="amd64"
                    ;;
                  aarch64)
                    TARGET="aarch64-unknown-linux-gnu"
                    LABEL="arm64"
                    ;;
                  *)
                    echo "Unsupported architecture: $ARCH" >&2
                    exit 1
                    ;;
                esac

                # Force C17 to avoid GCC 15 / krb5-src incompatibility.
                export CFLAGS="-std=gnu17"

                echo "Building Rust workspace for $TARGET …"
                cargo build \
                  --manifest-path "$REPO_ROOT/rust/Cargo.toml" \
                  --workspace \
                  --release 

                echo "Copying binaries to repo root …"
                cp "$HOME/.cargo/target/release/numaflow" \
                   "$REPO_ROOT/numaflow-rs-linux-$LABEL"
                cp "$HOME/.cargo/target/release/entrypoint" \
                   "$REPO_ROOT/entrypoint-linux-$LABEL"

                echo "Done: numaflow-rs-linux-$LABEL and entrypoint-linux-$LABEL are ready."
              '';
            };
          in
          {
            type = "app";
            program = script.outPath + "/bin/build-rust";
          };
      }
    );
}
