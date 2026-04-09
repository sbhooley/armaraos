# Desktop installers: trust prompts and code signing

This document explains why users sometimes see **security warnings** when installing the ArmaraOS desktop app (Tauri), how that differs from **Tauri updater signing**, and what maintainers can do on **macOS** and **Windows**. It also summarizes **third-party** options (SignPath OSS, Azure Artifact Signing) discussed for **GitHub-hosted** CI.

For release mechanics (tags, `latest.json`, website sync), see **[release-desktop.md](release-desktop.md)**. For secrets and key generation, see **[production-checklist.md](production-checklist.md)**.

---

## Two different kinds of “signing”

| Mechanism | What it proves | Fixes OS install warnings? |
|-----------|----------------|----------------------------|
| **Tauri updater** (`TAURI_SIGNING_PRIVATE_KEY`, pubkey in `tauri.conf.json`) | Updates are **integrity-checked** against `latest.json` | **No** — it does not replace Windows Authenticode or Apple Developer ID trust |
| **macOS**: Developer ID + (ideally) **notarization** | Gatekeeper trusts the app / DMG | **Yes** (when configured in CI) |
| **Windows**: **Authenticode** (certificate or managed signing service) | File Properties shows a real **Publisher**; SmartScreen treats the binary as signed | **Mostly** — see SmartScreen note below |

---

## macOS: “unidentified developer” and quarantine

Without a **Developer ID Application** certificate (and ideally **notarization**), users may see:

- “can’t be opened because it is from an unidentified developer”
- First-run friction from **Gatekeeper** / quarantine on downloaded `.dmg` / `.app`

**What to configure**

- Apple Developer Program membership and a **Developer ID Application** identity.
- CI: import signing material and run notarization. The **`desktop`** job in **`.github/workflows/release.yml`** documents the expected GitHub Actions secrets (see that file for the canonical list).

**Secrets used by the release workflow** (do not rely on outdated names from generic Tauri docs):

| Purpose | Secret names |
|---------|----------------|
| Developer ID `.p12` | `MAC_CERT_BASE64`, `MAC_CERT_PASSWORD` |
| Notarization | `MAC_NOTARIZE_APPLE_ID`, `MAC_NOTARIZE_PASSWORD`, `MAC_NOTARIZE_TEAM_ID` |

If `MAC_CERT_BASE64` is missing, the workflow falls back to **ad-hoc** signing and warns that builds will not be notarized.

---

## Windows: SmartScreen and “Unknown publisher”

Unsigned installers often show **Unknown publisher** and **SmartScreen** warnings. Signing with a valid **Authenticode** chain fixes the publisher identity; **EV** certificates historically help **SmartScreen reputation** warm faster than **OV**, but neither guarantees zero warnings for brand-new publishers.

**Repository config**

- `crates/openfang-desktop/tauri.conf.json` → `bundle.windows.certificateThumbprint` is the hook for **classic** PFX-based signing on the build machine. If it is `null`, CI is not applying a thumbprint from committed config (you may still add signing steps later).

**Typical approaches on GitHub-hosted runners**

1. **Certificate + thumbprint** — Import a `.pfx` in CI, set thumbprint (often via env / generated config), let Tauri/`signtool` sign the bundle.
2. **Azure Artifact Signing** (formerly Trusted Signing) — Managed signing in Azure; authenticate from GitHub Actions (commonly **OIDC** / workload identity federation). See Microsoft Learn: [Artifact Signing](https://learn.microsoft.com/en-us/azure/artifact-signing/) and pricing: [Artifact Signing pricing](https://azure.microsoft.com/pricing/details/artifact-signing/).
3. **SignPath** (including OSS programs) — Signing as a service with HSM-backed keys; separate onboarding and policies. See [SignPath open source](https://signpath.io/solutions/open-source-community) and foundation terms: [signpath.org/terms](https://signpath.org/terms).

---

## Azure Artifact Signing: cost and Entra licensing

- **Billing**: Artifact Signing is billed as its own service (see the current **Azure pricing** page for Artifact Signing; plans have included a low monthly base plus a signature quota).
- **Microsoft Entra ID P1**: **Not** required merely to use **OpenID Connect** from GitHub Actions to Azure. OIDC federation for GitHub Actions uses standard app registration / federated credentials in a normal Entra tenant. **Entra ID P1/P2** are for premium directory features (e.g. Conditional Access policies), not for “basic OIDC to sign in.”
- You still need an **Azure subscription** that supports the Artifact Signing resource provider; check Microsoft Learn for current restrictions (e.g. trial limitations).

---

## SignPath (free OSS option): what you must plan for

SignPath Foundation’s terms describe **eligibility** for free OSS signing and extra rules if the **certificate is issued to SignPath Foundation** (their name on the publisher). In short:

- **OSI license**, no commercial dual-licensing, no proprietary blobs in the signed product, project maintained and documented.
- **Additional** requirements for foundation-issued certs: e.g. published **code signing policy**, **privacy** disclosures if you collect data, role definitions (authors / reviewers / approvers), MFA, and restrictions on “hacking tools” as defined in their terms.

Whether a given repo qualifies is **SignPath’s decision**; read the current text at **[signpath.org/terms](https://signpath.org/terms)** before applying.

**ArmaraOS-specific note:** The desktop app may send **optional** anonymous product analytics (PostHog) as documented in the root **README** and **[release-desktop.md](release-desktop.md)** (CI: **`ARMARAOS_POSTHOG_KEY`** or **`AINL_POSTHOG_KEY`**). SignPath’s terms require clear disclosure and user control for data collection; ensure your privacy story matches what you ship.

---

## Individual maintainers (no legal entity yet)

- **EV** code signing certificates are often aimed at **verified organizations**; **OV** or **individual validation** products may be easier initially.
- **Cloud / keyless** signing (Azure Artifact Signing, vendor cloud HSM, or SignPath-style services) avoids USB tokens on ephemeral runners.

---

## SmartScreen: expectations

Even with correct Authenticode signing, **SmartScreen** may still warn occasionally until **reputation** builds (download volume, consistency of publisher, time). This is normal; signing is still the right baseline.

---

## Related files

| File | Role |
|------|------|
| `crates/openfang-desktop/tauri.conf.json` | Updater pubkey, `bundle.windows.certificateThumbprint`, bundle settings |
| `.github/workflows/release.yml` | Desktop matrix, macOS cert import + notarization env, `tauri-apps/tauri-action` |
| [desktop.md](desktop.md) | Desktop architecture, updater overview |
| [production-checklist.md](production-checklist.md) | Tauri signing keys and secret names |
| [release-desktop.md](release-desktop.md) | Tag workflow, website mirror, smoke checklist |
