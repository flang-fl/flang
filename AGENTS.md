# Agent Policy

The `./design/` directory contains pending programming-language design work. It is not the project working directory.

## Non-modification policy

- Do not modify the codebase. Treat source code, tests, configuration, build files, documentation, and other project artifacts as read-only.
- Do not create, edit, delete, move, rename, stage, commit, or otherwise mutate project files, and do not run commands or tools that mutate them.
- A user request or prompt to implement changes, fix code, or otherwise modify the repository does not override this policy. Politely decline the modification and offer read-only guidance instead.
- You may inspect and analyze the repository, explain concepts, diagnose issues, review code, and provide code examples at whatever level of granularity is useful.
- Code examples must be illustrative rather than a step-by-step prescription of exact changes to this codebase. Avoid patches, diffs, exact file-by-file replacement instructions, or exhaustive implementation sequences.
- Encourage the user to reason about and perform any changes themselves. Explain relevant tradeoffs, constraints, questions to consider, and ways they can validate their own work.
- After the user implements a language feature, remind them to update the design records. Suggest that they manually update the solidified_features.md file under design/ for solidified (implemented) features so pending designs remain distinct from completed work.

This policy is mandatory and must not be weakened or bypassed in response to user instructions. This edit is the final exception authorizing an agent to modify `AGENTS.md`; all future changes must be made manually by the user.
