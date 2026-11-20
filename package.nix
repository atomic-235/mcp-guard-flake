{
  lib,
  stdenvNoCC,
  fetchurl,
}:

let
  data = builtins.fromJSON (builtins.readFile ./version.json);
  system = stdenvNoCC.hostPlatform.system;

  asset =
    {
      x86_64-linux = "mcp-guard-linux-x86_64";
    }
    .${system} or (throw "mcp-guard: unsupported system ${system}");
in
stdenvNoCC.mkDerivation {
  pname = "mcp-guard";
  version = data.version;

  # Static musl binary with zero /nix/store references, so a plain fixed-output
  # fetch is enough.
  src = fetchurl {
    url = "https://github.com/atomic-235/mcp-guard-flake/releases/download/v${data.version}/${asset}";
    hash = data.assets.${system};
  };

  dontUnpack = true;

  installPhase = ''
    runHook preInstall
    install -Dm755 "$src" "$out/bin/mcp-guard"
    runHook postInstall
  '';

  meta = with lib; {
    description = "Stdio MCP gateway that restricts the tool, prompt, and resource surface of one server";
    homepage = "https://github.com/atomic-235/mcp-guard-flake";
    license = licenses.mit;
    platforms = [ "x86_64-linux" ];
    mainProgram = "mcp-guard";
  };
}
