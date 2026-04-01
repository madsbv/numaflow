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
      {
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = with pkgs; [
            # Rust toolchain
            rustc
            cargo
            rust-analyzer
            rustfmt
            clippy

            # Build dependencies
            protobuf
            cmake
            pkg-config
            clang
            rustPlatform.bindgenHook
          ];

          buildInputs = with pkgs; [
            openssl
          ];

          # Linux: Use nix clang
          # CC = "${pkgs.clang}/bin/clang";
          # LIBCLANG_PATH = "${pkgs.libclang}/lib";
          # LIBCLANG_PATH = "${pkgs.llvmPackages.libclang}/lib";
          # KRB5_DIR = "${pkgs.krb5}";
          # Platform-specific setup
        };
      }
    );
}
