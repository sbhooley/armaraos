# Email Adapter System

ArmaraOS provides comprehensive email capabilities through a dual-approach adapter system that supports both MCP-based integrations and traditional IMAP/SMTP protocols.

## Architecture

The email adapter system consists of three layers:

1. **Built-in Email Tools** - Five core tools that agents can use to interact with email
2. **MCP Email Servers** - OAuth-enabled integrations for major providers (Gmail, Outlook, etc.)
3. **Email Channels** - Traditional IMAP/SMTP adapter for any email provider

## Available Email Tools

### 1. `email_send`
Send an email via SMTP or MCP integration.

**Parameters:**
- `to` (required): Recipient email address
- `subject` (required): Email subject line
- `body` (required): Email body (plain text or HTML)
- `cc` (optional): CC recipients (comma-separated)
- `bcc` (optional): BCC recipients (comma-separated)
- `provider` (optional): Provider hint ("gmail", "outlook", "yahoo", "smtp", "mcp")

**Example:**
```json
{
  "to": "user@example.com",
  "subject": "Meeting Reminder",
  "body": "Don't forget about our 2pm meeting today.",
  "cc": "manager@example.com"
}
```

### 2. `email_read`
Read recent emails from inbox via IMAP or MCP.

**Parameters:**
- `folder` (optional): Email folder (default: "INBOX")
- `limit` (optional): Max emails to return (default: 10, max: 50)
- `unread_only` (optional): Only return unread emails (default: true)
- `from` (optional): Filter by sender email address
- `subject_contains` (optional): Filter by subject keyword
- `provider` (optional): Provider hint

**Example:**
```json
{
  "folder": "INBOX",
  "limit": 20,
  "unread_only": true,
  "from": "notifications@github.com"
}
```

### 3. `email_search`
Search emails using provider-specific query syntax.

**Parameters:**
- `query` (required): Search query (provider-specific syntax)
- `limit` (optional): Max results (default: 20, max: 100)
- `folder` (optional): Folder to search
- `provider` (optional): Provider hint

**Query Syntax Examples:**
- **Gmail**: `from:user@example.com subject:urgent`
- **Outlook**: `subject:meeting`
- **IMAP**: `SUBJECT "meeting"`

**Example:**
```json
{
  "query": "from:alerts@example.com is:unread",
  "limit": 50
}
```

### 4. `email_reply`
Reply to an email thread with proper threading headers.

**Parameters:**
- `message_id` (required): Original message ID to reply to
- `body` (required): Reply body text
- `reply_all` (optional): Reply to all recipients (default: false)
- `provider` (optional): Provider hint

**Example:**
```json
{
  "message_id": "<CABc123@mail.gmail.com>",
  "body": "Thanks for the update! I'll review this by EOD.",
  "reply_all": true
}
```

### 5. `email_draft`
Create or update an email draft without sending.

**Parameters:**
- `to` (required): Recipient email address
- `subject` (required): Email subject
- `body` (required): Email body
- `draft_id` (optional): Existing draft ID to update
- `provider` (optional): Provider hint

**Example:**
```json
{
  "to": "team@example.com",
  "subject": "Weekly Update - DRAFT",
  "body": "Here's what we accomplished this week:\n\n[TODO: Add details]"
}
```

## Provider Priority

The email tools automatically detect and prioritize email providers in this order:

1. **MCP Email Servers** (highest priority) - OAuth-enabled, full-featured
2. **Email Channels** (fallback) - IMAP/SMTP with credentials
3. **Error** - No email configuration found

## Configuration

### Option 1: MCP Email Integration (Recommended)

MCP integrations provide the best experience with OAuth authentication and full feature support.

**Gmail Integration:**
```bash
# The Gmail MCP server is pre-configured in integrations/gmail.toml
# Just enable it in the dashboard:
1. Go to Settings → Integrations
2. Find "Gmail" and click "Connect"
3. Complete the OAuth flow
```

**Outlook Integration:**
```bash
# The Outlook integration requires an MCP server package
# Install it globally:
npm install -g @modelcontextprotocol/server-outlook

# Then enable in dashboard:
1. Go to Settings → Integrations
2. Find "Outlook" and click "Connect"
3. Complete the OAuth flow
```

### Option 2: Email Channel (IMAP/SMTP)

For providers without MCP servers, use traditional email channels.

**Add to `~/.armaraos/config.toml`:**
```toml
[[channels]]
type = "email"
name = "work_email"
imap_host = "imap.example.com"
imap_port = 993
smtp_host = "smtp.example.com"
smtp_port = 587
username = "user@example.com"
password = "your_app_password"  # Use app-specific passwords when possible
poll_interval_secs = 300
folders = ["INBOX"]
```

**Common Provider Settings:**

| Provider | IMAP | SMTP |
|----------|------|------|
| Gmail | imap.gmail.com:993 | smtp.gmail.com:587 |
| Outlook | outlook.office365.com:993 | smtp.office365.com:587 |
| Yahoo | imap.mail.yahoo.com:993 | smtp.mail.yahoo.com:587 |
| ProtonMail | 127.0.0.1:1143 (Bridge) | 127.0.0.1:1025 (Bridge) |
| FastMail | imap.fastmail.com:993 | smtp.fastmail.com:587 |
| iCloud | imap.mail.me.com:993 | smtp.mail.me.com:587 |

**Security Notes:**
- Use app-specific passwords instead of your main account password
- Enable 2FA on your email account
- ProtonMail requires the ProtonMail Bridge app

## Usage in Agent Manifests

Grant email capabilities to agents:

```toml
[capabilities]
tools_enabled = true

[capabilities.tool_allowlist]
allow = [
  "email_send",
  "email_read",
  "email_search",
  "email_reply",
  "email_draft"
]
```

## Usage in AINL Programs

Example AINL program that checks for urgent emails:

```ainl
# Check for urgent emails and summarize
emails = email_read {
  unread_only: true,
  subject_contains: "URGENT"
}

if emails.count > 0:
  summary = llm.generate {
    prompt: "Summarize these urgent emails: " + emails,
    model: "groq/mixtral-8x7b"
  }
  
  # Send summary to manager
  email_send {
    to: "manager@example.com",
    subject: "Urgent Email Summary",
    body: summary
  }
```

## Provider-Specific Features

### Gmail (via MCP)
- ✅ Full search syntax support
- ✅ Label management
- ✅ Draft creation/editing
- ✅ Thread preservation
- ✅ Attachment handling

### Outlook (via MCP)
- ✅ Full Microsoft Graph API access
- ✅ Calendar integration
- ✅ Category management
- ✅ Rich text formatting

### IMAP/SMTP (Generic)
- ✅ Basic send/receive
- ✅ IMAP SEARCH queries
- ✅ Thread headers (In-Reply-To, References)
- ⚠️ Limited draft support
- ⚠️ No labels/categories

## Troubleshooting

### No MCP email integration found
**Solution:** Install and configure an MCP email server (Gmail, Outlook) or set up an email channel.

### MCP email send failed
**Solution:** 
1. Check that the MCP server is running (`ps aux | grep mcp`)
2. Re-authenticate the OAuth connection
3. Check MCP server logs for errors

### IMAP connection timeout
**Solution:**
1. Verify IMAP host and port are correct
2. Check firewall allows outbound connections on port 993
3. Ensure app-specific password is enabled for your account

### SMTP authentication failed
**Solution:**
1. Use an app-specific password, not your main password
2. Enable "less secure app access" if required by provider
3. Check SMTP host uses STARTTLS (port 587) or TLS (port 465)

## Security Considerations

1. **Credentials Storage** - Email passwords are stored encrypted in the kernel config
2. **OAuth Tokens** - MCP integrations use OAuth tokens, refreshed automatically
3. **Sandboxing** - Email tools can be restricted via agent tool allowlists
4. **Approval Gates** - Sensitive email operations can require human approval
5. **Audit Trail** - All email operations are logged to the audit database

## Future Enhancements

Planned features for future releases:

- [ ] AINL adapter definitions in upstream AI_Native_Lang repository
- [ ] Yahoo Mail MCP integration
- [ ] ProtonMail MCP integration (via Bridge)
- [ ] Attachment upload/download support
- [ ] Email template system
- [ ] Scheduled email sending
- [ ] Email rules and filters
- [ ] HTML email composition tools

## Contributing

To add support for a new email provider:

1. Create an MCP server package (Node.js) that implements the email tools
2. Add an integration TOML file in `crates/openfang-extensions/integrations/`
3. Document provider-specific setup instructions
4. Submit a PR with tests

For questions or issues, see the [main troubleshooting guide](troubleshooting.md).
