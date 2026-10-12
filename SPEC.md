# Cogito - Project Specification

> *"Cogito, ergo sum"* — The intelligent context harness for AI coding agents.

## 1. Concept & Vision

**Cogito** is an open-source context optimization harness that sits between AI coding agents and their context sources. It reduces token consumption, improves context relevance, and accelerates agent performance — without modifying the underlying models or agents.

The core philosophy: *Give AI agents exactly what they need, when they need it, no more.*

### The Problem

AI coding agents suffer from context inflation:

- **Context overflow**: Large codebases exceed context windows
- **Irrelevant retrieval**: Agents fetch unrelated code
- **No memory persistence**: Every conversation starts from scratch
- **Knowledge loss**: Decisions made in previous sessions are forgotten

Existing solutions (LangChain, LlamaIndex) are integration frameworks — they connect things but don't optimize context intelligently.

### The Solution

Cogito introduces **four core abstractions**:

1. **Code Fingerprints** — Minimal structural snapshots of projects
2. **Semantic Memory Tags** — Indexed knowledge with zero reading
3. **Decision Logs** — Structured reasoning that survives sessions
4. **Context Compressor** — Intelligent context reduction

---

## 2. Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           COGITO CORE                               │
│                                                                     │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────────┐ │
│  │   Code      │  │  Semantic   │  │      Decision Log           │ │
│  │ Fingerprint │  │   Memory    │  │        Engine               │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────────┘ │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────────┐ │
│  │   Hot/Cold  │  │  Context    │  │    MCP Protocol            │ │
│  │   Zones     │  │ Compressor  │  │      Adapter               │ │
│  └─────────────┘  └─────────────┘  └─────────────────────────────┘ │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
           │                    │                      │
    ┌──────┴──────┐      ┌──────┴──────┐        ┌──────┴──────┐
    │  IDE        │      │   CLI       │        │  REST API   │
    │  Plugins    │      │  Tool       │        │  Server     │
    └─────────────┘      └─────────────┘        └─────────────┘
                               │
                    ┌──────────┴──────────┐
                    │  ANY AI AGENT      │
                    │                    │
                    │  • Claude Code     │
                    │  • GPT-4o/Codex    │
                    │  • Gemini          │
                    │  • DeepSeek        │
                    │  • Qwen            │
                    │  • MiniMax         │
                    │  • Grok            │
                    │  • Cursor          │
                    │  • Copilot         │
                    │  • Aider           │
                    └────────────────────┘
```

---

## 3. Core Modules

### 3.1 Code Fingerprint Generator

**Purpose**: Create minimal structural snapshots of codebases.

```rust
pub struct Fingerprint {
    pub project_id: Uuid,
    pub entry_points: Vec<PathBuf>,
    pub data_flows: Vec<DataFlow>,
    pub hot_zones: Vec<Zone>,
    pub cold_zones: Vec<Zone>,
    pub frozen_zones: Vec<Zone>,
    pub language: Language,
    pub generated_at: DateTime,
}
```

**Algorithm**:
1. Parse project structure (AST for supported languages)
2. Identify entry points (main, index, routes, handlers)
3. Detect data flows between modules
4. Calculate hot/cold zones via git history analysis
5. Generate weighted dependency graph

**Supported Languages**: Rust, TypeScript, JavaScript, Python, Go, Java, C#, Ruby, PHP

### 3.2 Semantic Memory Tags

**Purpose**: Index knowledge without reading code.

```rust
pub struct SemanticTag {
    pub id: Uuid,
    pub project_id: Uuid,
    pub tag: String,           // "auth", "billing", "api"
    pub domain: String,        // "security", "commerce", "network"
    pub paths: Vec<PathBuf>,
    pub confidence: f32,
    pub created_at: DateTime,
}
```

**Auto-tagging Rules**:
- File naming patterns → tag inference
- Directory structure → domain mapping
- Import/export analysis → dependency tags
- Comment extraction → semantic hints

### 3.3 Decision Log Engine

**Purpose**: Capture and retrieve architectural decisions.

```rust
pub struct Decision {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub why: String,
    pub alternatives: Vec<String>,
    pub paths: Vec<PathBuf>,
    pub consequences: Vec<String>,
    pub tags: Vec<String>,
    pub created_at: DateTime,
    pub author: String,
}
```

**Decision Extraction**:
- **Manual**: CLI/API to register decisions
- **Auto**: Git commit analysis + LLM inference
- **Hybrid**: Blame analysis + human confirmation

### 3.4 Context Compressor

**Purpose**: Reduce context size while preserving meaning.

```rust
pub struct CompressedContext {
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub savings_percent: f32,
    pub content: String,
    pub sources: Vec<SourceRef>,
}

pub struct CompressionOptions {
    pub max_tokens: usize,
    pub preserve_imports: bool,
    pub preserve_types: bool,
    pub hot_zones_priority: bool,
}
```

**Compression Strategies**:
1. **Structural**: Remove whitespace, comments
2. **Semantic**: Summarize functions, keep signatures
3. **Relevance**: Remove unused code paths
4. **Hybrid**: Combine based on query intent

### 3.5 Hot/Cold Zones

**Purpose**: Prioritize code by change frequency.

```rust
pub struct Zone {
    pub path: PathBuf,
    pub frequency: ZoneFrequency,
    pub last_modified: DateTime,
    pub change_count: usize,
}

pub enum ZoneFrequency {
    Hot,      // > 5 changes/month
    Warm,     // 2-5 changes/month
    Cold,     // < 2 changes/month
    Frozen,   // no changes in 6 months
}
```

**Detection**: Git history analysis with configurable time windows.

### 3.6 MCP Protocol Adapter

**Purpose**: Connect to any MCP-compatible client.

**Transport**: stdio (subprocess) or HTTP/SSE

**Tools Exposed**:
- `cogito_fingerprint_get` — Get project fingerprint
- `cogito_knowledge_search` — Search semantic tags
- `cogito_decision_find` — Query decisions
- `cogito_context_compress` — Compress context
- `cogito_decision_add` — Register new decision

**Resources**:
- `cogito://fingerprint/{project_id}` — Current fingerprint
- `cogito://tags/{project_id}` — All semantic tags
- `cogito://decisions/{project_id}` — Decision log

---

## 4. Data Models

### Project

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub path: PathBuf,
    pub name: String,
    pub language: Language,
    pub fingerprint: Option<Fingerprint>,
    pub created_at: DateTime,
    pub updated_at: DateTime,
}
```

### Language Detection

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Ruby,
    Php,
    Unknown,
}
```

### Context Request/Response

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextRequest {
    pub query: String,
    pub scope: Scope,
    pub max_tokens: usize,
    pub hot_zones_only: bool,
    pub include_decisions: bool,
    pub include_tags: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextResponse {
    pub compressed: String,
    pub tokens_used: usize,
    pub tokens_saved: usize,
    pub savings_percent: f32,
    pub sources: Vec<SourceRef>,
    pub tags: Vec<SemanticTag>,
    pub decisions: Vec<Decision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Scope {
    Project,
    Directory(PathBuf),
    Files(Vec<PathBuf>),
    HotOnly,
}
```

---

## 5. API Design

### REST Endpoints

```
POST   /api/v1/projects              # Create project
GET    /api/v1/projects/{id}        # Get project
DELETE /api/v1/projects/{id}        # Delete project

POST   /api/v1/projects/{id}/scan    # Generate fingerprint
GET    /api/v1/projects/{id}/fingerprint  # Get current fingerprint

POST   /api/v1/context/compress      # Compress context
GET    /api/v1/context/estimate      # Estimate compression

POST   /api/v1/tags                  # Create tag
GET    /api/v1/tags?project={id}     # List tags
DELETE /api/v1/tags/{id}             # Delete tag

POST   /api/v1/decisions             # Create decision
GET    /api/v1/decisions?project={id}  # List decisions
GET    /api/v1/decisions/search?q={query}  # Search decisions
DELETE /api/v1/decisions/{id}        # Delete decision

GET    /api/v1/zones?project={id}    # Get hot/cold zones
```

### Request/Response Examples

**POST /api/v1/context/compress**
```json
// Request
{
  "project_id": "uuid",
  "content": "...",
  "max_tokens": 8000,
  "query": "how does auth work?",
  "include_decisions": true
}

// Response
{
  "compressed": "...",
  "tokens_used": 7500,
  "tokens_saved": 12500,
  "savings_percent": 62.5,
  "sources": [
    {"path": "src/auth/mod.rs", "type": "file", "relevance": 0.95}
  ],
  "tags": [
    {"tag": "auth", "domain": "security", "confidence": 0.92}
  ],
  "decisions": [
    {
      "title": "JWT with refresh tokens",
      "why": "stateless, scales better",
      "paths": ["src/auth/"]
    }
  ]
}
```

---

## 6. CLI Commands

```bash
# Project Management
cogito init                      # Initialize project in current directory
cogito project list              # List all projects
cogito project delete <id>       # Delete project

# Scanning & Fingerprinting
cogito scan                      # Scan and generate fingerprint
cogito scan --watch              # Watch mode for changes
cogito fingerprint show          # Display current fingerprint

# Context Operations
cogito compress [file]            # Compress context from file or stdin
cogito compress --max-tokens 8000 # Set max tokens
cogito query "question"          # Query knowledge base

# Decision Log
cogito decision add               # Interactive decision creation
cogito decision add --title "..." # Create from arguments
cogito decision list              # List all decisions
cogito decision search "query"   # Search decisions
cogito decision show <id>        # Show decision details

# Semantic Tags
cogito tag add <tag> <paths...>  # Add semantic tag
cogito tag list                   # List all tags
cogito tag search <query>         # Search tags

# Server Mode
cogito serve                      # Start REST API server
cogito serve --mcp               # Start MCP server
cogito serve --port 8080         # Custom port

# Utilities
cogito stats                      # Show token savings stats
cogito export --format json      # Export project data
cogito doctor                    # Diagnose configuration
```

---

## 7. Roadmap

### Phase 1: MVP (Months 1-2)

- [ ] Core Rust engine with workspace setup
- [ ] Code Fingerprint generator (basic)
- [ ] Context Compressor (structural only)
- [ ] CLI tool with basic commands
- [ ] REST API server
- [ ] SQLite storage backend

**Success Criteria**:
- Generates fingerprint for TypeScript/Rust projects
- Compresses context by 30%+ without semantic loss
- CLI functional with `scan`, `compress`, `decision add` commands

### Phase 2: Intelligent (Months 3-4)

- [ ] Semantic Memory Tags (auto-tagging)
- [ ] Hot/Cold zone detection (git history)
- [ ] Decision Log auto-extractor
- [ ] Weighted dependency graph
- [ ] MCP server adapter
- [ ] PostgreSQL optional backend

**Success Criteria**:
- Auto-tags accuracy > 80%
- Hot zone detection within 10% of manual classification
- MCP server connects to Claude Code/Cursor

### Phase 3: Advanced (Months 5-6)

- [ ] Context-on-Demand (lazy loading)
- [ ] LLM-assisted semantic compression
- [ ] Multi-model router
- [ ] Analytics dashboard
- [ ] Plugin system (WASM)

**Success Criteria**:
- Agents request only needed context
- Token savings > 60% with semantic compression
- Plugin ecosystem started

### Phase 4: Ecosystem (Months 7+)

- [ ] VSCode extension
- [ ] Neovim plugin
- [ ] JetBrains plugin
- [ ] CI/CD GitHub Action
- [ ] Collaborative memory sync
- [ ] Enterprise features (SSO, audit logs)

---

## 8. Success Metrics

### Primary Metrics

| Metric | Target | Measurement |
|--------|--------|------------|
| Token Savings | >50% | (original - compressed) / original |
| Context Relevance | >90% | Agent accuracy on compressed vs full |
| Tag Accuracy | >80% | Auto-tag vs manual classification |
| Hot Zone Precision | >90% | Auto vs manual hot zone match |

### Secondary Metrics

| Metric | Target |
|--------|--------|
| Compression Speed | <100ms for 50k tokens |
| Fingerprint Generation | <5s for 10k files |
| Memory Usage | <100MB idle |
| Binary Size | <10MB compressed |

---

## 9. Open Questions

1. **LLM Integration**: Should Cogito call an LLM for semantic compression, or stay model-agnostic?
2. **Sync Protocol**: How should collaborative memory sync work across team members?
3. **Privacy Model**: Should fingerprints/decisions be encrypted at rest?
4. **Versioning**: How to handle project restructuring and migration of knowledge?

## 10. Storage Architecture

Cogito uses **SQLite** as the primary storage backend. The schema is
designed to leverage modern SQLite features:

### Schema

```sql
-- Projects
CREATE TABLE projects (
    id TEXT PRIMARY KEY,           -- UUID v5 of canonical path
    path TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    language TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Decisions with FTS5 + generated columns
CREATE TABLE decisions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    why TEXT NOT NULL,
    alternatives TEXT NOT NULL DEFAULT '[]',
    paths TEXT NOT NULL DEFAULT '[]',
    consequences TEXT NOT NULL DEFAULT '[]',
    tags TEXT NOT NULL DEFAULT '[]',
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,

    -- Generated columns (auto-calculated on write)
    title_lower TEXT GENERATED ALWAYS AS (lower(title)) STORED,
    path_count INTEGER GENERATED ALWAYS AS (json_array_length(paths)) STORED,
    tag_count INTEGER GENERATED ALWAYS AS (json_array_length(tags)) STORED,
    alt_count INTEGER GENERATED ALWAYS AS (json_array_length(alternatives)) STORED,
    cons_count INTEGER GENERATED ALWAYS AS (json_array_length(consequences)) STORED,

    -- Concatenated search text (auto-updated, indexed by FTS5)
    search_text TEXT GENERATED ALWAYS AS (
        lower(coalesce(title, '')) || ' ' ||
        lower(coalesce(why, '')) || ' ' ||
        lower(coalesce(alternatives, '')) || ' ' ||
        lower(coalesce(consequences, '')) || ' ' ||
        lower(coalesce(tags, ''))
    ) STORED
);

CREATE INDEX idx_decisions_project ON decisions(project_id);
CREATE INDEX idx_decisions_created ON decisions(created_at DESC);
CREATE INDEX idx_decisions_path_count ON decisions(path_count) WHERE path_count > 0;
CREATE INDEX idx_decisions_tag_count ON decisions(tag_count) WHERE tag_count > 0;
CREATE INDEX idx_decisions_title_lower ON decisions(title_lower);

-- FTS5 with porter stemming
CREATE VIRTUAL TABLE decisions_fts USING fts5(
    search_text,
    content='decisions',
    content_rowid='rowid',
    tokenize='porter unicode61'
);

-- Triggers keep FTS5 in sync
CREATE TRIGGER decisions_ai AFTER INSERT ON decisions ...;
CREATE TRIGGER decisions_au AFTER UPDATE ON decisions ...;
CREATE TRIGGER decisions_ad AFTER DELETE ON decisions ...;

-- Tags (similar structure with FTS5)
CREATE TABLE tags (...);
CREATE VIRTUAL TABLE tags_fts USING fts5(...);
```

### PRAGMAs

```sql
PRAGMA journal_mode = WAL;             -- concurrent readers
PRAGMA synchronous = NORMAL;          -- balanced safety/speed
PRAGMA temp_store = MEMORY;           -- fast temp tables
PRAGMA mmap_size = 268435456;         -- 256 MB memory-mapped I/O
PRAGMA page_size = 8192;
PRAGMA foreign_keys = ON;             -- FK constraints enforced
PRAGMA optimize;                       -- auto-tune over time
```

### Key Features

| Feature | Benefit |
|---|---|
| **FTS5 + porter** | Stemming: "running" matches "run" |
| **Generated columns** | Auto-calculated `path_count`, `tag_count` |
| **Expression indexes** | Partial indexes (`WHERE path_count > 0`) |
| **Triggers** | FTS5 always in sync, no manual updates |
| **WAL mode** | Readers don't block writers |
| **mmap** | ~Zero-syscall reads |
| **JSONB** | Path arrays stored as binary JSON (planned) |
| **Foreign keys** | Cascading deletes on project removal |

### File location

Default: `<project>/.cogito/state.db`

WAL file: `<project>/.cogito/state.db-wal`
Shared memory: `<project>/.cogito/state.db-shm`

### Migration from JSON

On first scan of a project that has legacy `.cogito/tags.json` and
`.cogito/decisions.json`, Cogito auto-migrates them into SQLite.
Migrated records are re-assigned to the current project's UUID
(determined by the canonical path).

### Measured Performance

See `docs/benchmarks.md` for real numbers. Highlights:

- **Write throughput**: 309x faster than JSON for 1000 decisions
- **File size**: 3x more compact (164 vs 495 bytes/decision)
- **FTS5 stemming**: matches "scalable" → "scalability" automatically
- **No regression** on small N: both backends load in ~4 µs at 100 records

---

## 10. Future Work

- **Context Caching**: Intelligent cache for frequently accessed contexts
- **Learned Patterns**: ML model to predict needed context based on task type
- **Multi-Project Memory**: Cross-project knowledge linking
- **Audit Trail**: Full history of context decisions for compliance
- **A/B Testing**: Compare different compression strategies
