#!/usr/bin/env bash
# Install llm-spell into /usr/local/bin and (optionally) pip deps for t5.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-/usr/local}"

echo ">> install llm-spell → ${PREFIX}/bin/llm-spell"
install -m 0755 "${ROOT}/llm-spell" "${PREFIX}/bin/llm-spell"

if [[ "${WITH_T5:-0}" == "1" ]]; then
  echo ">> pip install transformers+torch (CPU)"
  python3 -m pip install --user -r "${ROOT}/requirements.txt" \
    --index-url https://download.pytorch.org/whl/cpu
  echo ">> warm model cache (UrukHan/t5-russian-spell)"
  LLM_SPELL_BACKEND=t5 llm-spell --word тест --lang ru >/dev/null || true
fi

echo
echo "Config (~/.config/switchfix/config.toml):"
echo '  [spell]'
echo '  backend = "hybrid"'
echo '  llm_cmd = "llm-spell"'
echo
echo "Then: pkill -x switchfix && switchfix -v daemon"
