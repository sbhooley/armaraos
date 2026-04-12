# Anthropic Prompt Caching Implementation

## Overview

ArmaraOS now automatically enables **Anthropic prompt caching** for all Claude models, including when accessed via OpenRouter. This provides **massive cost savings** on multi-turn conversations.

## Cost Savings

- **Direct Anthropic**: 90% reduction on cached input tokens ($3.00 → $0.30/MTok for Sonnet 4.6)
- **Combined with input compression**: 60-70% total API cost reduction
- **Cache hit rates**: Expect 60-80%+ after the first turn

## How It Works

### Automatic Cache Markers

The Anthropic driver automatically marks content for caching:

1. **System prompt** — Cached on last text block
2. **Conversation history** — Second-to-last message cached (keeps current user message fresh)

### Cache Strategy

- **Turn 1**: System prompt cached (cache write)
- **Turn 2+**: System prompt + history cached (cache read + small write for new content)
- **Max breakpoints**: 2 (system + history)

## Configuration

### Direct Anthropic

```toml
[llm]
provider = "anthropic"
# model = "claude-sonnet-4-20250514"  # in agent manifest
```

**Environment**: `ANTHROPIC_API_KEY=sk-ant-...`

Prompt caching is **always enabled** — no configuration needed.

### OpenRouter + Anthropic Models

**Two ways to enable caching with OpenRouter:**

#### Option 1: Automatic (Recommended)
```toml
[llm]
provider = "openrouter"  # Use OpenRouter provider
# model = "anthropic/claude-sonnet-4-20250514"  # In agent manifest
```

**Environment**: `OPENROUTER_API_KEY=sk-or-v1-...`

The system **automatically detects** when you're using an Anthropic model (starts with `anthropic/`) and routes to the Anthropic driver with caching enabled.

#### Option 2: Manual
```toml
[llm]
provider = "anthropic"  # Explicitly use Anthropic driver
base_url = "https://openrouter.ai/api/v1"  # Route through OpenRouter
```

**Environment**: `ANTHROPIC_API_KEY=sk-or-v1-...` (your OpenRouter API key)

Both methods work identically — Option 1 is more intuitive for users already using `provider="openrouter"`.

## Telemetry

### Logs

When caching is active, debug logs show:

```
cache_write=1234 cache_read=5678 cache_hit_rate=82 input=7000 output=500
```

### Token Usage Tracking

`TokenUsage` now includes:
- `cache_creation_input_tokens` — Tokens written to cache
- `cache_read_input_tokens` — Tokens read from cache
- `cache_hit_rate()` — Percentage (0-100)

These fields are exposed in the API response and can be tracked for cost analysis.

## Expected Savings

### Example: 10-turn conversation with Sonnet 4.6

**Without caching + compression:**
- System: 2000 tokens @ $3.00/MTok = $0.006 per turn × 10 = $0.060
- History: grows 500 tokens/turn avg @ $3.00/MTok = $0.075 total
- **Total input cost**: $0.135

**With caching + compression (balanced mode):**
- Turn 1: 2000 tokens compressed to 1100 @ $3.00/MTok = $0.0033 (cache write)
- Turn 2-10: 
  - Cache read: ~1500 tokens avg @ $0.30/MTok = $0.00045/turn
  - New content: ~300 tokens @ $3.00/MTok = $0.0009/turn
  - Combined: $0.00135/turn × 9 = $0.01215
- **Total input cost**: $0.0155

**Savings**: 88% reduction on input costs
**Combined with output compression**: 65-70% total API cost reduction

## Limitations

- Cache TTL: 5 minutes (Anthropic's default)
- Only works with Anthropic models (Claude)
- OpenRouter adds markup (~50%) but caching still applies to their inflated rates

## Troubleshooting

### Check if caching is working

Set debug logging:
```bash
RUST_LOG=openfang_runtime::drivers::anthropic=debug
```

Look for log lines with `cache_read` > 0 after the first turn.

### Common issues

**Q: Using OpenRouter but not seeing cache savings?**
- Verify you're using `provider = "anthropic"` with `base_url = "https://openrouter.ai/api/v1"`
- Check model name starts with `anthropic/`

**Q: Cache hit rate is 0%?**
- First turn always writes cache (hit rate = 0)
- Check if turns are more than 5 minutes apart (cache expires)

## Implementation Details

- **Code**: `crates/openfang-runtime/src/drivers/anthropic.rs`
- **Types**: `crates/openfang-types/src/message.rs` (TokenUsage)
- **Docs**: `docs/prompt-compression-efficient-mode.md` (input compression)
