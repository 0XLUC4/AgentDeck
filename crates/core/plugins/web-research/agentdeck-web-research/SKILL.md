---
name: agentdeck-web-research
description: Research on the internet fast and reliably - precise queries, official docs and primary sources first, parallel searches, read the page instead of trusting snippets, check dates and versions, cross-check facts, cite sources. Use for any lookup of docs, APIs, library versions, error messages, prices, news, comparisons, or when facts may have changed since training.
---

# Web research

Goal: the right answer, from a source you can name, as of today. Your training data is old; the web
is the truth for anything that changes (versions, APIs, prices, news).

## 1. Plan the question

- Write what you need in one line, and what would count as an answer (a version number, a code
  sample, a yes/no with source).
- Note today's date. Note the versions in the project (`package.json`, `Cargo.toml`, lock files):
  search for *those* versions.

## 2. Search well

- Several searches at once, in parallel, with different angles, not one vague query.
- Precise queries: exact error text in quotes, library name + version, the API name.
  - `"Cannot find module" vite 6 site:github.com`
  - `next.js 15 "use cache" site:nextjs.org`
  - `tokio 1.40 changelog`
- Operators: `site:`, `"exact phrase"`, `-exclude`, `after:2025-01-01`, `filetype:pdf`.
- Search in English for tech; in the local language for local topics.

## 3. Pick sources, best first

1. Official docs, specs, changelogs, release notes, migration guides.
2. Source code and issues of the project (GitHub: code search, issues, PRs, discussions).
3. Package registries for versions and dates (npm, crates.io/docs.rs, PyPI, pkg.go.dev).
4. Maintainers' blogs, well-known engineering blogs.
5. Stack Overflow / forums: check the date, the votes, and that the answer matches your version.

Avoid: SEO content farms, AI-generated summaries, undated pages, answers for another major version.

## 4. Read, do not skim

- Open the page (fetch tool, or the AgentDeck browser panel for pages needing JS or login) and read
  the relevant section. Snippets lie by omission.
- Check the page date and the version it talks about.
- For code: prefer the example from the docs or tests of the library over blog code.
- For a GitHub issue: read to the end; the fix or workaround is often in the last comments.

## 5. Check

- An important fact needs two independent sources, or one primary source.
- Sources disagree: prefer the newer primary one, and say they disagree.
- Try it when possible: run the snippet, check the installed version (`npm view pkg version`,
  `cargo search`, `pip index versions`).

## 6. Answer

- Answer first, then the evidence: source title + URL, date/version.
- Separate what the sources say from what you infer.
- Say what you could not confirm. Never invent a URL, a version or a quote.
- Do not paste long copyrighted text; summarize and link.

## Privacy

Never put the user's private data (names, emails, keys, internal code or URLs) in search queries.
