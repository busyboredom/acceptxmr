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
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            gcc
            rustup
            pkg-config
            openssl.dev
          ];
        };
      }
    );
}
