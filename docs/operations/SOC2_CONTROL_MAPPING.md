# SOC2-style control mapping (TSC) — product vs operator

This document maps common **AICPA Trust Services Criteria (TSC)** control themes to **what ArmaraOS provides in software** versus **what deployers and enterprises must own** in their environment, process, and monitoring stack. Use it for **shared responsibility** statements and customer questionnaires. Align criterion IDs to the exact **TSC** publication year your auditor uses; numbering and labels can differ slightly by edition.

**Sister document (AINL language & runtime, not the host OS):** the [AINL SOC2 / TSC control mapping in **ainativelang**](https://github.com/sbhooley/ainativelang/blob/main/docs/operations/AINL_SOC2_CONTROL_MAPPING.md) (`docs/operations/AINL_SOC2_CONTROL_MAPPING.md`) covers the compiler, Python runtime, HTTP runner audit stream, and workspace isolation — not ArmaraOS-specific controls.

**Related in-repo:** [`../security.md`](../security.md) (security systems reference), [`../../SECURITY.md`](../../SECURITY.md) (reporting policy).

---

## Shared responsibility (summary)

ArmaraOS implements **in-process** controls: **agent** least-privilege (**capabilities**, **tool policy**), **kernel** **RBAC** for configured human/API use, a **tamper-evident** **Merkle audit** trail (with HTTP inspect/verify; see [`../api-reference.md`](../api-reference.md)), and other layers documented in [`../security.md`](../security.md).

**Organizational** access (e.g. tenants, org-wide IdP, SCIM), **network and host** boundaries, **encryption** at rest and in motion for a given deployment, **log retention** in a compliance or WORM store, and **SIEM** correlation (including **human user identity** and **client IP** for every event) are typically **customer or operator** responsibilities unless you run a **managed** service that explicitly includes them.

---

## CC6 — Logical and physical access controls

| Criterion (typical intent) | In ArmaraOS (software) | Operator / customer | Notes |
|----------------------------|--------------------------|------------------------|--------|
| **CC6.1** — Logical access: software, infrastructure, architecture | Capabilities; RBAC (owner / admin / user / viewer); path / SSRF / tool policy; WASM sandbox; OFP wire auth (see `SECURITY.md` / `docs/security.md`) | OS hardening, segmentation, install/run policy, endpoint protection | **Multi-tenant control plane** (orgs, teams, IdP) is not a first-class product object. |
| **CC6.2** — Register / authorize before system credentials | User config, channel bindings, `api_key_hash` | IdP provisioning, secrets manager, API key lifecycle | **SSO/SCIM** is not built-in. |
| **CC6.3** — Remove or deactivate access | Edits to user config; key changes | Offboarding, rotation, HR-driven deprovisioning | Deprovisioning is **process** + **automation** around config, not a cloud user directory. |
| **CC6.4** — Physical access to facilities | *N/A* (software) | Device / facility / colo physical security | |
| **CC6.5** — Logical access to protected information | Capability grants; child cannot exceed parent; secret handling (see `docs/security.md`) | Data classification, DLP, encryption of home dir / backups | **Classification** of business data is **customer**. |
| **CC6.6** — Restrict logical access to sensitive data | Per-agent memory; tool allow/deny; path restrictions | Host filesystem permissions, FDE, backup access | **Host OS** may access the same paths as the process. |
| **CC6.7** — Restriction of data during transmission (e.g. encryption) | TLS / mTLS where used for admin or wire; SSRF mitigations | Cert management, TLS for all admin surfaces, perimeter controls | **End-to-end** story is the **stack**. |
| **CC6.8** — Malware and unauthorized software | Ed25519 manifests, skill scanning, supply-chain awareness in policy | Change control, allowlisted installs | **Governance** of third-party skills is **process**. |

---

## CC7 — System operations (detection, monitoring, response)

| Criterion (typical intent) | In ArmaraOS (software) | Operator / customer | Notes |
|----------------------------|--------------------------|------------------------|--------|
| **CC7.1** — Detect and communicate anomalies and events | Merkle **audit** log; `GET /api/audit/recent`, `GET /api/audit/verify`; graph-memory JSONL audit (separate surface) | SIEM, alerting, runbooks | **Integrity** of the in-product log is strong; **actor (human) + IP** are not the default **uniform** fields on every line — **enrich** in SIEM. |
| **CC7.2** — Monitor system components | Health endpoints (redaction per security doc); usage persistence (see architecture/monitoring docs) | Infra APM, disk/CPU, log shipping, SLOs | No managed 24/7 NOC. |
| **CC7.3** — Evaluate security events | `verify_integrity()` for audit chain; policy deny outcomes in logs | SOC playbooks, correlation | Detection logic is **customer** SIEM. |
| **CC7.4** — Respond to security incidents and breaches | Hardening and safe defaults; `SECURITY.md` contact | IR plan, comms, forensics | **Product** is one component. |
| **CC7.5** — Vulnerabilities and system defects (identify, log, track) | Supported versions, disclosure process, development practices | Patching, VM, image updates | **Dependency and image** cadence is **operator**. |

---

## Suggested control ownership (matrix row)

| Control area | ArmaraOS (design) | Operator (run time) |
|--------------|-------------------|---------------------|
| Agent permission model | Yes (capabilities, tool policy) | Tunes manifests, skills, policy |
| Human RBAC | Yes (kernel config) | Provisions users, keys, service restarts |
| Tamper-evident audit | Yes (Merkle chain + verify API) | Export, retain, monitor, alert |
| Enterprise IAM, tenancy, ABAC | Not by default | IdP, namespaces, separate installs, wrappers |
| Immutable WORM / legal-hold log archive | Not default | Object lock, secondary store, log pipeline |

---

## Disclaimer

This is **not** legal or audit advice. A SOC2 report is issued for a **system description** and **control objectives** in scope; map this document to your auditor’s criteria list and to any **customer-specific** deployment architecture.
