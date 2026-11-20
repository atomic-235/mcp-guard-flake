{
  lib,
  pkgs,
  ...
}:

# Built by CI, not by consumers: the resulting binary is published as the
# release asset that package.nix fetches.
#
# pkgsStatic gives a fully static musl binary, so the asset has no /nix/store
# references and a plain fixed-output fetch can consume it.
let
  data = builtins.fromJSON (builtins.readFile ./version.json);
in
pkgs.pkgsStatic.rustPlatform.buildRustPackage {
  pname = "mcp-guard";
  version = data.version;

  src = lib.cleanSourceWith {
    src = ./.;
    filter = path: _: baseNameOf path != "target";
  };
  cargoLock.lockFile = ./Cargo.lock;

  doCheck = true;

  meta = with lib; {
    description = "Stdio MCP gateway that restricts the tool, prompt, and resource surface of one server";
    homepage = "https://github.com/atomic-235/mcp-guard-flake";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "mcp-guard";
  };
}
