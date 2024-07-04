{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  };
  outputs = {
    self,
    nixpkgs,
    flake-utils,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {inherit system;};
        fmtr = nixpkgs.legacyPackages.${system}.alejandra;
      in {
        formatter = fmtr;
        devShells.default =
          pkgs.mkShell.override {
            stdenv = pkgs.stdenvAdapters.useMoldLinker pkgs.clangStdenv;
          } {
            packages = with pkgs; [
              gcc
              rustup
              pkg-config
              rust-analyzer
              openssl.dev
              typos-lsp
            ];

            shellHook = ''
              alias clippy="cargo +nightly clippy --all-targets --all-features"
              alias test="cargo +nightly test --all-targets --all-features"
            '';
          };
      }
    );
}
