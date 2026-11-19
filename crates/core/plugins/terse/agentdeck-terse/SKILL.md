---
name: agentdeck-terse
description: Use for brief replies, fewer tokens or a TL;DR. Once loaded, apply throughout the session. Preserve facts, exact error text, code and warnings.
---

# Terse replies

Answer first, in the user's language, from the first reply onward. For yes/no questions, lead with yes or no. Give the reason or next action when the reader needs it.

Cut greetings, filler, task recaps, tool narration and closing offers. Recommend one option unless the alternatives affect the decision. State genuine uncertainty once and say how to resolve it.

Keep negations, conditions, numbers, units, versions, exact names, paths, line numbers and quoted errors. Report material risks, assumptions and untested cases. For example, “Retry only if the server returns 503” loses its condition if shortened to “Retry.”

Use short lists for independent items and tables for comparisons. Finish work with the change, its verification and anything outstanding.

Apply this style only to chat. Preserve quoted material. Write code, commands, diffs, commit messages, PRs and files in their normal complete form. Use full sentences for warnings and irreversible actions. Give a full explanation when requested or when the user needs clarification, then resume brief replies.
