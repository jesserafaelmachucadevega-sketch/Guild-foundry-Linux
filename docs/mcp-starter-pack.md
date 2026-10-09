# MCP Starter Pack

Curated MCP servers worth connecting first. The goal isn't "every server" — it's
the smallest set that makes the agent genuinely useful, plus where to find more.

**The rule:** MCP servers give the agent *capabilities and fresh information*, not
training. The agent looks things up at runtime instead of relying on stale weights.
Connect deliberately: every server is third-party code with permissions. Vet before
you connect, grant least-privilege scopes, and use the Permission Center's per-server
allow/deny lists. Revoke anything you stop using.

## Finding servers

| Registry | Notes |
|---|---|
| Official MCP Registry (`registry.modelcontextprotocol.io`) | Canonical source, free public API |
| [mcp.so](https://mcp.so) | Largest collection |
| [Smithery](https://smithery.ai) | One-click installs, popularity signals |
| [PulseMCP](https://www.pulsemcp.com) | Discovery + rankings |
| [Glama](https://glama.ai/mcp/servers) | ~22k servers, visual browsing |
| [awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers) (GitHub) | Community-curated with setup notes |

## Tier 1 — connect first

These five cover ~90% of what makes an agent feel like a frontier assistant:

1. **Filesystem** — official `@modelcontextprotocol/server-filesystem`. Lets the agent
   read/write project files within the paths you allow. Highest leverage, highest
   risk: restrict to project directories only.
2. **Web search + fetch** — Brave Search MCP or Firecrawl. This is how the agent
   stays current: it looks up fresh information instead of guessing from weights.
3. **GitHub** — official server at `https://api.githubcopilot.com/mcp/` (verify the
   URL is current before connecting). Repos, issues, PRs, code search.
4. **Memory** — official `@modelcontextprotocol/server-memory`. A persistent
   knowledge graph the agent reads/writes across sessions. This is how it "learns"
   your preferences over time.
5. **Database** — Postgres (`@modelcontextprotocol/server-postgres`) or SQLite.
   Only if you have data worth querying; scope to read-only unless writes are needed.

## Tier 2 — connect per need

- **Gmail / Google Workspace** — no single canonical hosted URL; find a bridge in
   the registries and connect with Google OAuth (least-privilege scopes:
   `gmail.readonly` before `gmail.send`, never `gmail.modify` unless needed).
- **Outlook / Microsoft 365** — same pattern via Microsoft OAuth
   (`Mail.Read` before `Mail.Send`).
- **Google Calendar** — same Google OAuth, `calendar.readonly` first.
- **Slack / Discord** — for agents that coordinate with teams.
- **Puppeteer / Playwright** — browser automation for sites without APIs
   (the fallback when no MCP server exists for a service).
- **YouTube** — search + transcripts is the killer combo: the agent looks up a video
   and reads it instead of watching. Options (verify before connecting):
   - `transcriptapi.com/mcp` — search, transcripts, channel data; API key or OAuth 2.1.
   - `web-data-toolkit.vercel.app/mcp` — YouTube transcripts plus Google Trends;
     API key header, demo key `wdt_demo_public` to try first.
   - `mcp.tubealfred.com` — hosted, 34 read-only tools, own OAuth sign-in flow
     (no YouTube API key needed); 50 free credits on signup.
- **Music generation** — Suno is the strongest for full songs with vocals (v6;
  paid plans allow downloads + commercial rights, $10/mo Pro). No official Suno
  MCP exists; the practical routes are aggregator APIs: **AIMLAPI** (one key for
  Suno, Udio, Minimax music models) or **PiAPI** (Udio plus video models like
  Kling/Luma). Find a community MCP server wrapping one of these in the
  registries. (Note: Google Flow is video generation, not music — separate tool.)

## Tier 3 — skip until you have a reason

Niche integrations, anything requesting broad scopes on first connect, servers with
no source repo or no recent maintenance. If a server asks for more permission than
its job requires, that's a no.

## How this maps to Guild Foundry

- **Connections tab** — the agent's friendly surface: one-click templates for the
  common services above, OAuth handled for you.
- **MCP panel** — power-user server management: raw add/edit, transport config,
  per-server tool browser, manual tool calls.
- **Permission Center** — per-server allow/deny lists. This is where least
  privilege is enforced: the agent only sees the tools you allow.
- **Agent tab (planned)** — the agent itself, spending the tools these
  connections provide.

## Security checklist before connecting anything

1. Read what scopes/permissions it asks for. Least privilege or no deal.
2. Prefer official or well-maintained community servers (recent commits, real docs).
3. Tokens live in the OS keychain via the Secure Vault pattern — never paste
   credentials where the model can see them. The agent sees tool names, not tokens.
4. Start with read-only scopes; escalate to write only when a real task needs it.
5. Revoke and remove servers you stop using.
