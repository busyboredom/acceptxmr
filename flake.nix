{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        stableToolchain = pkgs.rust-bin.stable.latest.default.override {
          extensions = [
            "rust-src"
            "rust-analyzer"
          ];
        };
        nightlyFmt = pkgs.rust-bin.nightly.latest.rustfmt;
        fmtr = nixpkgs.legacyPackages.${system}.alejandra;
      in
      {
        formatter = fmtr;
        devShells.default =
          pkgs.mkShell.override
            {
              stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.clangStdenv;
            }
            {
              packages = with pkgs; [
                gcc
                pkg-config
                stableToolchain
                nightlyFmt
                openssl.dev
                typos-lsp
              ];

              shellHook = ''
                export CARGO="${stableToolchain}/bin/cargo"
                export RUSTFMT="${nightlyFmt}/bin/rustfmt"
              '';
            };
      }
    );
}
