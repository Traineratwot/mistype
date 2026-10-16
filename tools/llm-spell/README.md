# llm-spell

Local spell / typo corrector for SwitchFix hybrid layer (`spell.llm_cmd`).

**≤300 MB models**, offline after download, JSON-lines protocol matching
`ProcessCorrector` in `switchfix-adapters`.

## Quick start

```bash
# 1) rules-only (no ML, works immediately)
./llm-spell --backend rules --word приветт --lang ru

# 2) with a small T5 model (~240 MB, needs pip install)
pip install --user 'transformers>=4.40' 'torch>=2.2' --index-url https://download.pytorch.org/whl/cpu
./llm-spell --backend t5            # first run downloads the model
# or pick a model explicitly:
LLM_SPELL_MODEL=google/flan-t5-small ./llm-spell --backend t5

# 3) via Ollama (Q4 GGUF, e.g. qwen2.5:0.5b ≈ 300 MB)
ollama pull qwen2.5:0.5b
./llm-spell --backend ollama
```

Install into PATH and point SwitchFix at it:

```bash
sudo install -m 0755 llm-spell /usr/local/bin/llm-spell
```

`~/.config/switchfix/config.toml`:

```toml
[spell]
backend = "hybrid"
llm_cmd = "llm-spell"
# or: llm_cmd = "/usr/local/bin/llm-spell --backend t5"
```

Restart the daemon after config changes.

## Protocol

Process stays alive (model stays warm). Line-oriented JSON:

```
→ {"ready":true,"backend":"t5","version":"0.1.0"}          # greeting
→ {"word":"приветт","context":"всем приветт","lang":"ru"}
← {"is_ok":false,"suggestions":["привет"],"backend":"t5"}
← {"is_ok":true,"backend":"t5"}    # word is fine — veto dictionary false positives
```

Bare lines and `{"word": "..."}` also work. `quit` / `exit` ends the process.

## Backends

| Backend | Size | Needs | Quality |
|---------|------|-------|---------|
| `t5` | ~240 MB | `pip install transformers torch` | best for RU typos / ASR-style |
| `ollama` | ~80–300 MB Q4 | `ollama` + pulled model | good, can over-rewrite — we ask for JSON + diff |
| `rules` | 0 | hunspell-ru/en (optional) | decent nonword, no realword |

Auto order: **t5 → ollama → rules**.

### Recommended models (≤300 MB)

| Model | ~size | Notes |
|-------|-------|-------|
| `UrukHan/t5-russian-spell` | ~240 MB | default, RU spell from noisy text |
| `google/flan-t5-small` | ~300 MB | general EN, less RU-specific |
| `qwen2.5:0.5b` (ollama Q4) | ~300 MB | multilingual, better RU than SmolLM |
| `smollm2:135m` (ollama) | ~90 MB | fastest, mostly EN |

Env overrides:

```bash
LLM_SPELL_BACKEND=t5|ollama|rules|auto
LLM_SPELL_MODEL=UrukHan/t5-russian-spell
LLM_SPELL_OLLAMA_MODEL=qwen2.5:0.5b
LLM_SPELL_CACHE=~/.local/share/switchfix/models
LLM_SPELL_WORDLIST=/path/extra-words.txt
```

## Safety / privacy

- No network except **your** Ollama (`127.0.0.1`) or HuggingFace download once.
- Typed context is RAM-only in the child process; SwitchFix never logs it.
- `llm_cmd` is split on spaces (no shell) — no command injection.

## Test

```bash
./llm-spell --backend rules --word helo --lang en
./llm-spell --backend rules --once <<< '{"word":"приветт","lang":"ru"}'
```
