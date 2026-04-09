# ArmaraOS Future Features Roadmap

This document captures planned features that have been scoped but deferred for future implementation.

## Dashboard & UX Enhancements

### FEATURE-4: Add AINL Graph Visualizer to Dashboard (Program Inspector)

**Status:** Planned  
**Priority:** Medium  
**Effort:** Large

**Description:**  
Add an interactive graph visualizer to the dashboard that allows users to inspect AINL programs visually. This would show:
- Node structure and connections
- Data flow through the graph
- Adapter usage and dependencies
- Interactive exploration of program logic

**Technical Approach:**
- Add new page route `#ainl-inspector` or integrate into `#ainl-library`
- Parse `.ainl` files and extract IR graph structure
- Use a graph visualization library (e.g., D3.js, Cytoscape.js, or vis.js)
- Support zoom, pan, and node inspection
- Show node types, adapter calls, and data transformations
- Link to source code positions

**Files to Modify:**
- `crates/openfang-api/static/js/pages/ainl-library.js` - Add inspector integration
- `crates/openfang-api/static/index_body.html` - Add visualizer page/modal
- New: `crates/openfang-api/static/js/ainl-graph-viz.js` - Visualization logic
- `crates/openfang-kernel/src/ainl_library.rs` - Add IR parsing/export endpoint if needed

**Dependencies:**
- Graph visualization library (add to static assets)
- AINL IR parser/reader (may already exist in `ainativelang`)
- API endpoint to get parsed AINL structure: `GET /api/ainl/library/{path}/graph`

---

### FEATURE-5: Replace Stub AINL Programs with Impressive Showcase Programs

**Status:** Planned  
**Priority:** High  
**Effort:** Medium

**Description:**  
The current embedded AINL programs (`programs/`) are functional but basic. Replace or augment them with more impressive, real-world examples that showcase ArmaraOS + AINL capabilities:

- **Agent orchestration demo** - Multi-agent collaboration graph
- **Smart home automation** - IoT device control patterns
- **Data pipeline** - ETL workflow with transformations
- **Intelligent routing** - Context-aware task distribution
- **Adaptive monitoring** - Self-adjusting thresholds and alerts
- **Workflow automation** - Email/Slack/calendar integration examples

**Technical Approach:**
1. Design 3-5 compelling showcase programs that demonstrate:
   - Multi-step logic with branching
   - External API integration (weather, news, GitHub, etc.)
   - Vector memory and search
   - Agent tool integration
   - Practical utility value
2. Create `.ainl` files in `programs/showcase/`
3. Add comprehensive README files explaining each program
4. Update `curated_ainl_cron.json` to include opt-in showcase schedules
5. Increment `EMBEDDED_PROGRAMS_REVISION`
6. Add smoke tests for each showcase program

**Files to Modify:**
- New: `programs/showcase/*.ainl` - New showcase programs
- New: `programs/showcase/README.md` - Documentation
- `crates/openfang-kernel/src/curated_ainl_cron.json` - Optional schedules
- `crates/openfang-kernel/src/embedded_ainl_programs.rs` - Bump revision
- `docs/ootb-ainl.md` - Document showcase programs

**Showcase Program Ideas:**
1. **GitHub PR Summarizer** - Monitors repo, generates PR summaries via LLM
2. **Smart Budget Optimizer** - Analyzes spend patterns, suggests model switches
3. **Contextual Task Router** - Routes tasks to appropriate agents based on complexity
4. **Multi-Source News Digest** - Fetches from RSS/APIs, deduplicates, summarizes
5. **Proactive Health Monitor** - Detects anomalies, auto-scales, sends alerts

---

### FEATURE-6: Add In-App AINL Program Editor to Dashboard

**Status:** Planned  
**Priority:** Medium  
**Effort:** Large

**Description:**  
Add a browser-based code editor to the dashboard that allows users to create, edit, and test AINL programs without leaving the UI. This would make ArmaraOS more accessible to non-technical users and provide instant feedback.

**Features:**
- Syntax highlighting for `.ainl` files
- Auto-completion for adapters and built-in functions
- Real-time validation (syntax check)
- Test runner with sample inputs
- Save to `~/.armaraos/ainl-library/user-programs/`
- Load existing programs from library
- Integration with scheduler (schedule edited program)

**Technical Approach:**
1. Choose a browser code editor:
   - **Monaco Editor** (VS Code engine) - Full-featured, larger bundle
   - **CodeMirror 6** - Lightweight, customizable
   - **Ace Editor** - Mature, good YAML/DSL support
2. Create custom syntax mode for AINL
3. Add API endpoints:
   - `POST /api/ainl/library/user-programs` - Save new program
   - `PUT /api/ainl/library/{path}` - Update existing
   - `POST /api/ainl/validate` - Validate syntax (calls `ainl validate`)
   - `POST /api/ainl/test-run` - Run program with test inputs
4. Add editor page or modal

**Files to Modify:**
- New: `crates/openfang-api/static/js/pages/ainl-editor.js` - Editor component
- `crates/openfang-api/static/index_body.html` - Add editor page/modal
- New: `crates/openfang-api/static/js/ainl-syntax.js` - Syntax highlighting
- `crates/openfang-api/src/routes.rs` - Add save/validate endpoints
- `crates/openfang-kernel/src/ainl_library.rs` - Add write operations

**Integration Points:**
- Link from App Store: "Edit this program"
- "New Program" button in `#ainl-library`
- Link from Scheduler: "Edit scheduled program"
- Validation uses desktop AINL venv or system `ainl` binary

**UX Considerations:**
- Auto-save to localStorage during editing
- Confirm before overwriting existing programs
- Show validation errors inline
- Provide starter templates for common patterns
- Test runner shows live output stream

---

## Implementation Priority

1. **FEATURE-5** (Showcase Programs) - High impact, demonstrates capabilities
2. **FEATURE-6** (In-App Editor) - Enables self-service program creation
3. **FEATURE-4** (Graph Visualizer) - Advanced feature for power users

## Notes

- All features should follow the existing dashboard patterns (Alpine.js components, card-based layouts)
- Maintain accessibility standards (ARIA labels, keyboard navigation)
- Add comprehensive testing (manual QA checklist in `docs/dashboard-testing.md`)
- Update `CHANGELOG.md` when implementing
- Consider progressive enhancement (features degrade gracefully if dependencies unavailable)

---

**Last Updated:** 2026-04-08  
**Roadmap Version:** 1.0
