# AGIME Team Server Backend - Comprehensive Documentation

**Project:** agime-team-server (Rust/Axum)
**Location:** `crates/agime-team-server/src/`
**Database:** MongoDB (primary), SQLite (fallback)
**Framework:** Axum 0.8.1, Tokio async runtime

---

## Executive Summary

The AGIME Team Server is a standalone collaboration platform that provides:
- **Centralized team data storage** with MongoDB as the primary database
- **Authentication & Authorization** via API keys and session management
- **Agent execution system** (Phase 1: Chat Track, Phase 2: Mission Track)
- **Real-time task streaming** with SSE (Server-Sent Events)
- **Workspace isolation** for multi-user collaborative sessions
- **AI-powered document analysis, portals, and knowledge management**
- **MCP (Model Context Protocol) integration** for subprocess and platform extensions
- **License and brand customization** support

---

## Project Structure

```
src/
├── main.rs              # Server startup, router setup, middleware configuration
├── state.rs             # AppState: shared application state
├── config.rs            # Configuration management (env vars, defaults)
├── license.rs           # License key generation and brand customization
├── auth/                # Authentication module
│   ├── mod.rs          # Module organization (MongoDB/SQLite versions)
│   ├── api_key.rs      # API key generation and validation
│   ├── middleware_mongo.rs  # Auth middleware (MongoDB)
│   ├── routes_mongo.rs      # Auth routes: register, login, logout, API keys
│   ├── service_mongo.rs     # AuthService: user/key management business logic
│   └── session_mongo.rs     # SessionService: session lifecycle management
└── agent/              # Agent execution module (main domain logic)
    ├── mod.rs                     # Module exports and re-exports
    ├── chat_manager.rs            # ChatManager: tracks active chat sessions (Phase 1)
    ├── chat_executor.rs           # ChatExecutor: multi-turn chat execution
    ├── chat_routes.rs             # Chat API routes (/api/team/agent/chat)
    ├── mission_manager.rs         # MissionManager: tracks active missions (Phase 2)
    ├── mission_executor.rs        # MissionExecutor: multi-step mission execution
    ├── mission_mongo.rs           # Mission domain models and enums
    ├── mission_routes.rs          # Mission API routes (/api/team/agent/mission)
    ├── mission_preflight_tools.rs # Preflight checks before mission execution
    ├── mission_verifier.rs        # Runtime contract verification
    ├── adaptive_executor.rs       # AGE (Adaptive Goal Execution) engine
    ├── session_mongo.rs           # AgentSessionDoc: conversation state persistence
    ├── service_mongo.rs           # AgentService: business logic layer
    ├── executor_mongo.rs          # TaskExecutor: core LLM execution logic
    ├── routes_mongo.rs            # Agent CRUD and task routes
    ├── task_manager.rs            # TaskManager: tracks background tasks
    ├── streamer.rs                # SSE streaming for real-time updates
    ├── runtime.rs                 # Shared bridge pattern utilities
    ├── platform_runner.rs         # In-process platform extension runner
    ├── mcp_connector.rs           # MCP client for subprocess extensions
    ├── provider_factory.rs        # Factory for creating LLM providers
    ├── context_injector.rs        # Document context injection into prompts
    ├── document_tools.rs          # McpClientTrait impl for document operations
    ├── portal_tools.rs            # McpClientTrait impl for portal operations
    ├── developer_tools.rs         # Shell/text editor tools provider
    ├── team_skill_tools.rs        # Team-specific skill tools provider
    ├── ai_describe.rs             # AI description generation service
    ├── document_analysis.rs       # Background document analysis trigger
    ├── smart_log.rs               # Smart log summarization trigger
    ├── extension_installer.rs     # Auto-installer for team extensions
    ├── extension_manager_client.rs # Client for extension manager
    ├── rate_limit.rs              # Rate limiting for API protection
    ├── resource_access.rs         # Resource access control policies
    ├── prompt_profiles.rs         # Portal-specific prompt overlays
    ├── portal_public.rs           # Public portal API routes (no auth)
    └── (many more utility modules...)
```

---

## Core Modules

### 1. main.rs — Server Bootstrap & Router Setup

**Responsibilities:**
- Parse CLI arguments (port override, MCP server, license generation)
- Load environment configuration
- Initialize database (MongoDB or SQLite)
- Setup background cleanup tasks for:
  - Chat sessions (stale session recovery)
  - Missions (orphaned mission recovery)
  - Auth sessions (expired session cleanup)
- Build Axum router with all routes and middleware
- Enable CORS (configurable whitelist or mirror_request for dev)

**Key Functions:**
- `run_server()`: Main async entrypoint
- `build_router()`: Constructs nested route structure
- `shutdown_signal()`: Graceful shutdown on SIGTERM/CTRL+C
- `run_mcp()`: Runs built-in MCP servers (AutoVisualiser, ComputerController, etc.)

**CLI Subcommands:**
- `--port N`: Override listen port
- `mcp SERVER`: Run MCP server over stdio
- `generate-license`: Create signed license keys
- `generate-keypair`: Generate Ed25519 keypair
- `machine-id`: Print machine fingerprint

**Startup Lifecycle:**
1. Load .env file
2. Generate unique server instance ID (for orphaned mission recovery)
3. Initialize tracing
4. Load config from environment
5. Setup TLS backend (rustls or native-tls)
6. Connect to database
7. Create auth middleware with rate limiters & login guard
8. Spawn background cleanup tasks
9. Build router with conditional MongoDB/SQLite routes
10. Start TCP listener with graceful shutdown

---

### 2. state.rs — Application State

```rust
pub struct AppState {
    pub db: DatabaseBackend,            // MongoDB | SQLite
    pub config: Config,                 // Server config
    pub register_limiter: Option<Arc<RateLimiter>>,
    pub login_limiter: Option<Arc<RateLimiter>>,
    pub login_guard: Option<Arc<LoginGuard>>,
    pub brand_config: Arc<RwLock<BrandConfig>>,
}
```

**Methods:**
- `require_mongodb()`: Get MongoDB or return 503 error
- `require_sqlite()`: Get SQLite or return 503 error

**Key Pattern:** Using `State` extractor in route handlers to access shared state.

---

### 3. config.rs — Configuration Management

**Environment Variables (with defaults):**
- `DATABASE_TYPE`: "mongodb" (default) or "sqlite"
- `TEAM_SERVER_HOST`: 0.0.0.0 (default)
- `TEAM_SERVER_PORT`: 8080 (default)
- `DATABASE_URL`: mongodb://localhost:27017
- `DATABASE_NAME`: agime_team
- `CORS_ALLOWED_ORIGINS`: comma-separated list or mirror_request mode
- `REGISTRATION_MODE`: "open" (default), "approval", or "disabled"
- `LOGIN_MAX_FAILURES`: 5
- `LOGIN_LOCKOUT_MINUTES`: 15
- `SECURE_COOKIES`: false (default)
- `BASE_URL`: Optional base URL for invite links
- `PORTAL_TEST_BASE_URL`: Optional test URL for portal links
- `WORKSPACE_ROOT`: ./data/workspaces (default)
- `TEAM_AGENT_RESOURCE_MODE`: "explicit" | "auto"
- `TEAM_AGENT_SKILL_MODE`: "assigned" | "on_demand"
- `TEAM_AGENT_AUTO_EXTENSION_POLICY`: "reviewed_only" | "all"
- `AI_DESCRIBE_API_KEY`, `AI_DESCRIBE_MODEL`, `AI_DESCRIBE_API_URL`, `AI_DESCRIBE_API_FORMAT`
- `LICENSE_KEY`: Optional for brand customization

**Validation:**
- Registration mode must be one of: "open", "approval", "disabled"
- API URLs must start with http:// or https://
- Priority must be 0-100

---

### 4. Authentication Module (auth/)

#### Structs & Enums

**User** (service_mongo.rs):
- `id`: MongoDB ObjectId
- `user_id`: String (unique)
- `email`: String
- `display_name`: String
- `password_hash`: Optional (for password auth)
- `role`: "user" (default) | "admin"
- `created_at`, `last_login_at`: DateTime<Utc>
- `is_active`: bool

**ApiKeyDoc**:
- `key_id`: Unique identifier
- `user_id`: Owner
- `key_prefix`: First 8 chars of key (displayed in UI)
- `key_hash`: Argon2 hash for verification
- `last_used_at`: Timestamp for auditing
- `expires_at`: Optional expiration

**Session** (session_mongo.rs):
- Stored in MongoDB sessions collection
- TTL: 7 days (max_age in cookie)
- Sliding window: renew if remaining < 2 hours

#### Routes

**Public Routes:**
- `POST /api/auth/register`: Create new account (with rate limiting)
- `POST /api/auth/login`: Start session (returns cookie)
- `POST /api/auth/login/password`: Password-based login
- `GET /api/auth/session`: Get current session info
- `POST /api/auth/logout`: Invalidate session

**Protected Routes (require auth):**
- `GET /api/auth/me`: Get current user info
- `GET /api/auth/keys`: List API keys for current user
- `POST /api/auth/keys`: Create new API key
- `DELETE /api/auth/keys/{key_id}`: Revoke key
- `POST /api/auth/deactivate`: Deactivate account
- `POST /api/auth/change-password`: Change password

**Admin Routes (require admin role):**
- User management endpoints

#### Middleware (middleware_mongo.rs)

**UserContext Extractor:**
```rust
pub struct UserContext {
    pub user_id: String,
    pub email: String,
    pub role: String,
}
```

Applied to all protected routes. Falls back to API key validation if no session cookie.

#### Service Layer (service_mongo.rs)

**AuthService Methods:**
- `register_user()`: Create new user with validation
- `login()`: Authenticate and create session
- `login_with_password()`: Password-based authentication
- `logout()`: Invalidate session
- `create_api_key()`: Generate new API key
- `revoke_api_key()`: Delete API key
- `validate_api_key()`: Check key hash against prefix
- `get_user()`: Fetch user by ID
- `list_api_keys()`: Get all keys for user
- `is_team_admin()`: Check admin role in team context

**Password Hashing:** Argon2 with salt

**API Key Format:** Prefix (8 chars) + random secret (32 chars) = 40 chars total

---

### 5. Agent Module (agent/)

The agent module is the heart of the team server, divided into two main execution tracks:

#### **Phase 1: Chat Track** (chat_routes.rs, chat_manager.rs, chat_executor.rs)

Direct multi-turn chat sessions that bypass the formal Task system.

**Session Flow:**
1. User creates chat session (CreateChatSessionRequest)
2. AgentSessionDoc persists conversation state (messages, tokens, extensions)
3. Each message sends to Agent via TaskExecutor's "bridge pattern"
4. ChatManager tracks active sessions with broadcast channels
5. Real-time events streamed via SSE
6. Session can be archived when done

**ChatManager:**
- Tracks active sessions by session_id
- Maintains event history (400 max events per session)
- Persists events to MongoDB in background (batched, 128 per batch, 25ms flush)
- Cleanup task removes stale sessions (default: 4 hours inactivity)

**ChatExecutor:**
- Reuses TaskExecutor via "bridge pattern"
- Creates temporary task, approves it, executes, cleans up
- Bridges stream events from TaskManager to ChatManager broadcast
- Manages workspace isolation per session

**Routes:**
- `POST /api/team/agent/chat/sessions`: Create session
- `GET /api/team/agent/chat/sessions`: List sessions (with pagination)
- `GET /api/team/agent/chat/sessions/{id}`: Get session details
- `POST /api/team/agent/chat/sessions/{id}/send`: Send message
- `GET /api/team/agent/chat/sessions/{id}/stream`: SSE stream
- `DELETE /api/team/agent/chat/sessions/{id}`: Archive session

#### **Phase 2: Mission Track** (mission_routes.rs, mission_manager.rs, mission_executor.rs)

Multi-step autonomous tasks with approval workflows and runtime contracts.

**Mission Lifecycle:**
1. Draft → Planning → Planned → Running → Completed/Failed/Cancelled
2. Steps: Pending → AwaitingApproval → Running → Completed/Failed/Skipped
3. Execution profiles: Auto, Fast, Full
4. Approval policies: Auto, Checkpoint, Manual
5. Token budget and artifact tracking

**MissionManager:**
- Tracks active missions by mission_id
- Similar event persistence and cleanup as ChatManager
- Orphaned mission recovery on startup (missions stuck in Running/Planning)
- Last activity tracking per mission

**MissionDoc** (mission_mongo.rs):
```rust
pub struct MissionDoc {
    pub mission_id: String,
    pub team_id: String,
    pub creator_id: String,
    pub goal: String,                        // Primary objective
    pub context: Option<String>,             // Background context
    pub status: MissionStatus,               // Draft → Planning → Planned → Running → ...
    pub execution_mode: ExecutionMode,       // Sequential | Adaptive
    pub execution_profile: ExecutionProfile, // Auto | Fast | Full
    pub approval_policy: ApprovalPolicy,     // Auto | Checkpoint | Manual
    pub steps: Vec<MissionStep>,             // Execution plan
    pub artifacts: Vec<MissionArtifactDoc>, // Generated outputs
    pub token_budget: i32,                   // 0 = no limit
    pub step_timeout_seconds: Option<u64>,
    pub step_max_retries: Option<u32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // ... many more fields
}

pub enum MissionStatus {
    Draft, Planning, Planned, Running, Paused, Completed, Failed, Cancelled,
}

pub enum ApprovalPolicy {
    Auto,        // Execute without prompts
    Checkpoint,  // Pause after plan, before execution
    Manual,      // Pause before each step
}

pub enum ExecutionMode {
    Sequential,  // Standard step-by-step
    Adaptive,    // Adaptive Goal Execution (AGE)
}
```

**MissionStep:**
- `title`: Human-readable description
- `status`: Pending | AwaitingApproval | Running | Completed | Failed | Skipped
- `action_input`: Task content to execute
- `approval_required`: bool
- `artifacts_required`: Vec<ArtifactSpec>
- `completion_checks`: Vec<CompletionCheck>
- `retries`: Retry tracking

**Routes:**
- `POST /api/team/agent/mission/missions`: Create mission
- `GET /api/team/agent/mission/missions`: List missions
- `GET /api/team/agent/mission/missions/{id}`: Get mission details
- `POST /api/team/agent/mission/missions/{id}/execute`: Start execution
- `POST /api/team/agent/mission/missions/{id}/steps/{step_id}/approve`: Approve step
- `POST /api/team/agent/mission/missions/{id}/steps/{step_id}/reject`: Reject step
- `DELETE /api/team/agent/mission/missions/{id}`: Cancel mission
- `GET /api/team/agent/mission/missions/{id}/stream`: SSE stream

---

#### Task Executor & Core LLM Integration (executor_mongo.rs)

**TaskExecutor** orchestrates the full LLM execution pipeline:

**Execution Flow:**
1. Load agent config (API key, model, extensions, skills)
2. Create provider (via provider_factory)
3. Initialize extensions:
   - MCP connections (subprocess or StreamableHttp)
   - Platform extensions (in-process)
4. Setup tools from all extensions
5. Create/restore agent session for context
6. Execute multi-turn conversation with LLM
7. Handle tool calls via McpConnector or PlatformExtensionRunner
8. Track token usage and compaction
9. Stream events in real-time
10. Return final result

**Key Responsibilities:**
- **Extension Loading:**
  - Load MCP servers (stdio child process or HTTP)
  - Load platform extensions (Skills, Team, Todo, Developer, Document Tools, Portal Tools)
  - Collect tool definitions from all sources
  - Prefix tool names to avoid collisions (e.g., "developer__text_editor")

- **Provider Selection:**
  - Supports multiple LLM APIs: Anthropic, OpenAI, Volcengine
  - Uses factory to create provider with correct API format
  - Handles request/response serialization differences

- **Context Management:**
  - Loads conversation history from AgentSessionDoc
  - Injects system prompt + user custom instructions
  - Implements context compaction (when context exceeds threshold)
  - Tracks token counts (input/output)

- **Tool Execution:**
  - Dispatches tool calls to correct extension (by prefix)
  - Handles concurrent tool calls
  - Enforces timeout per tool
  - Captures multi-content tool results (text + images)
  - Auto-retries failed tools (configurable)

- **Error Recovery:**
  - Implements retry config (max retries, timeout, backoff)
  - Success checks (shell commands or output validation)
  - On failure: can escalate to user or abandon

**Tool Integration Patterns:**

1. **MCP Servers** (McpConnector):
   - Child process over stdio (stdio transport)
   - HTTP with streaming (StreamableHttpClient)
   - Supports MCP Sampling (LLM calls via agent API key)
   - Long-running task support with polling

2. **Platform Extensions** (PlatformExtensionRunner):
   - In-process implementations of McpClientTrait
   - DocumentTools: read/create/search/list documents
   - PortalTools: create/update/publish portals
   - DeveloperTools: shell commands, text editor
   - TeamSkillTools: call team-specific skills
   - MissionPreflightTools: preflight validation

---

### 6. Session Management (session_mongo.rs)

**AgentSessionDoc** persists conversation state:
- `session_id`: Unique identifier
- `team_id`, `agent_id`, `user_id`: Context
- `messages_json`: Serialized conversation (Vec<Message>)
- `message_count`, `total_tokens`, `input_tokens`, `output_tokens`
- `compaction_count`: Number of times context was compacted
- `disabled_extensions`, `enabled_extensions`: User overrides
- `status`: "active" | "archived"
- `workspace_path`: Isolated workspace for this session
- `extra_instructions`: Injected into system prompt
- `allowed_extensions`, `allowed_skill_ids`: Optional allowlist
- `max_turns`: Optional turn cap
- `tool_timeout_seconds`: Tool execution timeout
- `require_final_report`: For portal sessions
- `portal_restricted`: Portal visitor restriction flag
- `document_access_mode`: "full" | "read_only" | "co_edit_draft" | "controlled_write"

---

### 7. Service Layer (service_mongo.rs)

**AgentService** implements high-level business logic:

**Agent CRUD:**
- `create_agent()`: Create new agent with validation
- `get_agent()`: Fetch agent by ID
- `update_agent()`: Modify agent config
- `delete_agent()`: Remove agent
- `list_agents()`: Paginated agent list

**Task Management:**
- `submit_task()`: Create task from request
- `approve_task()`: Mark task as approved
- `reject_task()`: Reject task with reason
- `cancel_task()`: Cancel running task
- `list_tasks()`: Paginated task list
- `get_task()`: Fetch task details
- `get_task_results()`: Fetch task results

**Session Management:**
- `create_session()`: Create new conversation session
- `get_session()`: Fetch session
- `list_sessions()`: Paginated session list
- `archive_session()`: Archive session
- `save_chat_stream_events()`: Persist chat events in batch
- `reset_stuck_processing()`: Recovery for stuck sessions

**Mission Management:**
- `create_mission()`: Create new mission
- `get_mission()`: Fetch mission
- `list_missions()`: Paginated list
- `update_mission_status()`: Change mission status
- `update_mission_step()`: Update step status/output
- `save_mission_stream_events()`: Persist mission events

**Document/Extension Management:**
- `get_agent_extensions()`: List extensions
- `update_agent_extensions()`: Modify extension config
- `reload_agent_extensions()`: Refresh from sources
- `get_available_skills()`: List all skills

**Admin Utilities:**
- `is_team_admin()`: Check admin role
- `backfill_session_source_and_visibility()`: Data migration
- `recover_orphaned_missions()`: Recovery routine

---

### 8. Real-Time Streaming (streamer.rs, task_manager.rs)

**StreamEvent** enum (task_manager.rs):
```rust
pub enum StreamEvent {
    Status { status: String },
    Text { content: String },
    Thinking { content: String },
    ToolCall { name: String, id: String },
    ToolResult { id: String, success: bool, content: String, ... },
    WorkspaceChanged { tool_name: String },
    Turn { current: usize, max: usize },
    Compaction { strategy: String, before_tokens: usize, after_tokens: usize },
    SessionId { session_id: String },
    Done { status: String, error: Option<String> },
    // AGE events:
    GoalStart { goal_id: String, title: String, depth: u32 },
    GoalComplete { goal_id: String, signal: String },
    Pivot { goal_id: String, from_approach: String, to_approach: String, learnings: String },
    GoalAbandoned { goal_id: String, reason: String },
}
```

**Streaming Endpoints** (SSE format):
- `GET /api/team/agent/tasks/{task_id}/stream`: Task result streaming
- `GET /api/team/agent/chat/sessions/{id}/stream`: Chat message streaming
- `GET /api/team/agent/mission/missions/{id}/stream`: Mission execution streaming

**Implementation:**
- SSE with event type and JSON data
- Supports `last_event_id` query param for recovery
- Event history maintained in TaskManager/ChatManager/MissionManager
- Clients can resubscribe and resume from last known event

---

### 9. Platform Extensions (platform_runner.rs, developer_tools.rs, document_tools.rs, portal_tools.rs)

**PlatformExtensionRunner** loads built-in extensions:

1. **DocumentTools**:
   - `create_document`: Create new document
   - `read_document`: Fetch document by ID
   - `search_documents`: Full-text search
   - `list_documents`: Paginated list
   - `update_document`: Modify document
   - `delete_document`: Remove document
   - Access control: read_only, co_edit_draft, controlled_write, full

2. **PortalTools**:
   - `create_portal`: Create new portal with project folder
   - `update_portal`: Modify portal config
   - `publish_portal`: Make portal public
   - `list_portals`: Paginated list
   - Returns project_path for building

3. **DeveloperTools**:
   - `text_editor`: Create/read/write files
   - `shell`: Execute shell commands
   - Path restrictions for workspace isolation
   - Multi-platform support (Windows/Unix)

4. **TeamSkillTools** (if configured):
   - Dynamic tool loading from team skill definitions
   - Delegates to skill execution system

5. **MissionPreflightTools** (mission execution):
   - `mission_preflight__preflight`: Validate mission feasibility
   - `mission_preflight__verify_contract`: Verify runtime contract

---

### 10. MCP Connector (mcp_connector.rs)

**McpConnector** manages MCP server connections:

**Supported Transports:**
- **Child Process (stdio)**: Launch subprocess, communicate via stdin/stdout
- **StreamableHttp**: HTTP-based MCP transport with streaming

**Features:**
- Tool definition collection and caching (with TTL)
- Tool call execution with timeout
- Multi-content tool result support (text + images)
- Long-running task support with polling
- MCP Sampling integration (LLM calls via agent API)
- Elicitation bridge for user prompts

**Tool Call Flow:**
1. Collect tools from all MCP servers
2. Agent calls tool (e.g., "developer__text_editor")
3. McpConnector routes to correct server
4. Server executes and returns result
5. Agent incorporates result into conversation

---

### 11. Document Tools & Context Injection (document_tools.rs, context_injector.rs)

**DocumentToolsProvider** implements McpClientTrait for document operations:

**Methods:**
- `create_document()`: Create with auto-generated ID, optional folder
- `read_document()`: Fetch by ID
- `search_documents()`: Full-text search with filters
- `list_documents()`: Paginated with sorting
- `update_document()`: Modify content, title, metadata
- `delete_document()`: Remove document
- `attach_document_to_chat()`: Link doc to session for context

**Access Control:**
- `write_mode`: Full, ReadOnly, CoEditDraft, ControlledWrite
- `allowed_document_ids`: Optional whitelist (for portal sessions)
- `restrict_to_allowed_documents`: Enforce whitelist

**DocumentContextInjector** (context_injector.rs):
- Automatically injects referenced documents into system prompt
- Extracts document IDs from session context
- Limits to configured max documents
- Handles metadata and lineage tracking

---

### 12. Portal System (portal_tools.rs, portal_public.rs)

**PortalToolsProvider** enables agents to create/manage portals:

**Portal Creation:**
- Agent calls `create_portal` with name, description, output_form
- Server creates MongoDB Portal document
- Allocates workspace folder (./data/workspaces/teams/{team_id}/portals/{portal_id}/)
- Returns `project_path` for agent to populate
- Generates public URL for sharing

**Portal Types:**
- `website`: Full HTML website
- `widget`: Embeddable chat widget
- `agent_only`: Agent execution only (no UI)

**Features:**
- Embedded agent chat (optional)
- Multiple agent assignment (coding vs service agents)
- Document binding for agent context
- Visitor restrictions and access modes
- Custom system prompt and welcome message
- Skill/extension allowlists

**portal_public.rs** routes (no auth required):
- `GET /api/portal/:{slug}`: Fetch portal by slug
- `POST /api/portal/{slug}/chat/sessions`: Create visitor session
- `POST /api/portal/{slug}/chat/sessions/{id}/send`: Send message
- `GET /api/portal/{slug}/chat/stream`: Stream events

---

### 13. Adaptive Goal Execution (AGE) (adaptive_executor.rs, mission_executor.rs, mission_verifier.rs)

**Adaptive Goal Execution** provides intelligent mission planning and execution:

**ExecutionMode:**
- **Sequential**: Standard step-by-step linear execution
- **Adaptive**: Goal-tree based execution with pivot protocol

**Key Features:**

1. **Goal-Tree Planning:**
   - Agent generates execution plan from mission goal
   - Plan includes 2-10 steps with dependencies
   - Steps have required artifacts and completion checks

2. **Progress Evaluation:**
   - After each step, verify if goal is achieved
   - Check completion criteria (shell commands, output validation)
   - Decide: Continue → Next Step, Pivot → New Approach, Abandon → Stop

3. **Pivot Protocol:**
   - Max 3 pivots per goal, 15 total per mission
   - When step fails, agent decides whether to retry or pivot
   - Pivot: switch approach with learnings from failure
   - Abandon: give up with reason

4. **Timeout Management:**
   - Default goal execution: 20 minutes
   - Planning phase: 5 minutes
   - Configurable per mission
   - Graceful cancellation with cleanup window

5. **Token Budget:**
   - Optional cap on tokens per mission
   - Enforce budget across all steps
   - Cancel mission if budget exceeded

**AdaptiveExecutor** (adaptive_executor.rs):
- Orchestrates goal-tree execution
- Reuses MissionManager and TaskExecutor
- Implements pivot decision logic
- Tracks completion signals and learnings

**MissionVerifier** (mission_verifier.rs):
- Verifies runtime contracts (task requirements)
- Checks artifact existence
- Validates completion criteria
- Supports before/after verification phases

---

### 14. Database Integration

#### MongoDB (Primary)

**Collections:**
- `users`: User accounts
- `api_keys`: API key documents
- `sessions`: Auth sessions
- `agent_tasks`: Tasks submitted to agents
- `agent_task_results`: Task output/results
- `chat_sessions`: Agent conversation sessions (Phase 1)
- `chat_stream_events`: Event history for chats
- `missions`: Mission documents (Phase 2)
- `mission_stream_events`: Event history for missions
- `team_agents`: Agent configurations
- `agent_extensions`: Extension configs
- `agent_skills`: Team skill configs
- `documents`: Knowledge base documents
- `portals`: Portal configurations
- ... and many more

**Indexes:** Automatically created on startup via `ensure_*_indexes()` methods

**Data Models:**
- Use BSON DateTime with chrono compatibility
- ObjectId for _id fields
- Custom serde serializers for datetime options
- Document validation in service layer

#### SQLite (Fallback)

**Usage:** Same schema as MongoDB, migrated via sqlx
**Migrations:** Located at `../agime-team/src/migrations`
**Limitation:** Some MongoDB-specific features (transactions, change streams) unavailable

---

### 15. API Routes Overview

**Authentication:**
- `POST /api/auth/register`
- `POST /api/auth/login`
- `GET /api/auth/session`
- `POST /api/auth/logout`
- `GET /api/auth/me` (protected)
- `GET /api/auth/keys` (protected)
- `POST /api/auth/keys` (protected)
- `DELETE /api/auth/keys/{key_id}` (protected)

**Brand & License:**
- `GET /api/brand/config`
- `POST /api/brand/activate`
- `GET /api/brand/overrides` (protected, licensed)
- `PUT /api/brand/overrides` (protected, licensed)

**Team Data** (via agime-team crate):
- `GET /api/team/...` (various team management endpoints)

**Agents:**
- `POST /api/team/agent/agents` (protected)
- `GET /api/team/agent/agents` (protected)
- `GET /api/team/agent/agents/{id}` (protected)
- `PUT /api/team/agent/agents/{id}` (protected)
- `DELETE /api/team/agent/agents/{id}` (protected)

**Tasks:**
- `POST /api/team/agent/tasks` (protected)
- `GET /api/team/agent/tasks` (protected)
- `GET /api/team/agent/tasks/{id}` (protected)
- `POST /api/team/agent/tasks/{id}/approve` (protected)
- `POST /api/team/agent/tasks/{id}/reject` (protected)
- `GET /api/team/agent/tasks/{id}/stream` (protected, SSE)

**Chat Sessions** (Phase 1):
- `POST /api/team/agent/chat/sessions` (protected)
- `GET /api/team/agent/chat/sessions` (protected)
- `GET /api/team/agent/chat/sessions/{id}` (protected)
- `POST /api/team/agent/chat/sessions/{id}/send` (protected)
- `GET /api/team/agent/chat/sessions/{id}/stream` (protected, SSE)

**Missions** (Phase 2):
- `POST /api/team/agent/mission/missions` (protected)
- `GET /api/team/agent/mission/missions` (protected)
- `GET /api/team/agent/mission/missions/{id}` (protected)
- `POST /api/team/agent/mission/missions/{id}/execute` (protected)
- `GET /api/team/agent/mission/missions/{id}/stream` (protected, SSE)

**Portals:**
- `GET /api/portal/{slug}` (public)
- `POST /api/portal/{slug}/chat/sessions` (public)
- `GET /api/portal/{slug}/chat/stream` (public, SSE)

---

### 16. Middleware & Interceptors

**CORS Layer:**
- Configurable allowed origins or mirror_request for dev
- Allows: GET, POST, PUT, DELETE, OPTIONS
- Allows credentials (cookies)

**Auth Middleware:**
- Extracts session cookie or X-Authorization API key header
- Validates and creates UserContext
- Injects into Extension for route handlers
- Rate limiting on login/register endpoints

**Tracing:**
- Tower HTTP trace layer for request/response logging
- Structured logging via tracing crate
- JSON output support

**Body Limit:**
- 16MB max for request bodies (for package uploads)

---

### 17. Workspace Isolation & Multi-Tenancy

**Workspace Structure:**
```
./data/workspaces/
├── teams/
│   └── {team_id}/
│       ├── chat_sessions/
│       │   └── {session_id}/
│       │       ├── files/
│       │       └── artifacts/
│       ├── missions/
│       │   └── {mission_id}/
│       │       ├── steps/
│       │       └── artifacts/
│       └── portals/
│           └── {portal_id}/
│               └── project/
```

**Benefits:**
- Prevents cross-session file access
- Supports concurrent executions
- Easy cleanup on session/mission end
- Per-workspace `.bash_env` for environment

**Environment Variables:**
- `WORKSPACE_ROOT`: Root directory (default: ./data/workspaces)
- `PORTAL_BASE_URL`: Public URL for portal links
- Set automatically during startup

---

### 18. Error Handling Patterns

**HTTP Error Responses:**
```rust
(StatusCode, Json<ErrorResponse>)
```

**Common Codes:**
- 400: Bad Request (validation error)
- 401: Unauthorized (no/invalid auth)
- 403: Forbidden (insufficient permissions)
- 404: Not Found (resource doesn't exist)
- 429: Too Many Requests (rate limited)
- 500: Internal Server Error
- 503: Service Unavailable (DB down)

**Database Errors:**
- Wrapped in anyhow::Result
- Propagated to route handlers
- Logged via tracing

**Validation Errors:**
- Checked in service layer
- Returned as custom error types
- Converted to HTTP responses in routes

---

### 19. Rate Limiting

**RateLimiter** (rate_limit.rs):
- Tracks requests per IP
- Configurable: max requests, time window
- Used for:
  - Registration: 5 requests per 3600 seconds
  - Login: 10 requests per 60 seconds

**LoginGuard:**
- Tracks failed login attempts
- Locks account after N failures (default: 5)
- Lockout duration: configurable (default: 15 minutes)

---

### 20. Configuration & Extensibility

**Agent Configuration** (TeamAgent model):
- `name`: Agent display name
- `api_key`: LLM API key (user-specific or team-shared)
- `api_url`: LLM endpoint
- `model`: Model name (e.g., "claude-3-sonnet")
- `api_format`: "anthropic", "openai", "volcengine"
- `extensions`: List of AgentExtensionConfig
- `skills`: List of AgentSkillConfig (team skills)
- `priority`: Execution priority (0-100)
- `description`: User-friendly description
- `is_system`: bool (system agents can't be deleted)
- `created_by`: User ID
- `created_at`: Timestamp

**Extension Configuration:**
- **Builtin** (docker/platform): name, enabled
- **Custom**: uri_or_cmd, args, envs, enabled, source

**Skill Configuration:**
- `skill_id`: Unique identifier
- `name`: Display name
- `provider`: LLM provider for this skill
- `instructions`: Prompt/behavior guide
- `tools`: Available tools for skill
- `enabled`: bool

---

### 21. Key Design Patterns

#### Bridge Pattern (executor → platform extensions)
- TaskExecutor creates temp task
- Task registered in TaskManager
- Events bridged to ChatManager/MissionManager
- Task executed via TaskExecutor
- Cleanup happens in outer executor

#### Factory Pattern (provider_factory)
- Create provider based on api_format
- Supports: Anthropic, OpenAI, Volcengine
- Handles API differences transparently

#### Event Sourcing (StreamEvent + Persistence)
- All significant events streamed in real-time
- Events persisted to MongoDB for replay
- Supports subscription with last_event_id

#### Service Layer Architecture
- Routes → Service → MongoDB/Extensions
- Business logic centralized in Service
- Validation in service, not routes
- Consistent error handling

#### Async Streaming (SSE)
- Real-time updates without polling
- Event history for recovery
- Browser-native support

---

### 22. Background Tasks & Cleanup

**Startup Tasks:**
- Reset stuck chat sessions (is_processing flag)
- Recover orphaned missions

**Recurring Tasks:**
- Chat cleanup (every 60 seconds, remove stale sessions >4 hours)
- Mission cleanup (every 120 seconds, remove stale missions)
- Auth session cleanup (every 600 seconds, remove expired sessions)
- Stale analysis cleanup (every 300 seconds, cancel pending AI analyses)

**Cleanup Intervals:**
All configurable via environment variables (with validation)

---

### 23. Logging & Monitoring

**Tracing Setup** (main.rs):
```rust
tracing_subscriber::registry()
    .with(tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "agime_team_server=info,tower_http=debug".into()))
    .with(tracing_subscriber::fmt::layer())
    .init()
```

**Log Levels:**
- `INFO`: Server startup, connections, major operations
- `WARN`: Recoverable errors, stale cleanup, stuck sessions
- `DEBUG`: HTTP requests/responses
- `ERROR`: Unrecoverable errors

**Key Logs:**
- Server startup/shutdown
- Database connection
- Chat/mission session lifecycle
- Task execution progress
- Extension loading
- Rate limit hits
- Cleanup operations

---

### 24. Security Considerations

**Authentication:**
- Session cookies with HttpOnly, SameSite=Lax
- Optional Secure flag (prod: true)
- API key format: 8-char prefix + 32-char secret
- Key hash stored (Argon2), only prefix displayed

**Authorization:**
- Role-based (user/admin)
- Team-scoped access
- Creator/admin checks for mutations
- Resource-level access control

**Input Validation:**
- Name length (1-100 chars)
- URL format (http/https)
- Priority range (0-100)
- Registration mode enum check
- Document write mode enforcement

**CORS:**
- Configurable whitelist
- Default: mirror_request (dev mode)
- Prod: explicit origin list

**Secrets:**
- API keys stored hashed
- License keys validated
- Env vars redacted in logs

---

### 25. Notable File List

**Core Files:**
- `src/main.rs` (977 lines): Server bootstrap, router setup
- `src/state.rs` (97 lines): Shared state
- `src/config.rs` (356 lines): Configuration management
- `src/license.rs`: License signing and brand customization

**Auth Module:**
- `src/auth/mod.rs`: Module organization
- `src/auth/service_mongo.rs` (~1000 lines): User/key management
- `src/auth/routes_mongo.rs` (~500 lines): Auth endpoints
- `src/auth/middleware_mongo.rs`: Auth middleware
- `src/auth/session_mongo.rs`: Session management
- `src/auth/api_key.rs`: Key generation/validation

**Agent Module (Chat Track - Phase 1):**
- `src/agent/chat_manager.rs` (~300 lines): Session tracking
- `src/agent/chat_executor.rs` (~400 lines): Chat execution
- `src/agent/chat_routes.rs` (~800 lines): Chat endpoints

**Agent Module (Mission Track - Phase 2):**
- `src/agent/mission_manager.rs` (~300 lines): Mission tracking
- `src/agent/mission_executor.rs` (~1500 lines): Mission execution
- `src/agent/mission_mongo.rs` (~1000 lines): Mission models
- `src/agent/mission_routes.rs` (~800 lines): Mission endpoints
- `src/agent/adaptive_executor.rs` (~800 lines): AGE engine
- `src/agent/mission_verifier.rs`: Contract verification

**Core Execution:**
- `src/agent/executor_mongo.rs` (~2000 lines): TaskExecutor
- `src/agent/service_mongo.rs` (~2000 lines): AgentService
- `src/agent/session_mongo.rs` (~500 lines): Session models
- `src/agent/routes_mongo.rs` (~600 lines): Agent CRUD routes

**Real-Time & Streaming:**
- `src/agent/task_manager.rs` (~300 lines): Task tracking
- `src/agent/streamer.rs`: SSE streaming
- `src/agent/chat_manager.rs`: Chat event persistence
- `src/agent/mission_manager.rs`: Mission event persistence

**Extensions & Tools:**
- `src/agent/mcp_connector.rs` (~1000 lines): MCP client
- `src/agent/platform_runner.rs` (~600 lines): Platform extensions
- `src/agent/document_tools.rs` (~800 lines): Document operations
- `src/agent/portal_tools.rs` (~800 lines): Portal operations
- `src/agent/developer_tools.rs`: Shell/editor tools
- `src/agent/team_skill_tools.rs`: Team skill integration

**Utilities:**
- `src/agent/context_injector.rs`: Document context injection
- `src/agent/runtime.rs`: Shared bridge utilities
- `src/agent/provider_factory.rs`: LLM provider factory
- `src/agent/ai_describe.rs` (~400 lines): AI description service
- `src/agent/rate_limit.rs`: Rate limiting
- `src/agent/resource_access.rs`: Resource ACL
- `src/agent/prompt_profiles.rs`: Portal prompt customization
- `src/agent/portal_public.rs`: Public portal routes
- `src/agent/extension_installer.rs`: Auto-install extensions
- `src/agent/extension_manager_client.rs`: Extension manager client
- `src/agent/mission_preflight_tools.rs`: Preflight validation
- `src/agent/document_analysis.rs`: Async document analysis
- `src/agent/smart_log.rs`: Log summarization trigger

---

## Key Dependencies

**Web Framework:**
- `axum 0.8.1`: HTTP server, routing, extractors
- `tower 0.5`: Middleware, services
- `tower-http 0.5`: CORS, tracing, static files
- `tokio 1`: Async runtime

**Database:**
- `mongodb 2.8`: MongoDB driver (primary)
- `sqlx 0.7`: SQLite driver (fallback)
- `bson 2.9`: BSON serialization

**Serialization:**
- `serde 1`: Serialization framework
- `serde_json 1`: JSON support
- `toml 0.8`: TOML parsing

**Security & Crypto:**
- `argon2 0.5`: Password hashing
- `sha2 0.10`: SHA hashing
- `ed25519-dalek 2`: Digital signatures
- `base64 0.22`: Base64 encoding

**Async & Streams:**
- `tokio 1`: Async runtime
- `futures 0.3`: Stream utilities
- `async-stream 0.3`: Stream macros
- `tokio-util 0.7`: Sync primitives (CancellationToken)
- `async-trait 0.1`: Async trait support

**MCP & Extensions:**
- `rmcp`: Model Context Protocol client
  - Features: client, child-process, streamable-http transports

**Utilities:**
- `uuid 1`: UUID generation
- `chrono 0.4`: Date/time
- `tracing 0.1`: Structured logging
- `tracing-subscriber 0.3`: Logging sink
- `anyhow 1`: Error handling
- `thiserror 1`: Error types
- `clap 4`: CLI argument parsing
- `dotenvy 0.15`: .env file loading
- `dirs 5`: Directory paths
- `reqwest 0.12`: HTTP client
- `pulldown-cmark 0.10`: Markdown rendering
- `mime_guess 2`: MIME type detection
- `rustls`: TLS backend
- `winreg 0.55`: Windows registry (proxy detection)

---

## Conclusion

The AGIME Team Server is a comprehensive platform for AI-driven team collaboration, combining:
1. **Robust authentication** with rate limiting and API keys
2. **Two-phase execution system** (Chat for multi-turn, Missions for multi-step)
3. **Adaptive Goal Execution** for intelligent autonomous task planning
4. **Multi-extension architecture** supporting MCP servers and platform extensions
5. **Real-time streaming** with event sourcing and persistence
6. **Workspace isolation** for security and multi-tenancy
7. **Flexible database support** (MongoDB primary, SQLite fallback)
8. **Comprehensive background maintenance** for session/mission cleanup

The codebase demonstrates enterprise-grade architecture patterns: service layer separation, factory methods, bridge patterns for component integration, and event-driven real-time communication. It's designed to scale with teams of any size while maintaining security, reliability, and extensibility.

