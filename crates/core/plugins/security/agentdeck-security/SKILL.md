---
name: agentdeck-security
description: Write and check code for security - input validation, injection (SQL, shell, HTML/XSS, path), authentication, authorization, sessions, secrets, dependencies, file uploads, CORS, headers, rate limits, logging of sensitive data. Use when code handles user input, auth, payments, files, external APIs or personal data, before a release, or when asked for a security review or hardening.
---

# Security

Treat every input from outside (user, network, file, env, other services, AI output) as hostile until
validated.

## Checklist by area

| Area | Do |
|---|---|
| Input | validate on the server: type, length, format, range (zod, valibot, serde, pydantic); allowlists over denylists |
| SQL | parameterized queries / ORM; never build SQL with string concatenation |
| Shell | no shell strings; pass args as an array (`Command::new(x).arg(y)`, `execFile`); never `eval` |
| HTML | framework escaping; no `innerHTML`/`dangerouslySetInnerHTML` with user data; sanitize (DOMPurify) when HTML is needed |
| Paths | resolve then check the result stays inside the allowed folder; reject `..`, absolute paths, symlinks out |
| URLs fetched by server | allowlist hosts; block internal IPs (SSRF: 127.0.0.1, 169.254.169.254, 10/8, 192.168/16) |
| Auth | proven library (Better Auth, Auth.js, framework built-ins); passwords hashed with argon2id/bcrypt; rate-limit login |
| Authorization | check on every request server-side: this user may do this action on this object (IDOR) |
| Sessions/tokens | httpOnly, Secure, SameSite cookies; short-lived tokens; rotate on login; invalidate on logout |
| CSRF | SameSite cookies + CSRF token or origin check for state-changing requests |
| Secrets | env vars or secret manager; never in code, git, logs, URLs, client bundles; `.env` in `.gitignore` |
| Files | check size and type (magic bytes, not just extension); random names; store outside web root; no execution |
| CORS | explicit origins, never `*` with credentials |
| Headers | CSP, HSTS, X-Content-Type-Options, frame-ancestors |
| Errors | generic message to users; details in server logs only |
| Logs | no passwords, tokens, card numbers, personal data |
| Crypto | standard libraries only; random from a CSPRNG; constant-time compare for secrets |
| Deserialization | no `pickle`/`eval`/unsafe YAML on untrusted data |
| LLM apps | treat model output as untrusted input; tool calls need permission checks; watch prompt injection from fetched content |

## Dependencies

- Audit: `pnpm audit`, `npm audit`, `cargo audit`, `pip-audit`, `osv-scanner`. Fix high/critical.
- New package: maintained, popular, small, no install scripts surprise; check the exact name
  (typosquatting). Pin with a lock file.

## Review a change

1. List the trust boundaries the change crosses (input in, data out, privileged action).
2. For each, walk the table above.
3. Try the attack: a quick test or request with a malicious value (`' OR 1=1--`, `../../etc/passwd`,
   `<img src=x onerror=alert(1)>`, another user's id).
4. Fix at the boundary, once, in shared code; add a test for the attack.

## Report

Findings ranked (critical / high / medium / low), each with the location, the attack, and the fix.
Only test systems you own or are allowed to test.
