#!/usr/bin/env bash
# Fetch TEXMEX SIFT1M/GIST1M at runtime. Never bundle (redistribution license unknown, see research/DATASET_TERMS.md).
# Cite: Jegou, Douze, Schmid, "Product quantization for nearest neighbor search", IEEE TPAMI 2011.
set -euo pipefail
DEST="${1:?dest dir (outside the repo)}"
mkdir -p "$DEST"
for n in sift gist; do
  [ -d "$DEST/$n" ] && { echo "$n present"; continue; }
  curl -fL --retry 3 -o "$DEST/$n.tar.gz" "ftp://ftp.irisa.fr/local/texmex/corpus/$n.tar.gz"
  tar -xzf "$DEST/$n.tar.gz" -C "$DEST"
done
