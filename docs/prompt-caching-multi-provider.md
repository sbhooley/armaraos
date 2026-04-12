# Multi-Provider Prompt Caching Guide

## Overview

Prompt caching support varies significantly across providers. This guide covers implementation status and capabilities for each provider.

---

## ✅ Anthropic (Claude) — FULLY IMPLEMENTED

**Status**: Production-ready with automatic caching

**Supported Models**: All Claude models (Opus, Sonnet, Haiku)

**Implementation**: Active cache_control markers on system prompt + conversation history

**Cost Savings**:
- 90% reduction on cached tokens ($3.00 → $0.30/MTok for Sonnet 4.6)
- Combined with compression: 65-70% total savings

**Configuration**:
```toml
provider = "anthropic"  # Direct
# OR
provider = "openrouter"  # Auto-detected for anthropic/* models
```

**Cache TTL**: 5 minutes  
**Max breakpoints**: 4 (we use 2: system + history)

---

## ⚠️ OpenAI — AUTOMATIC (Limited Models)

**Status**: Automatic caching for gpt-4o/gpt-4o-mini only

**Supported Models**:
- `gpt-4o`
- `gpt-4o-mini`

**Implementation**: OpenAI's automatic prompt caching (no explicit cache_control needed)

**How It Works**:
- OpenAI automatically caches prompt prefixes ≥1024 tokens
- Cache hit if same prefix appears within 5-10 minutes
- No code changes required — works transparently

**Cost Savings**:
- 50% reduction on cached inputs ($2.50 → $1.25/MTok for gpt-4o)
- Combined with compression: 55-60% total savings

**Configuration**:
```toml
provider = "openai"  # Works automatically
# OR
provider = "openrouter"
model = "openai/gpt-4o"  # Works via OpenRouter
```

**Cache Requirements**:
- Minimum prefix length: 1024 tokens
- Cache TTL: 5-10 minutes (varies)
- No explicit control over what's cached

**Limitations**:
- Only for gpt-4o and gpt-4o-mini (not gpt-4-turbo, gpt-3.5, etc.)
- No control over cache markers
- Lower savings than Anthropic (50% vs 90%)

---

## ❌ Google (Gemini) — NOT IMPLEMENTED

**Status**: Requires complex implementation

**Supported Models**: Gemini 1.5 Pro, Gemini 1.5 Flash

**Why Not Implemented**:
1. Requires **separate cache creation API call** before each conversation
2. Cache must be created with specific content, then referenced by ID
3. Cache lifecycle management (manual TTL tracking, cleanup)
4. Complex two-phase flow not compatible with current driver architecture

**Cost Savings (If Implemented)**:
- 75% reduction on cached tokens ($0.315 → $0.07875/MTok for Gemini 1.5 Pro)
- Separate pricing for cache storage

**How Gemini Caching Works**:
```python
# Step 1: Create cache (separate API call)
cache = genai.caching.CachedContent.create(
    model="gemini-1.5-pro-001",
    system_instruction="...",
    contents=[...],  # Conversation history
    ttl=datetime.timedelta(minutes=5)
)

# Step 2: Use cache in request
response = model.generate_content(
    "user message",
    cached_content=cache.name  # Reference by ID
)
```

**Implementation Complexity**: High (6-8 hours)
- Add cache creation/management layer
- Track cache IDs per conversation
- Handle cache expiration & cleanup
- Two API calls per turn instead of one

**Recommendation**: Wait for Gemini to add inline cache control like Anthropic

---

## ❌ DeepSeek — NO CACHING SUPPORT

**Status**: Provider does not support prompt caching

**Supported Models**: N/A

**Why**: DeepSeek's API is OpenAI-compatible but doesn't implement prompt caching features

**Cost Savings**: 0% (input compression only: ~45%)

**Workaround**: None — focus on input/output compression instead

---

## 🔄 OpenRouter Pass-Through Caching

OpenRouter supports caching when the underlying provider supports it:

| Provider via OpenRouter | Caching Support | Auto-Detected |
|------------------------|----------------|---------------|
| `anthropic/*` | ✅ Yes (90%) | ✅ Automatic |
| `openai/gpt-4o*` | ✅ Yes (50%) | ✅ Automatic |
| `google/gemini-*` | ❌ No (complex) | N/A |
| `deepseek/*` | ❌ No | N/A |
| `meta-llama/*` | ❌ No | N/A |
| `mistralai/*` | ❌ No | N/A |

**OpenRouter Configuration**:
```toml
provider = "openrouter"
# Model in manifest determines caching automatically
```

**How Auto-Detection Works**:
- `model.starts_with("anthropic/")` → Routes to Anthropic driver (caching enabled)
- `model.starts_with("openai/gpt-4o")` → OpenAI driver (automatic caching)
- All others → Standard OpenAI driver (no caching)

---

## Summary Table

| Provider | Cache Support | Savings | Implementation | Auto-Detect |
|----------|--------------|---------|----------------|-------------|
| **Anthropic** | ✅ Full | 90% | Complete | ✅ Yes |
| **OpenAI (gpt-4o)** | ⚠️ Automatic | 50% | Built-in | ✅ Yes |
| **OpenAI (others)** | ❌ None | 0% | N/A | N/A |
| **Gemini** | ⚠️ Complex API | 75% | Not impl. | ❌ No |
| **DeepSeek** | ❌ None | 0% | N/A | N/A |

---

## Recommendations

### For Maximum Cost Savings
1. **Use Anthropic Claude** (Sonnet 4.6 recommended)
   - Best caching: 90% reduction
   - Works via OpenRouter or direct
   - Combined with compression: 65-70% total savings

### For OpenAI Users
2. **Use gpt-4o or gpt-4o-mini**
   - Automatic caching (50% reduction)
   - Works transparently
   - Good balance of performance and cost

### For Google/DeepSeek Users
3. **Rely on input/output compression**
   - ~45% total savings without caching
   - Still significant cost reduction
   - No implementation complexity

---

## Future Work

### Gemini Context Caching (Medium Priority)
- **Effort**: 6-8 hours
- **Savings**: 75% on cached tokens
- **Blockers**: Requires cache management layer
- **Decision**: Wait for simpler API or high user demand

### Provider-Specific Optimizations
- **Groq**: No caching needed (already ultra-fast + cheap)
- **Mistral**: Check if they add caching support
- **Llama via OpenRouter**: No caching available

---

## Testing Caching

To verify caching is working:

```bash
# Enable debug logging
RUST_LOG=openfang_runtime::drivers=debug cargo run

# Look for log lines with:
# - cache_read > 0 (indicates cache hits)
# - cache_hit_rate > 70 (good performance)
```

For Anthropic:
```
cache_write=1234 cache_read=5678 cache_hit_rate=82
```

For OpenAI (gpt-4o):
Check API response for reduced token counts on subsequent requests.
