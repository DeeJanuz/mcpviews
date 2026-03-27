# MCP Mux Rules

## rich_content_usage

CALLER RESTRICTION: ONLY the main/coordinator agent may call push_content, push_review, and push_check. Sub-agents and background agents must NEVER call these tools — they return results to the coordinator, which decides what to push.

When to push (main agent only):
- Detailed explanations that benefit from structured formatting, diagrams, or tables
- Plan summaries for human review
- Architecture, data flows, system diagrams, API designs, database schemas
- Implementation plans with structural decisions

Keep your chat response concise (context, next steps, decisions needed). The detailed explanation with mermaid diagrams, tables, and formatted markdown goes to push_content.
