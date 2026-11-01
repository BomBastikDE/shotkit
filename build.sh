#!/usr/bin/env bash
# Baut ShotKit.exe (Windows x64, eigenständig, eine Datei) nach dist/ – funktioniert auch auf Linux/ARM.
set -euo pipefail
cd "$(dirname "$0")/src"
export DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1
DOTNET="${DOTNET:-$HOME/.dotnet/dotnet}"
"$DOTNET" publish -c Release -o ../dist
rm -f ../dist/*.pdb
ls -lh ../dist/ShotKit.exe
